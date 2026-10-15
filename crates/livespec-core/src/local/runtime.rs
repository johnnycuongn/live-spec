//! The managed llama.cpp server: install, start, health-check, stop.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

use super::{download, LocalPaths};
use crate::{Error, Result};

/// Pinned llama.cpp build. Pinning beats "latest": reproducible installs, no
/// GitHub API rate limits, and upgrades are a deliberate one-line change.
pub const LLAMA_CPP_BUILD: &str = "b11480";
const RELEASES: &str = "https://github.com/ggml-org/llama.cpp/releases/download";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);

/// CPU builds everywhere except macOS (Metal is built in). Users with a
/// GPU on Windows/Linux can switch the suffix to a Vulkan build.
pub fn asset_name() -> Option<String> {
    let suffix = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64.tar.gz",
        ("macos", "x86_64") => "macos-x64.tar.gz",
        ("linux", "x86_64") => "ubuntu-x64.tar.gz",
        ("linux", "aarch64") => "ubuntu-arm64.tar.gz",
        ("windows", "x86_64") => "win-cpu-x64.zip",
        ("windows", "aarch64") => "win-cpu-arm64.zip",
        _ => return None,
    };
    Some(format!("llama-{LLAMA_CPP_BUILD}-bin-{suffix}"))
}

pub fn find_server_binary(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).ok()?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == name) {
                return Some(p);
            }
        }
    }
    None
}

/// Downloads and unpacks the runtime. Extracts into a temp dir and swaps it
/// in, so a failed install never leaves a half-unpacked runtime behind.
pub async fn install(
    http: &reqwest::Client,
    paths: &LocalPaths,
    on_progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<PathBuf> {
    let asset = asset_name()
        .ok_or_else(|| Error::Precondition("Local models aren't supported on this platform yet.".into()))?;
    let archive = paths.downloads_dir().join(&asset);
    download::download(http, &format!("{RELEASES}/{LLAMA_CPP_BUILD}/{asset}"), &archive, on_progress).await?;

    let target = paths.runtime_dir();
    let staging = paths.root.join("runtime.staging");
    tokio::task::spawn_blocking(move || -> Result<()> {
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        extract(&archive, &staging)?;
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&staging, &target)?;
        let _ = std::fs::remove_file(&archive);
        Ok(())
    })
    .await
    .map_err(|e| Error::Archive(e.to_string()))??;

    paths
        .server_binary()
        .ok_or_else(|| Error::Archive("llama-server not found in the downloaded runtime".into()))
}

fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let name = archive.to_string_lossy();
    if name.ends_with(".zip") {
        // `extract` rejects paths escaping `dest`.
        zip::ZipArchive::new(file)
            .and_then(|mut z| z.extract(dest))
            .map_err(|e| Error::Archive(e.to_string()))
    } else {
        // `unpack` also sanitizes paths and keeps exec bits and lib symlinks.
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        tar.set_preserve_permissions(true);
        tar.unpack(dest).map_err(|e| Error::Archive(e.to_string()))
    }
}

/// A running llama-server bound to localhost. Dropping it kills the process.
#[derive(Debug)]
pub struct LocalServer {
    child: Child,
    pub port: u16,
    pub model_id: String,
    pub context_tokens: u32,
}

impl LocalServer {
    pub async fn start(
        http: &reqwest::Client,
        paths: &LocalPaths,
        model_id: &str,
        model_path: &Path,
        context_tokens: u32,
    ) -> Result<Self> {
        let binary = paths
            .server_binary()
            .ok_or_else(|| Error::Precondition("Install the local runtime first (Settings).".into()))?;
        if !model_path.is_file() {
            return Err(Error::Precondition(format!("Download the {model_id} model first (Settings).")));
        }
        let lib_dir = binary.parent().unwrap_or(Path::new(".")).to_path_buf();
        let port = free_port()?;
        let log = std::fs::File::create(paths.log_path())?;

        let mut cmd = Command::new(&binary);
        cmd.arg("-m")
            .arg(model_path)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["-c", &context_tokens.to_string()])
            .current_dir(&lib_dir)
            .env("LD_LIBRARY_PATH", &lib_dir) // Linux: shared libs sit next to the binary
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let child = cmd.spawn()?;
        let mut server = Self { child, port, model_id: model_id.to_owned(), context_tokens };
        server.wait_healthy(http, paths).await?;
        Ok(server)
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }

    /// `/health` answers 503 while the model loads and 200 once it's ready.
    async fn wait_healthy(&mut self, http: &reqwest::Client, paths: &LocalPaths) -> Result<()> {
        let url = format!("{}/health", self.base_url());
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(Error::Provider(format!(
                    "Local model server exited ({status}). Last log lines:\n{}",
                    tail(&paths.log_path(), 12)
                )));
            }
            if let Ok(r) = http.get(&url).timeout(Duration::from_secs(2)).send().await {
                if r.status().is_success() {
                    return Ok(());
                }
            }
            if started.elapsed() > STARTUP_TIMEOUT {
                let _ = self.child.kill().await;
                return Err(Error::Provider("Local model took too long to load.".into()));
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
}

fn free_port() -> Result<u16> {
    // Bind-then-release has a tiny race window; acceptable for a local app.
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn tail(path: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_asset_for_this_platform() {
        let name = asset_name().expect("CI platforms are supported");
        assert!(name.starts_with(&format!("llama-{LLAMA_CPP_BUILD}-bin-")));
    }
}
