//! Bounded, cancellable I/O for OAuth integrations. Cancellation is available
//! without the client mutex; an old HTTP lease stays cancelled after renewal.
use std::time::Duration;
use tokio::sync::watch;

#[derive(Clone)]
pub(crate) struct Cancellation(watch::Sender<u64>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

#[derive(Clone)]
pub(crate) struct Http {
    pub client: reqwest::Client,
    cancellation: Cancellation,
    generation: u64,
}

pub(crate) struct Response {
    status: reqwest::StatusCode,
    body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> reqwest::StatusCode {
        self.status
    }
    pub fn json<T: serde::de::DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
    pub fn text(self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn bytes(self) -> Vec<u8> {
        self.body
    }
}

impl Http {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(60))
    }

    fn with_timeout(timeout: Duration) -> Self {
        let (sender, _) = watch::channel(0);
        Self {
            client: reqwest::Client::builder()
                .user_agent(crate::client::USER_AGENT)
                .connect_timeout(Duration::from_secs(10))
                .timeout(timeout)
                .build()
                .expect("failed to build OAuth HTTP client"),
            cancellation: Cancellation(sender),
            generation: 0,
        }
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    /// Only renew while holding the client mutex, after retiring old work.
    pub fn renew(&mut self) {
        self.generation = *self.cancellation.0.borrow();
    }

    pub fn ensure_current(&self) -> Result<(), String> {
        if self.generation == *self.cancellation.0.borrow() {
            Ok(())
        } else {
            Err("OAuth request was cancelled".into())
        }
    }

    pub async fn send(&self, request: reqwest::RequestBuilder) -> Result<Response, String> {
        let mut changed = self.cancellation.0.subscribe();
        self.ensure_current()?;
        let response = tokio::select! {
            biased;
            _ = changed.changed() => return Err("OAuth request was cancelled".into()),
            response = async {
                let response = request.send().await.map_err(|e| e.to_string())?;
                let status = response.status();
                // The cancellation and deadline cover the entire body, including
                // a server that sends headers and then stops responding.
                let body = response.bytes().await.map_err(|e| e.to_string())?.to_vec();
                Ok::<_, String>(Response { status, body })
            } => response?,
        };
        self.ensure_current()?;
        Ok(response)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) async fn token_server() -> (
        String,
        tokio::task::JoinHandle<std::collections::HashMap<String, String>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let (body_start, length) = loop {
                let mut buf = [0; 4096];
                let count = stream.read(&mut buf).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buf[..count]);
                if let Some(start) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..start]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    break (start + 4, length);
                }
            };
            while request.len() < body_start + length {
                let mut buf = [0; 4096];
                let count = stream.read(&mut buf).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buf[..count]);
            }
            let body = br#"{"access_token":"access","refresh_token":"refresh","expires_in":3600}"#;
            let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            url::form_urlencoded::parse(&request[body_start..body_start + length])
                .into_owned()
                .collect()
        });
        (url, server)
    }

    async fn stalled_server(
        headers: bool,
    ) -> (
        String,
        tokio::sync::oneshot::Receiver<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (started, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            stream.read(&mut buf).await.unwrap();
            if headers {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                    .await
                    .unwrap();
            }
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        (url, ready, server)
    }

    #[tokio::test]
    async fn cancellation_releases_client_lock_during_headers_or_body_wait() {
        for headers in [false, true] {
            let (url, ready, server) = stalled_server(headers).await;
            let http = Http::new();
            let cancel = http.cancellation();
            let client = std::sync::Arc::new(tokio::sync::Mutex::new(http));
            let worker_client = client.clone();
            let request = tokio::spawn(async move {
                let http = worker_client.lock().await;
                http.send(http.client.get(url)).await.map(|_| ())
            });
            ready.await.unwrap();
            cancel.cancel(); // Logout never needs the held client mutex to cancel.
            let mut guard = tokio::time::timeout(Duration::from_secs(1), client.lock())
                .await
                .unwrap();
            assert!(request.await.unwrap().unwrap_err().contains("cancelled"));
            let old = guard.clone();
            guard.renew();
            assert!(guard.ensure_current().is_ok());
            assert!(old.ensure_current().is_err());
            server.abort();
        }
    }

    #[tokio::test]
    async fn request_deadline_includes_stalled_body() {
        let (url, ready, server) = stalled_server(true).await;
        let http = Http::with_timeout(Duration::from_millis(100));
        let response =
            tokio::spawn(async move { http.send(http.client.get(url)).await.map(|_| ()) });
        ready.await.unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), response)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        server.abort();
    }
}
