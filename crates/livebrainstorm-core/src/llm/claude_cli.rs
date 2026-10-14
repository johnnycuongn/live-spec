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

use super::{ChatRequest, Chunk, OnChunk, Secret};
use crate::settings::Effort;
use crate::{Error, Result};

/// The only tools a brief explorer gets. An allowlist (`--tools`), not a
/// denylist, so tools added to future Claude Code versions stay unavailable.
const READ_ONLY_TOOLS: &str = "Read,Glob,Grep";

#[derive(Debug, Clone)]
pub struct ClaudeCli {
    pub binary: PathBuf,
    /// Model alias ("haiku", "sonnet", "opus", "fable") or a full model id.
    pub model: String,
    /// `--effort`; None leaves Claude Code's default.
    pub effort: Option<Effort>,
    /// The user's Anthropic API key, given to the child as ANTHROPIC_API_KEY.
    /// None: Claude Code's own login.
    pub api_key: Option<Secret>,
    /// PATH for the child process (see `shell_env`).
    pub path_env: Option<String>,
}

/// Flags for one `claude -p` run. Opened projects may be untrusted clones,
/// so `--restricted` ignores settings files (no project hooks), confines file
/// tools to the project and refuses bypassPermissions, and
/// `--strict-mcp-config` without `--mcp-config` starts no MCP servers.
fn args(model: &str, effort: Option<Effort>, req: &ChatRequest) -> Vec<String> {
    let turns = req.max_turns.unwrap_or(if req.allow_repo_tools { 40 } else { 1 }).to_string();
    let mut a: Vec<String> = [
        "-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
        "--model", model, "--restricted", "--strict-mcp-config",
    ]
    .map(String::from)
    .to_vec();
    // Before `--tools`: that option takes a variadic list.
    if let Some(e) = effort {
        a.extend(["--effort", e.as_str()].map(String::from));
    }
    if req.allow_repo_tools {
        // Keep Claude Code's own system prompt: it knows how to explore repos.
        a.extend(["--append-system-prompt", req.system.as_str(), "--max-turns", turns.as_str()].map(String::from));
        a.extend(["--tools", READ_ONLY_TOOLS, "--allowedTools", READ_ONLY_TOOLS].map(String::from));
    } else {
        // Live analysis: replace the system prompt, single turn, no tools at all.
        // Smaller prompt = faster first token while the user types.
        a.extend(["--system-prompt", req.system.as_str(), "--max-turns", turns.as_str(), "--tools", ""].map(String::from));
    }
    a
}

impl ClaudeCli {
    /// Claude Code's own login and default effort; set `effort` / `api_key` after.
    pub fn new(binary: PathBuf, model: impl Into<String>, path_env: Option<String>) -> Self {
        Self { binary, model: model.into(), effort: None, api_key: None, path_env }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.binary);
        if let Some(p) = &self.path_env {
            cmd.env("PATH", p);
        }
        // The environment, never argv: any process can list another's arguments.
        if let Some(key) = &self.api_key {
            cmd.env("ANTHROPIC_API_KEY", key.expose());
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
        cmd.args(args(&self.model, self.effort, req));
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
    if stderr.contains("unknown option") {
        return "Claude Code is too old for LiveBrainstorm. Update it with `claude update`.".into();
    }
    let detail = result_text.filter(|t| !t.trim().is_empty()).unwrap_or(stderr).trim();
    if detail.contains("Invalid API key") {
        return "Claude Code rejected the Anthropic API key. Check the key in Settings, or switch Claude Code back to your login."
            .into();
    }
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

    fn request(allow_repo_tools: bool) -> ChatRequest {
        ChatRequest { system: "sys".into(), user: "u".into(), max_tokens: 900, cwd: None, allow_repo_tools, max_turns: None }
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str())
    }

    #[test]
    fn analysis_runs_with_no_tools_and_no_workspace_config() {
        let a = args("haiku", None, &request(false));
        assert_eq!(after(&a, "--tools"), Some(""));
        assert_eq!(after(&a, "--system-prompt"), Some("sys"));
        assert_eq!(after(&a, "--max-turns"), Some("1"));
        assert!(a.contains(&"--restricted".into()) && a.contains(&"--strict-mcp-config".into()));
        assert!(!a.iter().any(|x| x.contains("disallowed") || x.contains("Bash")));
    }

    #[test]
    fn brief_explorers_only_get_read_only_tools() {
        let a = args("sonnet", None, &request(true));
        assert_eq!(after(&a, "--tools"), Some("Read,Glob,Grep"));
        assert_eq!(after(&a, "--allowedTools"), Some("Read,Glob,Grep"));
        assert_eq!(after(&a, "--append-system-prompt"), Some("sys"));
        assert_eq!(after(&a, "--model"), Some("sonnet"));
        assert!(a.contains(&"--restricted".into()) && a.contains(&"--strict-mcp-config".into()));
    }

    #[test]
    fn errors_say_what_to_do() {
        assert_eq!(
            cli_error(None, "error: unknown option '--effort'"),
            "Claude Code is too old for LiveBrainstorm. Update it with `claude update`."
        );
        assert_eq!(
            cli_error(Some("Invalid API key · Fix external API key"), ""),
            "Claude Code rejected the Anthropic API key. Check the key in Settings, or switch Claude Code back to your login."
        );
        assert_eq!(cli_error(Some("boom"), ""), "Claude Code failed: boom");
    }

    #[test]
    fn effort_is_passed_only_when_set() {
        let a = args("haiku", None, &request(false));
        assert!(!a.contains(&"--effort".into()), "Default leaves Claude Code's own effort");
        assert_eq!(after(&args("haiku", Some(Effort::Low), &request(false)), "--effort"), Some("low"));
        let a = args("sonnet", Some(Effort::Xhigh), &request(true));
        assert_eq!(after(&a, "--effort"), Some("xhigh"));
        assert_eq!(after(&a, "--tools"), Some("Read,Glob,Grep"), "the variadic tool list still ends where it should");
    }

    #[test]
    fn the_api_key_goes_in_the_environment_never_in_args() {
        use std::ffi::OsStr;
        let cli = ClaudeCli {
            api_key: Some(Secret::new("sk-ant-secret")),
            effort: Some(Effort::Low),
            ..ClaudeCli::new("claude".into(), "haiku", None)
        };
        let cmd = cli.command();
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(envs.contains(&(OsStr::new("ANTHROPIC_API_KEY"), Some(OsStr::new("sk-ant-secret")))));
        let a = args(&cli.model, cli.effort, &request(false));
        assert!(!a.iter().any(|x| x.contains("sk-ant")), "every process can read another's argv");
        assert!(!format!("{cli:?}").contains("sk-ant-secret"), "Debug output never carries the key");
        let login = ClaudeCli::new("claude".into(), "haiku", None).command();
        assert!(!login.as_std().get_envs().any(|(k, _)| k == "ANTHROPIC_API_KEY"), "login mode leaves the environment alone");
    }

    #[test]
    fn a_turn_limit_overrides_the_default() {
        let req = ChatRequest { max_turns: Some(20), ..request(true) };
        assert_eq!(after(&args("sonnet", None, &req), "--max-turns"), Some("20"));
        assert_eq!(after(&args("sonnet", None, &request(true)), "--max-turns"), Some("40"));
    }
}
