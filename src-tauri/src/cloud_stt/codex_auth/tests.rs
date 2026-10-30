use super::*;
use crate::cloud_stt::test_support::{auth_json, jwt, MockServer};

fn manager(path: Option<PathBuf>, url: String) -> Arc<CodexAuthManager> {
    Arc::new(CodexAuthManager::with_config(path, url))
}

#[test]
fn empty_manual_token_is_rejected_without_discarding_a_session() {
    let manager = CodexAuthManager::with_config(None, AUTH_TOKEN_URL.into());
    assert!(manager.set_access_token(" \n".into()).is_err());
    let token = jwt("A", "manual", true);
    manager.set_access_token(token.clone()).unwrap();
    assert!(manager.set_access_token(String::new()).is_err());
    assert_eq!(
        manager
            .lock_state()
            .credentials
            .as_ref()
            .unwrap()
            .access_token,
        token
    );
}

#[tokio::test]
async fn manual_tokens_never_adopt_the_account_in_auth_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let file = auth_json(&jwt("B", "file", true), "refresh-B");
    std::fs::write(&path, &file).unwrap();
    let manager = manager(
        Some(path.clone()),
        "http://127.0.0.1:1/must-not-call".into(),
    );
    for valid in [false, true] {
        let token = jwt("A", "manual", valid);
        manager.set_access_token(token.clone()).unwrap();
        let error = if valid {
            manager.refresh_after_rejection(&token).await
        } else {
            manager.get_valid_token().await
        }
        .unwrap_err();
        assert!(error.contains("manually entered"));
        assert_eq!(
            manager
                .lock_state()
                .credentials
                .as_ref()
                .unwrap()
                .account_id
                .as_deref(),
            Some("A")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
    }
}

#[tokio::test]
async fn rejection_forces_refresh_of_an_unexpired_token_and_preserves_json() {
    let mut server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let old = jwt("A", "old", true);
    let new = jwt("A", "new", true);
    std::fs::write(&path, auth_json(&old, "refresh-old")).unwrap();
    let manager = manager(Some(path.clone()), format!("{}/oauth/token", server.url));
    let worker = Arc::clone(&manager);
    let result = tokio::spawn(async move { worker.refresh_after_rejection(&old).await });
    let request = server.next().await;
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["refresh_token"], "refresh-old");
    assert_eq!(body["grant_type"], "refresh_token");
    request.respond(
        200,
        serde_json::json!({"access_token": new, "refresh_token": "refresh-new"}).to_string(),
    );
    assert_eq!(result.await.unwrap().unwrap().0, new);
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(file["tokens"]["refresh_token"], "refresh-new");
    assert_eq!(file["tokens"]["id_token"], "id");
    assert_eq!(file["tokens"]["unknown_token_field"], 123);
    assert_eq!(file["unrelated"]["keep"], true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    server.assert_no_more_requests().await;
}

#[tokio::test]
async fn two_managers_share_a_file_lock_and_do_not_rotate_twice() {
    let mut server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(&path, auth_json(&jwt("A", "expired", false), "refresh-old")).unwrap();
    let first = manager(Some(path.clone()), server.url.clone());
    let second = manager(Some(path), server.url.clone());
    let a = tokio::spawn(async move { first.get_valid_token().await });
    let b = tokio::spawn(async move { second.get_valid_token().await });
    let token = jwt("A", "fresh", true);
    server
        .next()
        .await
        .respond(200, serde_json::json!({"access_token": token}).to_string());
    assert_eq!(a.await.unwrap().unwrap().0, token);
    assert_eq!(b.await.unwrap().unwrap().0, token);
    server.assert_no_more_requests().await;
}

#[tokio::test]
async fn externally_refreshed_file_is_reused_but_different_account_is_rejected() {
    let mut server = MockServer::start().await;
    for account in ["A", "B"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, auth_json(&jwt("A", "expired", false), "old")).unwrap();
        let manager = manager(Some(path.clone()), server.url.clone());
        let replacement = jwt(account, "external", true);
        let file = auth_json(&replacement, "external-refresh");
        std::fs::write(&path, &file).unwrap();
        let result = manager.get_valid_token().await;
        if account == "A" {
            assert_eq!(result.unwrap().0, replacement);
        } else {
            assert!(result.unwrap_err().contains("account changed"));
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), file);
    }
    server.assert_no_more_requests().await;
}

#[tokio::test]
async fn concurrent_external_write_is_never_overwritten() {
    let mut server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(&path, auth_json(&jwt("A", "expired", false), "old")).unwrap();
    let manager = manager(Some(path.clone()), server.url.clone());
    let worker = Arc::clone(&manager);
    let result = tokio::spawn(async move { worker.get_valid_token().await });
    let request = server.next().await;
    let external = auth_json(&jwt("B", "external-account", true), "external-refresh");
    std::fs::write(&path, &external).unwrap();
    request.respond(
        200,
        serde_json::json!({"access_token": jwt("A", "rotated", true)}).to_string(),
    );
    assert!(result
        .await
        .unwrap()
        .unwrap_err()
        .contains("account changed"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
    assert_eq!(
        manager
            .lock_state()
            .credentials
            .as_ref()
            .unwrap()
            .account_id
            .as_deref(),
        Some("A")
    );
}

#[tokio::test]
async fn logout_or_manual_switch_during_refresh_is_not_undone() {
    for switch_to_manual in [false, true] {
        let mut server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, auth_json(&jwt("A", "expired", false), "old")).unwrap();
        let manager = manager(Some(path.clone()), server.url.clone());
        let worker = Arc::clone(&manager);
        let result = tokio::spawn(async move { worker.get_valid_token().await });
        let request = server.next().await;
        if switch_to_manual {
            manager.set_access_token(jwt("B", "manual", true)).unwrap();
        } else {
            manager.logout();
        }
        let refreshed = jwt("A", "refreshed", true);
        request.respond(
            200,
            serde_json::json!({"access_token": refreshed, "refresh_token": "rotated"}).to_string(),
        );
        assert!(result.await.unwrap().unwrap_err().contains("login changed"));
        if switch_to_manual {
            assert_eq!(
                manager.get_valid_token().await.unwrap().1.as_deref(),
                Some("B")
            );
        } else {
            assert!(!manager.get_state().is_logged_in);
        }
        // Finish an already-started rotation without restoring the UI session.
        assert_eq!(
            read_auth_file(&path).unwrap().1.tokens.access_token,
            refreshed
        );
    }
}

#[tokio::test]
async fn cancelled_transcription_does_not_drop_rotated_refresh_credentials() {
    let mut server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(&path, auth_json(&jwt("A", "expired", false), "old")).unwrap();
    let manager = manager(Some(path.clone()), server.url.clone());
    let worker = Arc::clone(&manager);
    let caller = tokio::spawn(async move { worker.get_valid_token().await });
    let request = server.next().await;
    caller.abort();
    let refreshed = jwt("A", "refreshed", true);
    request.respond(
        200,
        serde_json::json!({"access_token": refreshed, "refresh_token": "rotated"}).to_string(),
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if read_auth_file(&path)
                .unwrap()
                .1
                .tokens
                .refresh_token
                .as_deref()
                == Some("rotated")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(manager.get_valid_token().await.unwrap().0, refreshed);
}

#[tokio::test]
async fn refresh_errors_and_invalid_responses_leave_the_original_file_intact() {
    for (status, body) in [
        (500, "я".repeat(600)),
        (200, "{\"access_token\":\"\"}".to_owned()),
        (
            200,
            serde_json::json!({"access_token": jwt("B", "wrong-account", true)}).to_string(),
        ),
    ] {
        let mut server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let original = auth_json(&jwt("A", "expired", false), "old");
        std::fs::write(&path, &original).unwrap();
        let manager = manager(Some(path.clone()), server.url.clone());
        let result = tokio::spawn(async move { manager.get_valid_token().await });
        server.next().await.respond(status, body);
        assert!(result.await.unwrap().is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }
}

#[test]
fn missing_file_clears_file_session_but_does_not_discard_manual_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(&path, auth_json(&jwt("A", "file", true), "refresh")).unwrap();
    let manager = manager(Some(path.clone()), AUTH_TOKEN_URL.into());
    std::fs::remove_file(path).unwrap();
    assert!(!manager.reload_from_file());
    assert!(!manager.get_state().is_logged_in);
    manager.set_access_token(jwt("A", "manual", true)).unwrap();
    assert!(!manager.reload_from_file());
    assert!(manager.get_state().is_logged_in);
}
