//! Claude Voice Pipeline — WebSocket-based streaming speech-to-text.
//!
//! Connects to `wss://<claude.ai>/api/ws/speech_to_text/voice_stream`
//! and streams raw PCM audio chunks, receiving interim/final transcripts.
//!
//! On 401/403, re-reads ~/.claude/.credentials.json and retries with backoff.

use super::claude_auth::ClaudeAuthManager;
use super::{CloudSttStatusEvent, CloudTranscriptEvent};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Deserialize)]
struct WsMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

pub struct CloudSttSession {
    audio_tx: Option<mpsc::Sender<Vec<u8>>>,
    shutdown: Arc<AtomicBool>,
    is_connected: Arc<AtomicBool>,
    accumulated_transcript: Arc<Mutex<String>>,
    /// Notified when the session read loop finishes (transcript is final).
    done_notify: Arc<tokio::sync::Notify>,
}

impl CloudSttSession {
    pub fn start(
        app_handle: AppHandle,
        access_token: String,
        origin: String,
        language: String,
    ) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let is_connected = Arc::new(AtomicBool::new(false));
        let accumulated_transcript = Arc::new(Mutex::new(String::new()));
        let done_notify = Arc::new(tokio::sync::Notify::new());
        let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>(256);

        let shutdown_clone = shutdown.clone();
        let is_connected_clone = is_connected.clone();
        let accumulated_clone = accumulated_transcript.clone();
        let done_clone = done_notify.clone();

        tauri::async_runtime::spawn(async move {
            run_stt_session(
                app_handle,
                access_token,
                origin,
                language,
                audio_rx,
                shutdown_clone,
                is_connected_clone,
                accumulated_clone,
                done_clone,
            )
            .await;
        });

        Self {
            audio_tx: Some(audio_tx),
            shutdown,
            is_connected,
            accumulated_transcript,
            done_notify,
        }
    }

    pub fn send_audio(&self, data: Vec<u8>) {
        if let Some(ref tx) = self.audio_tx {
            let _ = tx.try_send(data);
        }
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }

    pub fn get_transcript(&self) -> String {
        self.accumulated_transcript.lock().unwrap().clone()
    }

    pub fn close(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.audio_tx.take();
    }

    /// Drop the audio channel (triggers CloseStream) and wait for the server
    /// to send the final transcript before returning it. Times out after 5s.
    pub async fn close_and_wait(&mut self) -> String {
        self.audio_tx.take();
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            self.done_notify.notified(),
        )
        .await;
        self.shutdown.store(true, Ordering::Relaxed);
        self.accumulated_transcript.lock().unwrap().clone()
    }
}

impl Drop for CloudSttSession {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.audio_tx.take();
    }
}

fn build_ws_url(origin: &str, language: &str) -> String {
    let ws_origin = origin
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!(
        "{}/api/ws/speech_to_text/voice_stream?encoding=linear16&sample_rate=16000&channels=1&endpointing_ms=300&utterance_end_ms=1000&language={}",
        ws_origin, language
    )
}

fn build_ws_request(ws_url: &str, token: &str) -> Result<Request<()>, String> {
    let mut request = ws_url
        .into_client_request()
        .map_err(|e| format!("Failed to build WS request: {}", e))?;
    let headers = request.headers_mut();
    headers.insert(
        "Authorization",
        format!("Bearer {}", token).parse().unwrap(),
    );
    headers.insert("x-app", "cli".parse().unwrap());
    headers.insert(
        "User-Agent",
        "claude-cli/2.1.74 (external, cli)".parse().unwrap(),
    );
    Ok(request)
}

/// Connect with retry + backoff. On 401/403, re-read credentials file and retry.
async fn connect_with_retry(
    app_handle: &AppHandle,
    ws_url: &str,
    initial_token: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        String,
    ),
    String,
> {
    let backoff_ms = [0, 1000, 3000];
    let mut token = initial_token.to_string();

    for (attempt, &delay) in backoff_ms.iter().enumerate() {
        if delay > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
        }

        let request = build_ws_request(ws_url, &token)?;
        info!(
            "[cloud_stt] Attempt {} connecting to: {}",
            attempt + 1,
            ws_url
        );

        match connect_async(request).await {
            Ok((stream, _)) => return Ok((stream, token)),
            Err(e) => {
                let err_str = e.to_string();
                let is_auth_error = err_str.contains("401") || err_str.contains("403");

                if is_auth_error && attempt < backoff_ms.len() - 1 {
                    warn!(
                        "[cloud_stt] Auth error on attempt {} ({}), re-reading credentials...",
                        attempt + 1,
                        err_str
                    );

                    let auth = app_handle.state::<Arc<ClaudeAuthManager>>();
                    if auth.reload_from_claude_code() {
                        if let Some(new_token) = auth.get_access_token() {
                            if new_token != token {
                                token = new_token;
                                info!("[cloud_stt] Got updated token, retrying...");
                                continue;
                            }
                        }
                    }
                    warn!("[cloud_stt] Token unchanged after re-read, retrying anyway...");
                } else {
                    return Err(format!("WebSocket connection failed: {}", e));
                }
            }
        }
    }

    Err("Max connection retries exceeded".to_string())
}

async fn run_stt_session(
    app_handle: AppHandle,
    access_token: String,
    origin: String,
    language: String,
    audio_rx: mpsc::Receiver<Vec<u8>>,
    shutdown: Arc<AtomicBool>,
    is_connected: Arc<AtomicBool>,
    accumulated_transcript: Arc<Mutex<String>>,
    done_notify: Arc<tokio::sync::Notify>,
) {
    let _ = app_handle.emit(
        "cloud-stt-status",
        CloudSttStatusEvent {
            status: "connecting".to_string(),
            message: None,
        },
    );

    let ws_url = build_ws_url(&origin, &language);
    info!("[cloud_stt] Connecting to {}", ws_url);

    let ws_stream = match connect_with_retry(&app_handle, &ws_url, &access_token).await {
        Ok((stream, _)) => stream,
        Err(e) => {
            error!("[cloud_stt] {}", e);
            let _ = app_handle.emit(
                "cloud-stt-status",
                CloudSttStatusEvent {
                    status: "error".to_string(),
                    message: Some(e),
                },
            );
            return;
        }
    };

    info!("[cloud_stt] WebSocket connected");
    is_connected.store(true, Ordering::Relaxed);

    let _ = app_handle.emit(
        "cloud-stt-status",
        CloudSttStatusEvent {
            status: "connected".to_string(),
            message: None,
        },
    );

    run_stt_stream(
        app_handle,
        ws_stream,
        audio_rx,
        shutdown,
        is_connected,
        accumulated_transcript,
        done_notify,
    )
    .await;
}

async fn run_stt_stream(
    app_handle: AppHandle,
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut audio_rx: mpsc::Receiver<Vec<u8>>,
    shutdown: Arc<AtomicBool>,
    is_connected: Arc<AtomicBool>,
    accumulated_transcript: Arc<Mutex<String>>,
    done_notify: Arc<tokio::sync::Notify>,
) {
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Send initial KeepAlive
    let keepalive_msg = serde_json::json!({"type": "KeepAlive"}).to_string();
    if let Err(e) = ws_write.send(Message::Text(keepalive_msg)).await {
        error!("[cloud_stt] Failed to send initial KeepAlive: {}", e);
        return;
    }

    // Keepalive task
    let shutdown_ka = shutdown.clone();
    let keepalive_handle = tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(8));
        loop {
            interval.tick().await;
            if shutdown_ka.load(Ordering::Relaxed) {
                break;
            }
        }
    });

    // Audio sender task
    let shutdown_audio = shutdown.clone();
    let (ws_write_tx, mut ws_write_rx) = mpsc::channel::<Message>(256);

    let audio_sender = tauri::async_runtime::spawn(async move {
        while let Some(data) = audio_rx.recv().await {
            if shutdown_audio.load(Ordering::Relaxed) {
                break;
            }
            let _ = ws_write_tx.send(Message::Binary(data.into())).await;
        }
        let close_msg = serde_json::json!({"type": "CloseStream"}).to_string();
        let _ = ws_write_tx.send(Message::Text(close_msg)).await;
    });

    // WS write forwarder
    let ws_writer = tauri::async_runtime::spawn(async move {
        while let Some(msg) = ws_write_rx.recv().await {
            if let Err(e) = ws_write.send(msg).await {
                debug!("[cloud_stt] WS write error: {}", e);
                break;
            }
        }
    });

    // Read loop
    let mut last_interim = String::new();
    let close_stream_sent = false;

    while let Some(msg_result) = ws_read.next().await {
        if shutdown.load(Ordering::Relaxed) && close_stream_sent {
            break;
        }

        match msg_result {
            Ok(Message::Text(text)) => {
                debug!("[cloud_stt] Received: {}", &text[..text.len().min(200)]);

                match serde_json::from_str::<WsMessage>(&text) {
                    Ok(ws_msg) => match ws_msg.msg_type.as_str() {
                        "TranscriptText" => {
                            if let Some(data) = ws_msg.data {
                                let trimmed = data.trim().to_string();
                                if !trimmed.is_empty() {
                                    last_interim = trimmed.clone();
                                    let _ = app_handle.emit(
                                        "cloud-stt-transcript",
                                        CloudTranscriptEvent {
                                            text: trimmed,
                                            is_final: false,
                                        },
                                    );
                                }
                            }
                        }
                        "TranscriptEndpoint" => {
                            if !last_interim.is_empty() {
                                let final_text = last_interim.clone();
                                {
                                    let mut acc = accumulated_transcript.lock().unwrap();
                                    if !acc.is_empty() {
                                        acc.push(' ');
                                    }
                                    acc.push_str(&final_text);
                                }
                                let _ = app_handle.emit(
                                    "cloud-stt-transcript",
                                    CloudTranscriptEvent {
                                        text: final_text,
                                        is_final: true,
                                    },
                                );
                                last_interim.clear();
                            }
                        }
                        "TranscriptError" => {
                            let desc = ws_msg
                                .description
                                .or(ws_msg.error_code)
                                .unwrap_or_else(|| "unknown error".to_string());
                            warn!("[cloud_stt] TranscriptError: {}", desc);
                            let _ = app_handle.emit(
                                "cloud-stt-status",
                                CloudSttStatusEvent {
                                    status: "error".to_string(),
                                    message: Some(desc),
                                },
                            );
                        }
                        "error" => {
                            let msg = ws_msg
                                .message
                                .unwrap_or_else(|| "server error".to_string());
                            error!("[cloud_stt] Server error: {}", msg);
                            let _ = app_handle.emit(
                                "cloud-stt-status",
                                CloudSttStatusEvent {
                                    status: "error".to_string(),
                                    message: Some(msg),
                                },
                            );
                        }
                        _ => {
                            debug!("[cloud_stt] Unknown message type: {}", ws_msg.msg_type);
                        }
                    },
                    Err(e) => {
                        debug!("[cloud_stt] Failed to parse WS message: {}", e);
                    }
                }
            }
            Ok(Message::Close(_)) => {
                info!("[cloud_stt] WebSocket closed by server");
                break;
            }
            Err(e) => {
                error!("[cloud_stt] WebSocket read error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Promote remaining interim to final
    if !last_interim.is_empty() {
        let mut acc = accumulated_transcript.lock().unwrap();
        if !acc.is_empty() {
            acc.push(' ');
        }
        acc.push_str(&last_interim);
        let _ = app_handle.emit(
            "cloud-stt-transcript",
            CloudTranscriptEvent {
                text: last_interim,
                is_final: true,
            },
        );
    }

    is_connected.store(false, Ordering::Relaxed);
    shutdown.store(true, Ordering::Relaxed);

    let _ = app_handle.emit(
        "cloud-stt-status",
        CloudSttStatusEvent {
            status: "disconnected".to_string(),
            message: None,
        },
    );

    keepalive_handle.abort();
    audio_sender.abort();
    ws_writer.abort();

    info!(
        "[cloud_stt] Session ended, final transcript: {}",
        accumulated_transcript.lock().unwrap()
    );
    done_notify.notify_waiters();
}
