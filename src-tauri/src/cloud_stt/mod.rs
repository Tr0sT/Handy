//! Cloud speech-to-text providers.
//!
//! Supports cloud-backed STT infrastructure used by virtual transcription
//! models such as Codex Local.

pub mod claude_auth;
pub mod claude_session;
pub mod codex_auth;
pub mod codex_stt;

use serde::Serialize;

/// Events emitted to the frontend during cloud STT.
#[derive(Clone, Debug, Serialize)]
pub struct CloudTranscriptEvent {
    pub text: String,
    pub is_final: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CloudSttStatusEvent {
    pub status: String,
    pub message: Option<String>,
}

#[cfg(test)]
mod test_support;
