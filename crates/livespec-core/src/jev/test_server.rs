//! A tiny HTTP/1.1 server for client tests. `serve` answers each connection
//! with the next canned response; `serve_with` lets a handler pick the reply
//! from the request body. Connections are served concurrently.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
pub struct Canned {
    status: u16,
    body: String,
    delay: Duration,
}

impl Canned {
    pub fn new(status: u16, body: &str) -> Self {
        Self { status, body: body.to_owned(), delay: Duration::ZERO }
    }

    /// A 200 with one Noul answer under question id `id`.
    pub fn noul(id: &str, value: f64) -> Self {
        Self::new(200, &format!(r#"{{"model":"jev-1.13.0","answers":{{"{id}":{{"type":"noul","noul":{value}}}}}}}"#))
    }

    /// Answer only after `ms` milliseconds, for timeout tests.
    pub fn delayed(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }
}

pub async fn serve(responses: Vec<Canned>) -> (String, Arc<AtomicUsize>) {
    let queue = Mutex::new(VecDeque::from(responses));
    serve_with(move |_: &str| queue.lock().unwrap().pop_front()).await
}

/// `handler` gets each request body; `None` closes the connection unanswered.
pub async fn serve_with(handler: impl Fn(&str) -> Option<Canned> + Send + Sync + 'static) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            let handler = handler.clone();
            tokio::spawn(async move {
                let body = read_request(&mut sock).await;
                let Some(r) = handler(&body) else { return };
                tokio::time::sleep(r.delay).await;
                let reply = format!(
                    "HTTP/1.1 {} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    r.status,
                    r.body.len(),
                    r.body
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base_url, hits)
}

/// Reads the headers and a Content-Length body; returns the body.
async fn read_request(sock: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let Ok(n) = sock.read(&mut chunk).await else { return String::new() };
        if n == 0 {
            return String::new();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                return String::from_utf8_lossy(&buf[end + 4..end + 4 + len]).into_owned();
            }
        }
    }
}
