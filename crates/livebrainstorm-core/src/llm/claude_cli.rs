//! Claude through the user's own Claude Code install (`claude -p`).
//!
//! Why not our own "Sign in with Claude"? Anthropic only permits subscription
//! OAuth inside Claude Code and claude.ai; third-party apps must not offer
//! Claude.ai login or reuse those tokens. So we never touch credentials: we run
//! the unmodified `claude` binary, which uses whatever login the user already
//! has. Fine for personal use. If LiveBrainstorm is ever shipped to other people, they
//! should use the API-key provider instead.

use std::path::PathBuf;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::{ChatRequest, Chunk, OnChunk};
use crate::{Error, Result};

const READ_ONLY_TOOLS: [&str; 3] = ["Read", "Glob", "Grep"];
const MUTATING_TOOLS: [&str; 7] =
    ["Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch", "Task"];

#[derive(Debug, Clone)]
pub struct ClaudeCli {
    pub binary: PathBuf,
    /// Model alias ("haiku", "sonnet", "opus") or a full model id.
    pub model: String,
    /// PATH for the child process (see `shell_env`).
    pub path_env: Option<String>,
}

impl ClaudeCli {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        if let Some(p) = &self.path_env {
            cmd.env("PATH", p);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
        cmd
    }

    pub async fn version(&self) -> Result<String> {
        let out = self.command().arg("--version").output().await.map_err(|e| self.spawn_error(e))?;
        if !out.status.success() {
            return Err(Error::Provider(String::from_utf8_lossy(&out.stderr).trim().to_owned()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    pub async fn stream(&self, req: &ChatRequest, on_chunk: OnChunk<'_>) -> Result<String> {
        let mut cmd = self.command();
        cmd.arg("-p")
            .args(["--output-format", "stream-json", "--verbose", "--include-partial-messages"])
            .args(["--model", &self.model]);

        if req.allow_repo_tools {
            // Keep Claude Code's own system prompt: it knows how to explore repos.
            cmd.arg("--append-system-prompt").arg(&req.system).args(["--max-turns", "40"]);
            cmd.arg("--allowedTools").args(READ_ONLY_TOOLS);
            cmd.arg("--disallowedTools").args(MUTATING_TOOLS);
        } else {
            // Live analysis: replace the system prompt, single turn, no tools.
            // Smaller prompt = faster first token while the user types.
            cmd.arg("--system-prompt").arg(&req.system).args(["--max-turns", "1"]);
            cmd.arg("--disallowedTools").args(MUTATING_TOOLS).args(READ_ONLY_TOOLS);
        }
        if let Some(cwd) = &req.cwd {
            cmd.current_dir(cwd);
        }
        // kill_on_drop: aborting the owning task (user kept typing) kills the CLI.
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| self.spawn_error(e))?;

        // Prompt goes through stdin: no argv length limits (Windows caps ~32k).
        let mut stdin = child.stdin.take().expect("stdin piped");
        stdin.write_all(req.user.as_bytes()).await?;
        drop(stdin);

        // Drain stderr concurrently so a chatty CLI can't block on a full pipe.
        let mut stderr = child.stderr.take().expect("stderr piped");
        let stderr_task = tokio::spawn(async move {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s).await;
            s
        });

        let mut lines = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
        let mut streamed = String::new();
        let mut result: Option<(Option<String>, bool)> = None;

        while let Some(line) = lines.next_line().await? {
            for event in parse_line(&line) {
                match event {
                    CliEvent::Text(t) => {
                        streamed.push_str(&t);
                        on_chunk(Chunk::Text(t));
                    }
                    CliEvent::ToolUse(label) => on_chunk(Chunk::Activity(label)),
                    CliEvent::Result { text, is_error } => result = Some((text, is_error)),
                }
            }
        }

        let status = child.wait().await?;
        let stderr = stderr_task.await.unwrap_or_default();

        match result {
            // `result` is the final assistant message; for multi-turn exploration
            // it excludes the "let me look at..." narration we streamed.
            Some((Some(text), false)) => Ok(text),
            Some((_, false)) => Ok(streamed),
            // e.g. hit max turns after producing text: keep what we have.
            Some((_, true)) if !streamed.trim().is_empty() => Ok(streamed),
            Some((text, true)) => Err(Error::Provider(cli_error(text.as_deref(), &stderr))),
            None if status.success() => Ok(streamed),
            None => Err(Error::Provider(cli_error(None, &stderr))),
        }
    }

    fn spawn_error(&self, e: std::io::Error) -> Error {
        Error::Precondition(format!(
            "Couldn't run Claude Code at '{}' ({e}). Install it, run `claude` once to log in, \
             or set its path in Settings.",
            self.binary.display()
        ))
    }
}

fn cli_error(result_text: Option<&str>, stderr: &str) -> String {
    let detail = result_text.filter(|t| !t.trim().is_empty()).unwrap_or(stderr).trim();
    let detail = if detail.is_empty() { "no output" } else { detail };
    format!("Claude Code failed: {}", detail.chars().take(500).collect::<String>())
}

#[derive(Debug, PartialEq)]
enum CliEvent {
    Text(String),
    ToolUse(String),
    Result { text: Option<String>, is_error: bool },
}

/// One NDJSON line from `--output-format stream-json`. Unknown shapes are
/// ignored so CLI upgrades that add event types don't break us.
fn parse_line(line: &str) -> Vec<CliEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return vec![] };
    match v["type"].as_str() {
        Some("stream_event") => {
            let e = &v["event"];
            match (e["type"].as_str(), e["delta"]["type"].as_str(), e["delta"]["text"].as_str()) {
                (Some("content_block_delta"), Some("text_delta"), Some(t)) => {
                    vec![CliEvent::Text(t.to_owned())]
                }
                _ => vec![],
            }
        }
        // Full messages arrive too; we only need tool calls from them
        // (their text already streamed through deltas).
        Some("assistant") => v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| CliEvent::ToolUse(describe_tool(&b["name"], &b["input"])))
            .collect(),
        Some("result") => vec![CliEvent::Result {
            text: v["result"].as_str().map(str::to_owned),
            is_error: v["is_error"].as_bool().unwrap_or(false),
        }],
        _ => vec![],
    }
}

fn describe_tool(name: &Value, input: &Value) -> String {
    let name = name.as_str().unwrap_or("tool");
    let arg = ["file_path", "pattern", "path"].iter().find_map(|k| input[*k].as_str());
    match arg {
        Some(a) => format!("{name} {a}"),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_delta() {
        let l = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#;
        assert_eq!(parse_line(l), vec![CliEvent::Text("Hi".into())]);
    }

    #[test]
    fn parses_tool_use_and_result() {
        let a = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"x"},{"type":"tool_use","name":"Read","input":{"file_path":"src/main.rs"}}]}}"#;
        assert_eq!(parse_line(a), vec![CliEvent::ToolUse("Read src/main.rs".into())]);
        let r = r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#;
        assert_eq!(parse_line(r), vec![CliEvent::Result { text: Some("done".into()), is_error: false }]);
    }

    #[test]
    fn ignores_noise() {
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
    }
}
