//! A tiny HTTP/1.1 server for client tests: each connection gets the next
//! canned response, in order, then the connection closes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
pub struct Canned {
    status: u16,
    body: String,
}

impl Canned {
    pub fn new(status: u16, body: &str) -> Self {
        Self { status, body: body.to_owned() }
    }
}

pub async fn serve(responses: Vec<Canned>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        for r in responses {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            read_request(&mut sock).await;
            let reply = format!(
                "HTTP/1.1 {} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                r.status,
                r.body.len(),
                r.body
            );
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (base_url, hits)
}

/// Reads the headers and a Content-Length body, so the client sees a clean exchange.
async fn read_request(sock: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let Ok(n) = sock.read(&mut chunk).await else { return };
        if n == 0 {
            return;
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
                return;
            }
        }
    }
}
