//! Finding executables from a GUI app.
//!
//! Apps launched from Finder/a desktop launcher get a minimal PATH, so tools
//! installed via npm, Homebrew or the Claude native installer are invisible.
//! We ask the user's login shell for its PATH once and reuse it both to find
//! `claude` and as the child's PATH (the npm build needs `node` on PATH).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static LOGIN_PATH: OnceLock<Option<String>> = OnceLock::new();

/// The PATH your terminal would have. Blocking (runs a shell once); call it
/// from a blocking context or at startup.
pub fn login_shell_path() -> Option<String> {
    LOGIN_PATH.get_or_init(read_login_shell_path).clone()
}

#[cfg(unix)]
fn read_login_shell_path() -> Option<String> {
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    const MARK: &str = "__LIVESPEC_PATH__";
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // Markers isolate PATH from anything rc files print.
        let out = Command::new(shell)
            .args(["-ilc", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    // A slow or interactive rc file must never hang the app.
    let out = rx.recv_timeout(Duration::from_secs(4)).ok()?.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find(MARK)? + MARK.len();
    let end = start + text[start..].find(MARK)?;
    Some(text[start..end].to_owned()).filter(|p| !p.is_empty())
}

#[cfg(not(unix))]
fn read_login_shell_path() -> Option<String> {
    // Windows GUI apps inherit the user PATH already.
    std::env::var("PATH").ok()
}

/// Looks for `name` on `path_env` (or the process PATH) plus well-known
/// install locations that are often missing from PATH.
pub fn find_executable(name: &str, path_env: Option<&str>) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_env
        .map(str::to_owned)
        .or_else(|| std::env::var("PATH").ok())
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();

    if let Some(home) = home_dir() {
        for rel in [".claude/local", ".local/bin", ".npm-global/bin", ".bun/bin", ".volta/bin"] {
            dirs.push(home.join(rel));
        }
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));

    let candidates: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ""] } else { &[""] };
    dirs.iter()
        .flat_map(|d| candidates.iter().map(move |ext| d.join(format!("{name}{ext}"))))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}
