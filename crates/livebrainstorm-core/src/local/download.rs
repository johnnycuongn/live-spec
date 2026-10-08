//! Resumable HTTP download: multi-GB models over flaky Wi-Fi must survive
//! interruptions and app restarts.

use std::path::Path;

use futures_util::StreamExt;
use reqwest::{header, StatusCode};
use tokio::io::AsyncWriteExt;

use crate::{Error, Result};

const PROGRESS_EVERY: u64 = 1 << 20; // 1 MiB

/// Downloads `url` to `dest` via `dest.part`, resuming a previous partial file.
/// `on_progress(downloaded, total)` is throttled to roughly once per MiB.
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<()> {
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    ));
    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);

    let mut req = http.get(url);
    if have > 0 {
        req = req.header(header::RANGE, format!("bytes={have}-"));
    }
    let resp = req.send().await?;

    let append = match resp.status() {
        StatusCode::PARTIAL_CONTENT => true,
        StatusCode::OK => {
            have = 0; // server ignored the range: start over
            false
        }
        StatusCode::RANGE_NOT_SATISFIABLE if have > 0 => {
            // The .part is already complete; we just never renamed it.
            tokio::fs::rename(&part, dest).await?;
            return Ok(());
        }
        s => return Err(Error::Provider(format!("Download failed: {s} for {url}"))),
    };
    let total = resp.content_length().map(|len| len + have);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .await?;

    let mut done = have;
    let mut last_report = 0;
    on_progress(done, total);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if done - last_report >= PROGRESS_EVERY {
            last_report = done;
            on_progress(done, total);
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);

    if let Some(t) = total {
        if done != t {
            return Err(Error::Provider(format!("Download incomplete ({done} of {t} bytes). Try again to resume.")));
        }
    }
    tokio::fs::rename(&part, dest).await?;
    on_progress(done, total);
    Ok(())
}
