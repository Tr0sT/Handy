//! Codex Desktop / ChatGPT Whisper transcription endpoint.
//!
//! Implements the exact multipart/form-data request format used by Codex Desktop
//! (reverse-engineered from CodexDesktop-Rebuild v1.0.4).
//!
//! Flow: record audio → encode WAV → POST multipart to /transcribe → get text.
//!
//! Uses the app's existing `reqwest` client to avoid linking a second TLS stack
//! into the Tauri binary.

use super::codex_auth::CodexAuthManager;
use log::{debug, info};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(30);
use uuid::Uuid;

/// Encode f32 PCM samples (16kHz mono) to WAV bytes using hound.
fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| format!("Failed to create WAV writer: {}", e))?;

        for &sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let int_val = (clamped * 32767.0) as i16;
            writer
                .write_sample(int_val)
                .map_err(|e| format!("Failed to write WAV sample: {}", e))?;
        }

        writer
            .finalize()
            .map_err(|e| format!("Failed to finalize WAV: {}", e))?;
    }

    Ok(cursor.into_inner())
}

/// Build the multipart body matching Codex Desktop's Hxn function exactly.
///
/// Format (byte-exact match of Hxn / Wxn / $xn / Uxn / zxn):
///   --{boundary}\r\n
///   Content-Disposition: form-data; name="file"; filename="{filename}"\r\n
///   Content-Type: {content_type}\r\n
///   \r\n
///   {raw audio bytes}\r\n
///   [optional language field]
///   --{boundary}--\r\n
fn build_multipart_body(
    audio_data: &[u8],
    boundary: &str,
    filename: &str,
    content_type: &str,
    language: Option<&str>,
) -> Vec<u8> {
    let mut body = Vec::new();

    // File part
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
            filename
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n", content_type).as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(audio_data);
    body.extend_from_slice(b"\r\n");

    // Language part (optional — Codex Desktop never sends it, but the backend accepts it)
    if let Some(lang) = language {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"language\"\r\n");
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("{}\r\n", lang).as_bytes());
    }

    // Closing boundary
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    body
}

/// Transcription response from the Whisper endpoint.
#[derive(Debug, serde::Deserialize)]
struct TranscribeResponse {
    text: String,
}

/// Limit error text by Unicode scalar values, never by a byte offset.
fn error_preview(text: &str) -> String {
    text.chars().take(500).collect()
}

async fn do_transcribe_request(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    boundary: &str,
    token: &str,
    account_id: Option<&str>,
) -> Result<reqwest::Response, String> {
    let mut request = client
        .post(url)
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body);
    for (key, value) in CodexAuthManager::build_auth_headers(token, account_id) {
        request = request.header(key, value);
    }
    request
        .send()
        .await
        .map_err(|e| format!("Transcription request failed: {e}"))
}

/// Batch transcription with a deadline covering credentials, upload, response
/// body and the single 401 retry. Live dictation and History Retry share it.
pub async fn transcribe_samples(
    auth: &Arc<CodexAuthManager>,
    samples: &[f32],
    language: Option<&str>,
) -> Result<String, String> {
    let url = format!("{}/transcribe", CodexAuthManager::api_base_url());
    transcribe_with_deadline(auth, samples, language, &url, TRANSCRIPTION_TIMEOUT).await
}

async fn transcribe_with_deadline(
    auth: &Arc<CodexAuthManager>,
    samples: &[f32],
    language: Option<&str>,
    url: &str,
    timeout: Duration,
) -> Result<String, String> {
    tokio::time::timeout(timeout, transcribe_at(auth, samples, language, url))
        .await.map_err(|_| format!(
            "Codex transcription timed out after {} seconds. The recording can be retried from History.",
            timeout.as_secs()
        ))?
}

async fn transcribe_at(
    auth: &Arc<CodexAuthManager>,
    samples: &[f32],
    language: Option<&str>,
    url: &str,
) -> Result<String, String> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    let wav_data = encode_wav(samples, 16_000)?;
    let boundary = format!("----codex-transcribe-{}", Uuid::new_v4());
    let body = build_multipart_body(&wav_data, &boundary, "codex.wav", "audio/wav", language);
    debug!(
        "[codex_stt] Encoded {} samples ({} bytes)",
        samples.len(),
        body.len()
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(TRANSCRIPTION_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build transcription client: {e}"))?;
    let (token, account_id) = auth.get_valid_token().await?;
    let mut response = do_transcribe_request(
        &client,
        url,
        body.clone(),
        &boundary,
        &token,
        account_id.as_deref(),
    )
    .await?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        // Do not merely re-read auth.json: an unexpired JWT can be rejected.
        // Manual credentials must never switch to a file-backed account here.
        drop(response);
        let (token, account_id) = auth.refresh_after_rejection(&token).await?;
        response =
            do_transcribe_request(&client, url, body, &boundary, &token, account_id.as_deref())
                .await?;
    }
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Transcription failed: HTTP {status} — {}",
            error_preview(&body)
        ));
    }
    let result: TranscribeResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse transcription response: {e}"))?;
    let transcript = result.text.trim().to_owned();
    // Never log speech content at info level, not even a truncated preview.
    info!(
        "[codex_stt] Transcription complete ({} characters)",
        transcript.chars().count()
    );
    Ok(transcript)
}

#[cfg(test)]
mod tests;
