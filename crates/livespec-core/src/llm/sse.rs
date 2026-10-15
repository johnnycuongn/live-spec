//! Minimal Server-Sent Events reader shared by the HTTP providers.

use futures_util::StreamExt;

use crate::{Error, Result};

/// Calls `on_data` with the `data:` payload of each event.
/// Return `Ok(false)` from the callback to stop early.
pub async fn for_each_data(
    resp: reqwest::Response,
    mut on_data: impl FnMut(&str) -> Result<bool>,
) -> Result<()> {
    let mut stream = resp.bytes_stream();
    // Byte buffer, not String: a chunk can split a multi-byte UTF-8 char.
    let mut buf: Vec<u8> = Vec::new();

    while let Some(bytes) = stream.next().await {
        buf.extend_from_slice(&bytes?);
        while let Some(end) = find_event_end(&buf) {
            let event: Vec<u8> = buf.drain(..end.0 + end.1).collect();
            let text = String::from_utf8_lossy(&event[..end.0]);
            let data = event_data(&text);
            if !data.is_empty() && !on_data(&data)? {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Returns (event_len, separator_len) for the first complete event.
fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// Joins multi-line `data:` fields per the SSE spec.
fn event_data(event: &str) -> String {
    event
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| d.strip_prefix(' ').unwrap_or(d))
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn ensure_success(resp: reqwest::Response, who: &str) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Provider(format!("{who} returned {status}: {}", truncate(&body, 400))))
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lf_and_crlf_events() {
        assert_eq!(find_event_end(b"data: a\n\ndata: b"), Some((7, 2)));
        assert_eq!(find_event_end(b"data: a\r\n\r\n"), Some((7, 4)));
        assert_eq!(find_event_end(b"data: partial"), None);
    }

    #[test]
    fn joins_multiline_data() {
        assert_eq!(event_data("event: x\ndata: one\ndata: two"), "one\ntwo");
    }
}
