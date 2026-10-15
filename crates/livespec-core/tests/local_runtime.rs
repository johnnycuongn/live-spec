//! Network test: downloads the real pinned llama.cpp build (~20-60 MB).
//! Run with `cargo test -p livespec-core -- --ignored`.

use livespec_core::local::{runtime, LocalPaths};

#[tokio::test]
#[ignore = "downloads the llama.cpp runtime"]
async fn installs_runtime_and_server_runs() {
    let dir = tempfile::tempdir().unwrap();
    let paths = LocalPaths::new(dir.path());
    let http = reqwest::Client::new();

    let mut reports = 0;
    let bin = runtime::install(&http, &paths, |_, _| reports += 1).await.unwrap();
    assert!(reports > 1, "progress should be reported");
    assert_eq!(paths.server_binary().as_deref(), Some(bin.as_path()));
    assert!(!paths.downloads_dir().join(runtime::asset_name().unwrap()).exists(), "archive cleaned up");

    // Shared libraries must resolve next to the binary.
    let out = std::process::Command::new(&bin)
        .arg("--version")
        .env("LD_LIBRARY_PATH", bin.parent().unwrap())
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("version"), "unexpected output: {text}");
}
