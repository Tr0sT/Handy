//! Authentication with Claude.ai for cloud STT access.
//!
//! Reads tokens from Claude Code's ~/.claude/.credentials.json.
//! On 403, re-reads the file to pick up tokens refreshed by Claude Code.

use log::{debug, info};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

const CLAUDE_AI_ORIGIN: &str = "https://claude.ai";

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ClaudeAuthState {
    pub access_token: Option<String>,
    pub is_logged_in: bool,
}

impl Default for ClaudeAuthState {
    fn default() -> Self {
        Self {
            access_token: None,
            is_logged_in: false,
        }
    }
}

/// JSON structure of ~/.claude/.credentials.json
#[derive(Debug, Deserialize)]
struct ClaudeCodeCredentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeCodeOAuth>,
}

#[derive(Debug, Deserialize)]
struct ClaudeCodeOAuth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
}

fn claude_config_dir() -> Option<PathBuf> {
    // Match Claude Code: CLAUDE_CONFIG_DIR ?? os.homedir() + ".claude"
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    home_dir().map(|h| h.join(".claude"))
}

fn credentials_path() -> Option<PathBuf> {
    claude_config_dir().map(|d| d.join(".credentials.json"))
}

/// Cross-platform home directory (HOME on Unix, USERPROFILE on Windows).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn read_claude_code_token() -> Option<String> {
    let path = credentials_path()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let creds: ClaudeCodeCredentials = serde_json::from_str(&contents).ok()?;
    creds
        .claude_ai_oauth?
        .access_token
        .filter(|s| !s.is_empty())
}

/// Manager for Claude.ai authentication state.
#[derive(Clone)]
pub struct ClaudeAuthManager {
    state: Arc<Mutex<ClaudeAuthState>>,
    #[allow(dead_code)]
    app_handle: AppHandle,
}

impl ClaudeAuthManager {
    pub fn new(app_handle: &AppHandle) -> Self {
        let manager = Self {
            state: Arc::new(Mutex::new(ClaudeAuthState::default())),
            app_handle: app_handle.clone(),
        };

        // Load token from Claude Code credentials
        manager.reload_from_claude_code();

        manager
    }

    /// Read the latest token from ~/.claude/.credentials.json
    pub fn reload_from_claude_code(&self) -> bool {
        match read_claude_code_token() {
            Some(token) => {
                let mut state = self.state.lock().unwrap();
                state.access_token = Some(token);
                state.is_logged_in = true;
                info!("[claude_auth] Loaded token from Claude Code credentials");
                true
            }
            None => {
                debug!("[claude_auth] No token found in Claude Code credentials");
                false
            }
        }
    }

    pub fn get_state(&self) -> ClaudeAuthState {
        self.state.lock().unwrap().clone()
    }

    pub fn get_access_token(&self) -> Option<String> {
        self.state.lock().unwrap().access_token.clone()
    }

    pub fn get_origin(&self) -> String {
        CLAUDE_AI_ORIGIN.to_string()
    }

    /// Set the access token manually.
    pub fn set_access_token(&self, token: String) {
        let mut state = self.state.lock().unwrap();
        state.access_token = Some(token);
        state.is_logged_in = true;
        info!("[claude_auth] Access token set manually");
    }

    /// Log out.
    pub fn logout(&self) {
        let mut state = self.state.lock().unwrap();
        state.access_token = None;
        state.is_logged_in = false;
        info!("[claude_auth] Logged out");
    }

    /// Check if Claude Code credentials file exists.
    pub fn has_claude_code_credentials() -> bool {
        credentials_path().map(|p| p.exists()).unwrap_or(false)
    }
}
