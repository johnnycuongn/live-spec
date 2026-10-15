//! Claude through the user's own Claude Code install (`claude -p`).
//!
//! Why not our own "Sign in with Claude"? Anthropic only permits subscription
//! OAuth inside Claude Code and claude.ai; third-party apps must not offer
//! Claude.ai login or reuse those tokens. So we never touch credentials: we run
//! the unmodified `claude` binary, which uses whatever login the user already
//! has. Fine for personal use. If Live Spec is ever shipped to other people, they
//! should use the API-key provider instead.

use std::path::PathBuf;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::{ChatRequest, Chunk, OnChunk, Secret, Usage};
use crate::settings::Effort;
use crate::usage::price;
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
        let mut reported: Vec<Usage> = Vec::new();

        while let Some(line) = lines.next_line().await? {
            for event in parse_line(&line, &self.model) {
                match event {
                    CliEvent::Text(t) => {
                        streamed.push_str(&t);
                        on_chunk(Chunk::Text(t));
                    }
                    CliEvent::ToolUse(label) => on_chunk(Chunk::Activity(label)),
                    CliEvent::Result { text, is_error, usage } => {
                        result = Some((text, is_error));
                        reported = usage;
                    }
                }
            }
        }

        let status = child.wait().await?;
        let stderr = stderr_task.await.unwrap_or_default();
        // Before the outcome: a run that failed after the model ran was still billed.
        for usage in reported {
            on_chunk(Chunk::Usage(usage));
        }

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
        return "Claude Code is too old for Live Spec. Update it with `claude update`.".into();
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
    Result { text: Option<String>, is_error: bool, usage: Vec<Usage> },
}

/// One NDJSON line from `--output-format stream-json`. Unknown shapes are
/// ignored so CLI upgrades that add event types don't break us. `model` is
/// the model we asked for; usage without a per-model breakdown is filed under it.
fn parse_line(line: &str, model: &str) -> Vec<CliEvent> {
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
            usage: result_usage(&v, model),
        }],
        _ => vec![],
    }
}

/// What a `result` event says the run used: one `Usage` per model in
/// `modelUsage`, priced by its `costUSD` (their sum is `total_cost_usd`).
/// Without `modelUsage`, the `usage` totals with `total_cost_usd`, under the
/// requested model. A record with no reported cost is priced from
/// `usage::PRICES` (None if the table doesn't know the model). Empty when the
/// event reports neither, and for a record with no tokens at all. Sorted by model.
fn result_usage(v: &Value, requested: &str) -> Vec<Usage> {
    let n = |x: &Value| x.as_u64().unwrap_or(0);
    let mut usage: Vec<Usage> = if let Some(models) = v["modelUsage"].as_object().filter(|m| !m.is_empty()) {
        models
            .iter()
            .map(|(model, u)| Usage {
                model: model.clone(),
                input: n(&u["inputTokens"]),
                cache_write: n(&u["cacheCreationInputTokens"]),
                cache_read: n(&u["cacheReadInputTokens"]),
                output: n(&u["outputTokens"]),
                cost_usd: u["costUSD"].as_f64(),
            })
            .collect()
    } else if v["usage"].is_object() {
        let u = &v["usage"];
        vec![Usage {
            model: requested.to_owned(),
            input: n(&u["input_tokens"]),
            cache_write: n(&u["cache_creation_input_tokens"]),
            cache_read: n(&u["cache_read_input_tokens"]),
            output: n(&u["output_tokens"]),
            cost_usd: v["total_cost_usd"].as_f64(),
        }]
    } else {
        Vec::new()
    };
    // A failed run (bad key, auth failure) can report all zeros: that is not usage.
    usage.retain(|u| [u.input, u.cache_write, u.cache_read, u.output].iter().any(|&n| n > 0));
    for u in &mut usage {
        if u.cost_usd.is_none() {
            let listed = price(&u.model).map(|p| p.cost(u));
            u.cost_usd = listed;
        }
    }
    usage.sort_by(|a, b| a.model.cmp(&b.model));
    usage
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

    /// The `result` event recorded on 2026-10-14 with `--model haiku`, verbatim.
    const FIXTURE: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Hi!","total_cost_usd":0.00046765,"usage":{"input_tokens":2,"cache_creation_input_tokens":2198,"cache_read_input_tokens":535,"output_tokens":45},"modelUsage":{"claude-haiku-5-5":{"inputTokens":2,"outputTokens":45,"cacheReadInputTokens":535,"cacheCreationInputTokens":2198,"costUSD":0.00046765,"canonicalModel":"claude-haiku-5-5","costBasis":"list"}}}"#;

    /// A run that used two models (for example a sonnet run whose sub-tasks used haiku).
    const TWO_MODELS: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok","total_cost_usd":0.01283,"usage":{"input_tokens":12,"cache_creation_input_tokens":4100,"cache_read_input_tokens":900,"output_tokens":610},"modelUsage":{"claude-sonnet-5-5":{"inputTokens":10,"outputTokens":580,"cacheReadInputTokens":900,"cacheCreationInputTokens":4100,"costUSD":0.0124,"costBasis":"list"},"claude-haiku-5-5":{"inputTokens":2,"outputTokens":30,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.00043,"costBasis":"list"}}}"#;

    fn haiku(input: u64, cache_write: u64, cache_read: u64, output: u64, cost: f64) -> Usage {
        Usage { model: "claude-haiku-5-5".into(), input, cache_write, cache_read, output, cost_usd: Some(cost) }
    }

    #[test]
    fn parses_text_delta() {
        let l = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#;
        assert_eq!(parse_line(l, "haiku"), vec![CliEvent::Text("Hi".into())]);
    }

    #[test]
    fn parses_tool_use_and_result() {
        let a = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"x"},{"type":"tool_use","name":"Read","input":{"file_path":"src/main.rs"}}]}}"#;
        assert_eq!(parse_line(a, "haiku"), vec![CliEvent::ToolUse("Read src/main.rs".into())]);
        let r = r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#;
        assert_eq!(
            parse_line(r, "haiku"),
            vec![CliEvent::Result { text: Some("done".into()), is_error: false, usage: vec![] }],
            "a result without usage reports none"
        );
    }

    #[test]
    fn ignores_noise() {
        assert!(parse_line("not json", "haiku").is_empty());
        assert!(parse_line(r#"{"type":"system","subtype":"init"}"#, "haiku").is_empty());
    }

    #[test]
    fn the_recorded_result_costs_exactly_what_the_cli_said() {
        let events = parse_line(FIXTURE, "haiku");
        assert_eq!(
            events,
            vec![CliEvent::Result {
                text: Some("Hi!".into()),
                is_error: false,
                usage: vec![haiku(2, 2198, 535, 45, 0.00046765)],
            }]
        );
        let CliEvent::Result { usage, .. } = &events[0] else { unreachable!() };
        let total: f64 = usage.iter().filter_map(|u| u.cost_usd).sum();
        let cli: Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(Some(total), cli["total_cost_usd"].as_f64(), "the ledger's cost equals total_cost_usd");
    }

    #[test]
    fn a_run_on_two_models_reports_one_usage_per_model() {
        let events = parse_line(TWO_MODELS, "sonnet");
        let [CliEvent::Result { usage, .. }] = &events[..] else { panic!("one result") };
        let sonnet = Usage { model: "claude-sonnet-5-5".into(), input: 10, cache_write: 4100, cache_read: 900, output: 580, cost_usd: Some(0.0124) };
        assert_eq!(usage, &vec![haiku(2, 0, 0, 30, 0.00043), sonnet], "sorted by model, whatever the JSON order");
        let total: f64 = usage.iter().filter_map(|u| u.cost_usd).sum();
        assert!((total - 0.01283).abs() < 1e-12);
    }

    #[test]
    fn without_model_usage_the_totals_are_filed_under_the_requested_model() {
        let r = r#"{"type":"result","is_error":false,"result":"x","total_cost_usd":0.002,"usage":{"input_tokens":5,"cache_creation_input_tokens":10,"cache_read_input_tokens":20,"output_tokens":30}}"#;
        let want = Usage { model: "haiku".into(), input: 5, cache_write: 10, cache_read: 20, output: 30, cost_usd: Some(0.002) };
        assert_eq!(parse_line(r, "haiku"), vec![CliEvent::Result { text: Some("x".into()), is_error: false, usage: vec![want.clone()] }]);
        let no_cost = r.replace(r#""total_cost_usd":0.002,"#, "");
        let events = parse_line(&no_cost, "haiku");
        let [CliEvent::Result { usage, .. }] = &events[..] else { panic!("one result") };
        let cost = usage[0].cost_usd.expect("no cost reported: priced from usage::PRICES");
        assert!((cost - (35.0 * 0.10 + 30.0 * 0.50) / 1e6).abs() < 1e-15, "35 input-rate tokens and 30 output at haiku's list price");
        let events = parse_line(&no_cost, "claude-sonnet-9");
        let [CliEvent::Result { usage, .. }] = &events[..] else { panic!("one result") };
        let unknown = Usage { model: "claude-sonnet-9".into(), cost_usd: None, ..want };
        assert_eq!(usage, &vec![unknown], "a model the table doesn't know stays unpriced");
    }

    /// A stand-in `claude` that ignores its arguments, swallows the prompt and prints `lines`.
    #[cfg(unix)]
    fn fake_cli(dir: &std::path::Path, lines: &[&str], code: i32) -> ClaudeCli {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("claude");
        let script = format!("#!/bin/sh\ncat > /dev/null\ncat <<'JSON'\n{}\nJSON\nexit {code}\n", lines.join("\n"));
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        ClaudeCli::new(path, "haiku", None)
    }

    #[cfg(unix)]
    async fn run(cli: &ClaudeCli) -> (Result<String>, Vec<Chunk>) {
        let mut chunks = Vec::new();
        let result = cli.stream(&request(false), &mut |c: Chunk| chunks.push(c)).await;
        (result, chunks)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_reports_its_usage_after_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi!"}}}"#;
        let (result, chunks) = run(&fake_cli(dir.path(), &[delta, FIXTURE], 0)).await;
        assert_eq!(result.unwrap(), "Hi!");
        assert_eq!(chunks, vec![Chunk::Text("Hi!".into()), Chunk::Usage(haiku(2, 2198, 535, 45, 0.00046765))]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_error_result_still_reports_its_usage() {
        let dir = tempfile::tempdir().unwrap();
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"overloaded","total_cost_usd":0.0002,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}"#;
        let (result, chunks) = run(&fake_cli(dir.path(), &[failed], 1)).await;
        assert_eq!(result.unwrap_err().to_string(), "Claude Code failed: overloaded");
        let billed = Usage { model: "haiku".into(), input: 2000, cache_write: 0, cache_read: 0, output: 0, cost_usd: Some(0.0002) };
        assert_eq!(chunks, vec![Chunk::Usage(billed)], "the input was billed even though the run failed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_run_with_no_tokens_counts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Invalid API key","total_cost_usd":0,"usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0},"modelUsage":{}}"#;
        let (result, chunks) = run(&fake_cli(dir.path(), &[failed], 1)).await;
        assert!(result.is_err());
        assert_eq!(chunks, vec![], "zero tokens is no usage, not an exact $0 run");
    }

    #[test]
    fn a_model_with_zero_tokens_in_model_usage_is_dropped() {
        let r = r#"{"type":"result","is_error":false,"result":"x","modelUsage":{"claude-sonnet-5-5":{"inputTokens":0,"outputTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0},"claude-haiku-5-5":{"inputTokens":2,"outputTokens":3,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.001}}}"#;
        let [CliEvent::Result { usage, .. }] = &parse_line(r, "haiku")[..] else { panic!("one result") };
        assert_eq!(usage, &vec![haiku(2, 0, 0, 3, 0.001)]);
    }

    #[test]
    fn counts_that_would_overflow_a_sum_are_still_usage() {
        let big = u64::MAX;
        let r = format!(
            r#"{{"type":"result","is_error":false,"result":"x","usage":{{"input_tokens":{big},"output_tokens":{big},"cache_read_input_tokens":{big},"cache_creation_input_tokens":{big}}}}}"#
        );
        let [CliEvent::Result { usage, .. }] = &parse_line(&r, "haiku")[..] else { panic!("one result") };
        assert_eq!(usage.len(), 1, "a wrapped sum must not drop real usage");
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
            "Claude Code is too old for Live Spec. Update it with `claude update`."
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
