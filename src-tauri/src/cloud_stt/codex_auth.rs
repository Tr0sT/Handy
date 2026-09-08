//! Codex credentials are either an explicitly supplied access token or a file
//! import. Never fall back from one source/account to the other implicitly.

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROD_API_BASE: &str = "https://chatgpt.com/backend-api";
const AUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ORIGINATOR: &str = "Codex Desktop";
const APP_VERSION: &str = "1.0.4";
const EXPIRY_MARGIN_SECS: u64 = 300;
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodexAuthState {
    pub is_logged_in: bool,
    pub has_auth_file: bool,
}

#[derive(Deserialize, Serialize)]
struct CodexAuthFile {
    tokens: CodexTokens,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
struct CodexTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
enum CredentialSource {
    Manual,
    CodexFile(PathBuf),
}

// Deliberately no Debug: these types contain bearer/refresh credentials.
#[derive(Clone)]
struct Credentials {
    access_token: String,
    account_id: Option<String>,
    source: CredentialSource,
}

#[derive(Default)]
struct CodexAuthInner {
    // Changes only on explicit import, manual input or logout, not on rotation.
    revision: u64,
    credentials: Option<Credentials>,
}

pub struct CodexAuthManager {
    state: Mutex<CodexAuthInner>,
    refresh_gate: tokio::sync::Mutex<()>,
    file_path: Option<PathBuf>,
    refresh_url: String,
    client: reqwest::Client,
}

fn auth_file_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(home).join("auth.json"));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".codex/auth.json"))
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    decode_jwt_payload(token)?
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(str::to_owned)
}

fn is_token_expired(token: &str) -> bool {
    let exp = decode_jwt_payload(token)
        .and_then(|claims| claims.get("exp").and_then(serde_json::Value::as_u64))
        .unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now >= exp.saturating_sub(EXPIRY_MARGIN_SECS)
}

fn build_user_agent() -> String {
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unknown"
    };
    format!("Codex Desktop/{APP_VERSION} ({platform}; {arch})")
}

fn read_auth_file(path: &Path) -> Result<(String, CodexAuthFile), String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read Codex credentials: {e}"))?;
    let file: CodexAuthFile = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse Codex credentials: {e}"))?;
    if file.tokens.access_token.trim().is_empty() {
        return Err("Codex credentials contain an empty access token".into());
    }
    Ok((contents, file))
}

fn file_credentials(file: &CodexAuthFile, path: &Path) -> Credentials {
    Credentials {
        access_token: file.tokens.access_token.clone(),
        account_id: extract_chatgpt_account_id(&file.tokens.access_token).or_else(|| {
            file.tokens
                .extra
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        }),
        source: CredentialSource::CodexFile(path.to_owned()),
    }
}

fn require_same_account(expected: &Credentials, actual: &Credentials) -> Result<(), String> {
    if expected.account_id != actual.account_id {
        return Err(
            "Codex account changed. Import credentials again to confirm the new account.".into(),
        );
    }
    // The account claim can be absent on some tokens. Still reject a changed
    // user subject rather than adopting another user's file silently.
    let subject = |token: &str| {
        decode_jwt_payload(token)
            .and_then(|v| v.get("sub").and_then(|s| s.as_str()).map(str::to_owned))
    };
    if subject(&expected.access_token) != subject(&actual.access_token) {
        return Err("Codex user changed. Import credentials again to confirm the new user.".into());
    }
    Ok(())
}

/// A sidecar lock survives atomic replacement of auth.json. It coordinates
/// Handy instances; non-cooperating writers (e.g. Codex) are detected by the
/// content comparison before replacement. Do not delete the sidecar on unlock.
async fn lock_auth_file(path: &Path) -> Result<File, String> {
    let lock_path = path.with_file_name("auth.json.handy.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(lock_path)
        .map_err(|e| format!("Failed to open credential lock: {e}"))?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => {
                    tokio::time::sleep(Duration::from_millis(25)).await
                }
                Err(TryLockError::Error(e)) => {
                    return Err(format!("Failed to lock credentials: {e}"))
                }
            }
        }
    })
    .await
    .map_err(|_| "Timed out waiting for Codex credential refresh".to_string())?
}

fn write_auth_file_atomically(path: &Path, file: &CodexAuthFile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Codex credential directory is missing")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Failed to create credential update: {e}"))?;
    // NamedTempFile creates Unix files with mode 0600. Keep that restrictive
    // mode rather than copying potentially over-broad permissions from input.
    serde_json::to_writer_pretty(&mut temp, file)
        .map_err(|e| format!("Failed to serialize credentials: {e}"))?;
    temp.write_all(b"\n")
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| format!("Failed to flush credentials: {e}"))?;
    temp.persist(path)
        .map_err(|e| format!("Failed to replace credentials: {}", e.error))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| format!("Failed to sync credential directory: {e}"))?;
    Ok(())
}

impl CodexAuthManager {
    pub fn new() -> Self {
        Self::with_config(auth_file_path(), AUTH_TOKEN_URL.to_owned())
    }

    fn with_config(file_path: Option<PathBuf>, refresh_url: String) -> Self {
        let manager = Self {
            state: Mutex::new(CodexAuthInner::default()),
            refresh_gate: tokio::sync::Mutex::new(()),
            file_path,
            refresh_url,
            client: reqwest::Client::new(),
        };
        manager.reload_from_file();
        manager
    }

    #[cfg(test)]
    pub(crate) fn test_config(file_path: Option<PathBuf>, refresh_url: String) -> Self {
        Self::with_config(file_path, refresh_url)
    }

    fn lock_state(&self) -> MutexGuard<'_, CodexAuthInner> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn reload_from_file(&self) -> bool {
        let loaded = self
            .file_path
            .as_ref()
            .and_then(|path| match read_auth_file(path) {
                Ok((_, file)) => Some(file_credentials(&file, path)),
                Err(e) => {
                    debug!("[codex_auth] {e}");
                    None
                }
            });
        let mut state = self.lock_state();
        if let Some(credentials) = loaded {
            state.revision = state.revision.wrapping_add(1);
            state.credentials = Some(credentials);
            info!("[codex_auth] Imported file credentials");
            true
        } else {
            // A failed import must not leave an old file session looking valid.
            // Nor should it discard a manually entered credential.
            if state
                .credentials
                .as_ref()
                .is_some_and(|c| matches!(c.source, CredentialSource::CodexFile(_)))
            {
                state.revision = state.revision.wrapping_add(1);
                state.credentials = None;
            }
            false
        }
    }

    pub fn get_state(&self) -> CodexAuthState {
        CodexAuthState {
            is_logged_in: self.lock_state().credentials.is_some(),
            has_auth_file: self.file_path.as_ref().is_some_and(|p| p.is_file()),
        }
    }

    pub fn set_access_token(&self, token: String) -> Result<(), String> {
        let token = token.trim().to_owned();
        if token.is_empty() {
            return Err("Access token must not be empty".into());
        }
        let credentials = Credentials {
            account_id: extract_chatgpt_account_id(&token),
            access_token: token,
            source: CredentialSource::Manual,
        };
        let mut state = self.lock_state();
        state.revision = state.revision.wrapping_add(1);
        state.credentials = Some(credentials);
        Ok(())
    }

    pub fn logout(&self) {
        let mut state = self.lock_state();
        state.revision = state.revision.wrapping_add(1);
        state.credentials = None;
    }

    pub async fn get_valid_token(self: &Arc<Self>) -> Result<(String, Option<String>), String> {
        self.get_token(None).await
    }

    /// A server rejection forces rotation even when JWT exp is in the future.
    pub async fn refresh_after_rejection(
        self: &Arc<Self>,
        rejected: &str,
    ) -> Result<(String, Option<String>), String> {
        self.get_token(Some(rejected.to_owned())).await
    }

    async fn get_token(
        self: &Arc<Self>,
        rejected: Option<String>,
    ) -> Result<(String, Option<String>), String> {
        let (revision, credentials) = {
            let state = self.lock_state();
            (
                state.revision,
                state.credentials.clone().ok_or("Not logged in to Codex")?,
            )
        };
        if !is_token_expired(&credentials.access_token)
            && rejected.as_deref() != Some(credentials.access_token.as_str())
        {
            return Ok((credentials.access_token, credentials.account_id));
        }
        if credentials.source == CredentialSource::Manual {
            return Err("The manually entered Codex token expired or was rejected. Enter a new token; file credentials will not be substituted.".into());
        }
        let manager = Arc::clone(self);
        // Once refresh_token rotation starts it must finish persisting even if
        // the user cancels transcription or its outer deadline elapses. Dropping
        // this JoinHandle does not abort the bounded refresh task. The revision
        // check below prevents it from undoing a logout/manual account change.
        tokio::spawn(async move { manager.refresh_file(revision, credentials, rejected).await })
            .await
            .map_err(|e| format!("Credential refresh task failed: {e}"))?
    }

    async fn refresh_file(
        &self,
        revision: u64,
        original: Credentials,
        rejected: Option<String>,
    ) -> Result<(String, Option<String>), String> {
        let _gate = self.refresh_gate.lock().await;
        {
            let state = self.lock_state();
            if state.revision != revision {
                return Err("Codex login changed during refresh; try again".into());
            }
            if let Some(c) = &state.credentials {
                if !is_token_expired(&c.access_token)
                    && rejected.as_deref() != Some(c.access_token.as_str())
                {
                    return Ok((c.access_token.clone(), c.account_id.clone()));
                }
            }
        }
        let CredentialSource::CodexFile(path) = &original.source else {
            return Err("Manual tokens cannot be refreshed from a file".into());
        };
        let _file_lock = lock_auth_file(path).await?;
        let (before, mut file) = read_auth_file(path)?;
        let current = file_credentials(&file, path);
        require_same_account(&original, &current)?;
        if !is_token_expired(&current.access_token)
            && rejected.as_deref() != Some(current.access_token.as_str())
        {
            return self.install_refreshed(revision, current);
        }
        let refresh_token = file
            .tokens
            .refresh_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or("No refresh token in Codex credentials; sign in again")?;
        let response = self.client.post(&self.refresh_url)
            .timeout(REFRESH_TIMEOUT)
            .json(&serde_json::json!({"grant_type": "refresh_token", "refresh_token": refresh_token, "client_id": CLIENT_ID}))
            .send().await.map_err(|e| format!("Token refresh request failed: {e}"))?;
        if !response.status().is_success() {
            // Auth responses may echo secrets. Do not send their raw bodies to
            // logs or UI, even on failure.
            return Err(format!(
                "Token refresh failed: HTTP {}. Sign in to Codex again.",
                response.status()
            ));
        }
        let refreshed: RefreshResponse = response
            .json()
            .await
            .map_err(|e| format!("Invalid token refresh response: {e}"))?;
        if is_token_expired(&refreshed.access_token) {
            return Err("Token refresh returned an empty, invalid or expired token".into());
        }
        file.tokens.access_token = refreshed.access_token;
        if let Some(token) = refreshed.id_token {
            file.tokens.id_token = Some(token);
        }
        if let Some(token) = refreshed.refresh_token {
            file.tokens.refresh_token = Some(token);
        }
        let credentials = file_credentials(&file, path);
        require_same_account(&original, &credentials)?;

        let (after, latest_file) = read_auth_file(path)?;
        if before != after {
            // Another program doesn't use our sidecar lock. Never overwrite its
            // new account, rotated refresh token, or unrelated JSON fields.
            let latest = file_credentials(&latest_file, path);
            require_same_account(&original, &latest)?;
            if !is_token_expired(&latest.access_token)
                && rejected.as_deref() != Some(latest.access_token.as_str())
            {
                return self.install_refreshed(revision, latest);
            }
            warn!("[codex_auth] Credentials changed externally during refresh; not overwriting");
            return Err(
                "Codex credentials changed during refresh. Import credentials again.".into(),
            );
        }
        write_auth_file_atomically(path, &file)?;
        self.install_refreshed(revision, credentials)
    }

    fn install_refreshed(
        &self,
        revision: u64,
        credentials: Credentials,
    ) -> Result<(String, Option<String>), String> {
        let mut state = self.lock_state();
        if state.revision != revision {
            return Err("Codex login changed during refresh; try again".into());
        }
        let result = (
            credentials.access_token.clone(),
            credentials.account_id.clone(),
        );
        state.credentials = Some(credentials);
        Ok(result)
    }

    pub fn api_base_url() -> String {
        if let Ok(url) = std::env::var("CODEX_API_BASE_URL") {
            if !url.trim_end_matches('/').is_empty() {
                return url.trim_end_matches('/').to_owned();
            }
        }
        if std::env::var("CODEX_API_ENDPOINT").is_ok_and(|v| v.eq_ignore_ascii_case("localhost")) {
            return "http://localhost:8000/api".into();
        }
        PROD_API_BASE.into()
    }

    pub fn build_auth_headers(token: &str, account_id: Option<&str>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Authorization".into(), format!("Bearer {token}")),
            ("originator".into(), ORIGINATOR.into()),
            ("User-Agent".into(), build_user_agent()),
        ];
        if let Some(account_id) = account_id {
            headers.push(("ChatGPT-Account-Id".into(), account_id.into()));
        }
        headers
    }
}

#[cfg(test)]
mod tests;
