use super::*;
use crate::cloud_stt::test_support::{auth_json, jwt, MockServer};

fn manual_auth() -> Arc<CodexAuthManager> {
    let manager = Arc::new(CodexAuthManager::test_config(
        None,
        "http://127.0.0.1:1/must-not-call".into(),
    ));
    manager.set_access_token(jwt("A", "manual", true)).unwrap();
    manager
}

#[test]
fn error_previews_handle_ascii_cyrillic_emoji_and_empty_input() {
    for text in [
        String::new(),
        "short".into(),
        "a".repeat(600),
        format!("a{}", "я".repeat(600)),
        "🙂".repeat(600),
    ] {
        let preview = error_preview(&text);
        assert_eq!(preview, text.chars().take(500).collect::<String>());
    }
}

#[test]
fn wav_encoding_and_multipart_preserve_audio_and_language() {
    let wav = encode_wav(&[-2.0, 0.0, 2.0], 16_000).unwrap();
    let mut reader = hound::WavReader::new(Cursor::new(&wav)).unwrap();
    assert_eq!(reader.spec().sample_rate, 16_000);
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(
        reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        vec![-32767, 0, 32767]
    );
    let body = build_multipart_body(&wav, "test-boundary", "codex.wav", "audio/wav", Some("ru"));
    assert!(body.windows(wav.len()).any(|part| part == wav));
    assert!(String::from_utf8_lossy(&body).contains("name=\"language\"\r\n\r\nru\r\n"));
    assert!(body.ends_with(b"--test-boundary--\r\n"));
}

#[tokio::test]
async fn unicode_success_is_returned_intact_including_the_old_panic_boundary() {
    let mut server = MockServer::start().await;
    let text = format!("a{}🙂", "я".repeat(100));
    let auth = manual_auth();
    let url = server.url.clone();
    let result =
        tokio::spawn(async move { transcribe_at(&auth, &[0.1; 1600], Some("ru"), &url).await });
    server.next().await.respond(
        200,
        serde_json::json!({"text": format!("  {text}  ")}).to_string(),
    );
    assert_eq!(result.await.unwrap().unwrap(), text);
}

#[tokio::test]
async fn unicode_http_errors_return_an_error_instead_of_panicking() {
    let mut server = MockServer::start().await;
    let auth = manual_auth();
    let url = server.url.clone();
    let result = tokio::spawn(async move { transcribe_at(&auth, &[0.1; 100], None, &url).await });
    server
        .next()
        .await
        .respond(500, format!("a{}", "я".repeat(600)));
    assert!(result
        .await
        .unwrap()
        .unwrap_err()
        .starts_with("Transcription failed: HTTP 500"));
}

#[tokio::test]
async fn unexpired_401_refreshes_once_and_retries_identical_audio() {
    for retry_status in [200, 401] {
        let mut server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let old = jwt("A", "old", true);
        let new = jwt("A", "new", true);
        std::fs::write(&path, auth_json(&old, "refresh-old")).unwrap();
        let auth = Arc::new(CodexAuthManager::test_config(
            Some(path),
            format!("{}/oauth/token", server.url),
        ));
        let url = format!("{}/transcribe", server.url);
        let result =
            tokio::spawn(async move { transcribe_at(&auth, &[0.1; 100], Some("ru"), &url).await });
        let first = server.next().await;
        assert!(first.headers.contains(&format!("Bearer {old}")));
        let audio = first.body.clone();
        first.respond(401, "{}");
        let refresh = server.next().await;
        assert!(refresh.headers.starts_with("POST /oauth/token "));
        refresh.respond(
            200,
            serde_json::json!({"access_token": new, "refresh_token": "refresh-new"}).to_string(),
        );
        let second = server.next().await;
        assert!(second.headers.contains(&format!("Bearer {new}")));
        assert_eq!(second.body, audio);
        second.respond(retry_status, "{\"text\":\"done\"}");
        let result = result.await.unwrap();
        if retry_status == 200 {
            assert_eq!(result.unwrap(), "done");
        } else {
            assert!(result.unwrap_err().contains("HTTP 401"));
        }
        server.assert_no_more_requests().await;
    }
}

#[tokio::test]
async fn manual_401_does_not_send_a_second_request_with_file_credentials() {
    let mut server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let file = auth_json(&jwt("B", "file", true), "refresh-B");
    std::fs::write(&path, &file).unwrap();
    let auth = Arc::new(CodexAuthManager::test_config(
        Some(path.clone()),
        format!("{}/oauth/token", server.url),
    ));
    auth.set_access_token(jwt("A", "manual", true)).unwrap();
    let url = server.url.clone();
    let result = tokio::spawn(async move { transcribe_at(&auth, &[0.1; 100], None, &url).await });
    server.next().await.respond(401, "{}");
    assert!(result
        .await
        .unwrap()
        .unwrap_err()
        .contains("manually entered"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), file);
    server.assert_no_more_requests().await;
}

#[tokio::test]
async fn deadline_bounds_a_stalled_http_response() {
    let mut server = MockServer::start().await;
    let auth = manual_auth();
    let url = server.url.clone();
    // Client initialization shares CPU with the entire parallel test suite.
    // Allow it to reach the fixture before testing a genuinely stalled response.
    let result = tokio::spawn(async move {
        transcribe_with_deadline(&auth, &[0.1; 100], None, &url, Duration::from_secs(2)).await
    });
    let _request_without_response = server.next().await;
    let error = tokio::time::timeout(Duration::from_secs(5), result)
        .await
        .expect("the transcription deadline must terminate a stalled response")
        .unwrap()
        .unwrap_err();
    assert!(error.contains("timed out"));
}

#[tokio::test]
async fn empty_audio_never_performs_http_or_authentication() {
    let mut server = MockServer::start().await;
    assert_eq!(
        transcribe_at(&manual_auth(), &[], None, &server.url)
            .await
            .unwrap(),
        ""
    );
    server.assert_no_more_requests().await;
}
