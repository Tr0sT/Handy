//! Loopback-only HTTP fixture. Never reads real credentials or contacts a provider.
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

pub struct Request {
    pub headers: String,
    pub body: Vec<u8>,
    reply: oneshot::Sender<(u16, String)>,
}
impl Request {
    pub fn respond(self, status: u16, body: impl Into<String>) {
        let _ = self.reply.send((status, body.into()));
    }
}

pub struct MockServer {
    pub url: String,
    requests: mpsc::UnboundedReceiver<Request>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl MockServer {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let Ok((mut socket, _)) = result else { break; };
                        let tx = tx.clone();
                        connections.spawn(async move {
                            let mut data = Vec::new();
                            let end = loop {
                                let mut chunk = [0; 4096];
                                let n = socket.read(&mut chunk).await.unwrap();
                                if n == 0 { return; }
                                data.extend_from_slice(&chunk[..n]);
                                if let Some(pos) = data.windows(4).position(|v| v == b"\r\n\r\n") { break pos + 4; }
                                assert!(data.len() < 65_536, "oversized fixture headers");
                            };
                            let headers = String::from_utf8(data[..end].to_vec()).unwrap();
                            let len = headers.lines().filter_map(|line| line.split_once(':'))
                                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap()).unwrap_or(0);
                            while data.len() < end + len {
                                let mut chunk = [0; 4096];
                                let n = socket.read(&mut chunk).await.unwrap();
                                if n == 0 { return; }
                                data.extend_from_slice(&chunk[..n]);
                            }
                            let (reply, response) = oneshot::channel();
                            if tx.send(Request { headers, body: data[end..end+len].to_vec(), reply }).is_err() { return; }
                            if let Ok((status, body)) = response.await {
                                let message = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                                let _ = socket.write_all(message.as_bytes()).await;
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    pub async fn next(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(3), self.requests.recv())
            .await
            .expect("expected a loopback request")
            .expect("mock server closed")
    }
    pub async fn assert_no_more_requests(&mut self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(50), self.requests.recv())
                .await
                .is_err()
        );
    }
}

pub fn jwt(account: &str, tag: &str, valid: bool) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let exp = if valid {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600
    } else {
        1
    };
    let body = serde_json::json!({"exp": exp, "sub": format!("user-{account}"), "nonce": tag,
        "https://api.openai.com/auth": {"chatgpt_account_id": account}});
    format!("e30.{}.fixture", URL_SAFE_NO_PAD.encode(body.to_string()))
}

pub fn auth_json(token: &str, refresh: &str) -> String {
    serde_json::json!({"auth_mode": "chatgpt", "unrelated": {"keep": true},
        "tokens": {"access_token": token, "refresh_token": refresh, "id_token": "id", "unknown_token_field": 123}}).to_string()
}
