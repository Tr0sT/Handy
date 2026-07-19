//! Authentication for Codex Desktop / ChatGPT transcription endpoint.
//!
//! Reads tokens from `~/.codex/auth.json` (same file Codex Desktop uses).
//! Supports auto-refresh via OAuth2 refresh_token flow against auth.openai.com.
//!
//! Architecture matches Codex Desktop's Mhe.handleRequest:
//!   1. Load access_token from auth.json
//!   2. Check JWT expiry (with 5-minute margin)
//!   3. If expired, refresh via auth.openai.com/oauth/token
//!   4. On 401 during transcription, caller retries with refresh

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// Constants from Codex Desktop source (main.js / index-MmO6ZWIv.js)
const PROD_API_BASE: &str = "https://chatgpt.com/backend-api";
const AUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ORIGINATOR: &str = "Codex Desktop";
const APP_VERSION: &str = "1.0.4";
/// JWT expiry margin in seconds (refresh if expiring within 5 minutes).
const EXPIRY_MARGIN_SECS: u64 = 300;

/// Codex auth state exposed to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodexAuthState {
    pub is_logged_in: bool,
    pub has_auth_file: bool,
}

impl Default for CodexAuthState {
    fn default() -> Self {
        Self {
            is_logged_in: false,
            has_auth_file: auth_file_path().map(|p| p.exists()).unwrap_or(false),
        }
    }
}

/// JSON structure of ~/.codex/auth.json
#[derive(Debug, Deserialize, Serialize)]
struct CodexAuthFile {
    tokens: CodexTokens,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CodexTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

/// OAuth2 token refresh response from auth.openai.com/oauth/token
#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn auth_file_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".codex").join("auth.json"))
}

/// Decode the payload of a JWT (no signature verification).
fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    // JWT base64url may need padding
    let payload = parts[1];
    let padded = match payload.len() % 4 {
        2 => format!("{}==", payload),
        3 => format!("{}=", payload),
        _ => payload.to_string(),
    };
    let bytes = URL_SAFE_NO_PAD.decode(padded.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Extract chatgpt_account_id from JWT payload.
/// Matches: token.split(".")[1] → base64url decode →
///   ["https://api.openai.com/auth"]["chatgpt_account_id"]
fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let claims = decode_jwt_payload(token)?;
    claims
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(|s| s.to_string())
}

/// Check if a JWT is expired (with margin). Returns true if expired or unparseable.
fn is_token_expired(token: &str) -> bool {
    match decode_jwt_payload(token) {
        Some(claims) => {
            let exp = claims.get("exp").and_then(|v| v.as_u64()).unwrap_or(0);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now > exp.saturating_sub(EXPIRY_MARGIN_SECS)
        }
        None => true,
    }
}

/// Build the User-Agent string matching Codex Desktop's buildDesktopUserAgent().
fn build_user_agent() -> String {
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unknown"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };
    format!("Codex Desktop/{} ({}; {})", APP_VERSION, platform, arch)
}

/// Refresh the access token using the refresh_token.
/// Matches Codex Desktop's OAuth2 refresh flow via auth.openai.com/oauth/token.
///
/// Uses reqwest (not rquest) since auth.openai.com doesn't have the same
/// Cloudflare bot protection as chatgpt.com.
async fn refresh_access_token(refresh_token: &str) -> Result<RefreshResponse, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    });

    let resp = client
        .post(AUTH_TOKEN_URL)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Token refresh failed: HTTP {} — {}",
            status,
            &text[..text.len().min(500)]
        ));
    }

    resp.json::<RefreshResponse>()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))
}

/// Manager for Codex/ChatGPT authentication.
pub struct CodexAuthManager {
    state: Arc<Mutex<CodexAuthInner>>,
}

struct CodexAuthInner {
    access_token: Option<String>,
    account_id: Option<String>,
}

impl CodexAuthManager {
    pub fn new() -> Self {
        let manager = Self {
            state: Arc::new(Mutex::new(CodexAuthInner {
                access_token: None,
                account_id: None,
            })),
        };
        // Try to load on creation
        manager.reload_from_file();
        manager
    }

    /// Read tokens from ~/.codex/auth.json. Returns true if a token was loaded.
    pub fn reload_from_file(&self) -> bool {
        let path = match auth_file_path() {
            Some(p) if p.exists() => p,
            _ => {
                debug!("[codex_auth] No auth file found");
                return false;
            }
        };

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warn!("[codex_auth] Failed to read auth file: {}", e);
                return false;
            }
        };

        let auth_file: CodexAuthFile = match serde_json::from_str(&contents) {
            Ok(f) => f,
            Err(e) => {
                warn!("[codex_auth] Failed to parse auth file: {}", e);
                return false;
            }
        };

        let token = &auth_file.tokens.access_token;
        if token.is_empty() {
            debug!("[codex_auth] Auth file has empty access_token");
            return false;
        }

        let account_id = extract_chatgpt_account_id(token);
        let mut inner = self.state.lock().unwrap();
        inner.access_token = Some(token.clone());
        inner.account_id = account_id;
        info!("[codex_auth] Loaded token from ~/.codex/auth.json");
        true
    }

    pub fn get_state(&self) -> CodexAuthState {
        let inner = self.state.lock().unwrap();
        CodexAuthState {
            is_logged_in: inner.access_token.is_some(),
            has_auth_file: auth_file_path().map(|p| p.exists()).unwrap_or(false),
        }
    }

    pub fn is_logged_in(&self) -> bool {
        self.state.lock().unwrap().access_token.is_some()
    }

    /// Set the access token manually (in-memory only, does not write to auth.json).
    pub fn set_access_token(&self, token: String) {
        let account_id = extract_chatgpt_account_id(&token);
        let mut inner = self.state.lock().unwrap();
        inner.access_token = Some(token);
        inner.account_id = account_id;
        info!("[codex_auth] Access token set manually (in-memory only)");
    }

    pub fn logout(&self) {
        let mut inner = self.state.lock().unwrap();
        inner.access_token = None;
        inner.account_id = None;
        info!("[codex_auth] Logged out");
    }

    /// Get the current access token, refreshing if expired.
    /// This is async because token refresh requires an HTTP call.
    pub async fn get_valid_token(&self) -> Result<(String, Option<String>), String> {
        let (token, account_id) = {
            let inner = self.state.lock().unwrap();
            match &inner.access_token {
                Some(t) => (t.clone(), inner.account_id.clone()),
                None => return Err("Not logged in to Codex".to_string()),
            }
        };

        // Check if token needs refresh
        if !is_token_expired(&token) {
            return Ok((token, account_id));
        }

        info!("[codex_auth] Token expired, attempting refresh...");

        // Read the refresh token from file
        let path = auth_file_path().ok_or("No auth file path")?;
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read auth file: {}", e))?;
        let mut auth_file: CodexAuthFile = serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse auth file: {}", e))?;

        let refresh_token = auth_file
            .tokens
            .refresh_token
            .as_ref()
            .ok_or("No refresh_token in auth file")?
            .clone();

        let refreshed = refresh_access_token(&refresh_token).await?;

        // Update auth.json (matches what Codex Desktop does)
        auth_file.tokens.access_token = refreshed.access_token.clone();
        if let Some(id_token) = &refreshed.id_token {
            auth_file.tokens.id_token = Some(id_token.clone());
        }
        if let Some(rt) = &refreshed.refresh_token {
            auth_file.tokens.refresh_token = Some(rt.clone());
        }

        let updated_json = serde_json::to_string_pretty(&auth_file)
            .map_err(|e| format!("Failed to serialize auth file: {}", e))?;
        std::fs::write(&path, format!("{}\n", updated_json))
            .map_err(|e| format!("Failed to write auth file: {}", e))?;

        let new_account_id = extract_chatgpt_account_id(&refreshed.access_token);

        // Update in-memory state
        let mut inner = self.state.lock().unwrap();
        inner.access_token = Some(refreshed.access_token.clone());
        inner.account_id = new_account_id.clone();

        info!("[codex_auth] Token refreshed successfully");
        Ok((refreshed.access_token, new_account_id))
    }

    /// Get the API base URL (matches Mhe.resolveApiBaseUrl).
    pub fn api_base_url() -> String {
        if let Ok(url) = std::env::var("CODEX_API_BASE_URL") {
            let trimmed = url.trim_end_matches('/').to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        if let Ok(endpoint) = std::env::var("CODEX_API_ENDPOINT") {
            if endpoint.to_lowercase() == "localhost" {
                return "http://localhost:8000/api".to_string();
            }
        }
        PROD_API_BASE.to_string()
    }

    /// Build auth headers matching applyDesktopAuthHeaders().
    pub fn build_auth_headers(token: &str, account_id: Option<&str>) -> Vec<(String, String)> {
        let mut headers = vec![
            ("Authorization".to_string(), format!("Bearer {}", token)),
            ("originator".to_string(), ORIGINATOR.to_string()),
            ("User-Agent".to_string(), build_user_agent()),
        ];
        if let Some(aid) = account_id {
            headers.push(("ChatGPT-Account-Id".to_string(), aid.to_string()));
        }
        headers
    }
}
