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

/// Build the HTTP client for Codex/ChatGPT transcription requests.
fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Send a POST to /transcribe with the given body and auth headers.
async fn do_transcribe_request(
    url: &str,
    body: Vec<u8>,
    boundary: &str,
    token: &str,
    account_id: Option<&str>,
) -> Result<reqwest::Response, String> {
    let client = build_http_client()?;
    let auth_headers = CodexAuthManager::build_auth_headers(token, account_id);

    let mut request = client
        .post(url)
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(body);

    for (key, value) in &auth_headers {
        request = request.header(key.as_str(), value.as_str());
    }

    request
        .send()
        .await
        .map_err(|e| format!("Transcription request failed: {}", e))
}

/// Transcribe audio samples using the Codex/ChatGPT Whisper endpoint.
///
/// Matches the full Mhe.handleRequest pipeline from Codex Desktop:
///   1. Encode audio to WAV
///   2. Build multipart body (zxn/Hxn equivalent)
///   3. Apply auth headers (applyDesktopAuthHeaders equivalent)
///   4. POST to /transcribe
///   5. On 401, refresh token and retry (Mhe.handleRequest retry logic)
///   6. Return response.body.text.trim()
pub async fn transcribe_samples(
    auth: &CodexAuthManager,
    samples: &[f32],
    language: Option<&str>,
) -> Result<String, String> {
    let sample_rate = 16000u32;

    // Encode to WAV (matches getUserMedia → MediaRecorder pipeline)
    let wav_data = encode_wav(samples, sample_rate)?;
    debug!(
        "[codex_stt] Encoded {} samples to {} byte WAV",
        samples.len(),
        wav_data.len()
    );

    // Build multipart body matching Codex Desktop's $xn/Hxn/zxn functions
    // Boundary format: "----codex-transcribe-{crypto.randomUUID()}"
    let boundary = format!("----codex-transcribe-{}", Uuid::new_v4());
    // Filename: Uxn(opts.filename ?? `codex.${ext}`) — strips quotes
    let filename = "codex.wav";
    let content_type = "audio/wav";

    let body = build_multipart_body(&wav_data, &boundary, filename, content_type, language);

    // URL: ensureAbsoluteUrl("/transcribe") → apiBaseUrl + "/transcribe"
    let url = format!("{}/transcribe", CodexAuthManager::api_base_url());
    info!(
        "[codex_stt] POST {} (boundary={}, size={})",
        url, boundary, body.len()
    );

    // Get valid token (auto-refreshes if expired, matches getAuthToken flow)
    let (token, account_id) = auth.get_valid_token().await?;

    let resp = do_transcribe_request(
        &url,
        body.clone(),
        &boundary,
        &token,
        account_id.as_deref(),
    )
    .await?;

    // 401 retry with token refresh (matches Mhe.handleRequest):
    //   let p = await l(d);
    //   if (c && p.status === 401) {
    //     d = await getAuthToken({refreshToken: true});
    //     p = await l(d);
    //   }
    if resp.status().as_u16() == 401 {
        info!("[codex_stt] Got 401, refreshing token and retrying...");

        // Re-read credentials file (another process may have refreshed)
        auth.reload_from_file();
        let (new_token, new_account_id) = auth.get_valid_token().await?;

        let retry_resp = do_transcribe_request(
            &url,
            body,
            &boundary,
            &new_token,
            new_account_id.as_deref(),
        )
        .await?;

        if !retry_resp.status().is_success() {
            let status = retry_resp.status();
            let text = retry_resp.text().await.unwrap_or_default();
            return Err(format!(
                "Transcription failed after retry: HTTP {} — {}",
                status,
                &text[..text.len().min(500)]
            ));
        }

        let result: TranscribeResponse = retry_resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse transcription response: {}", e))?;

        // zxn does: (await zxn(blob)).trim()
        return Ok(result.text.trim().to_string());
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Transcription failed: HTTP {} — {}",
            status,
            &text[..text.len().min(500)]
        ));
    }

    let result: TranscribeResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse transcription response: {}", e))?;

    let transcript = result.text.trim().to_string();
    info!(
        "[codex_stt] Transcription complete: '{}' ({} chars)",
        &transcript[..transcript.len().min(100)],
        transcript.len()
    );

    Ok(transcript)
}
