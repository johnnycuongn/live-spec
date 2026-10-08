# Phase 1: Claude Code Hardening and Jev Locally — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lock the `claude -p` runs down for untrusted repos, then add Jev fact-checking: after the notes stream in, every Fault that names code gets a "Checked in code" verdict (✓ confirmed, ✗ contradicted, unclear, …), using a TypeSafe key read from the repo's `.env.local` in development builds.

**Architecture:**
- **Core:** `livebrainstorm-core` gains a typed Jev HTTP client (`jev`), plain-code evidence location plus a Jev Choice per claim (`jev::check`), a dotenv reader (`env_file`), `JevSettings`, and a Rust mirror of the UI's section parser (`prompt::section_body`).
- **Shell:** the Tauri shell resolves the key (Keychain → environment → `.env.local` in debug builds) and runs the check after `done` inside the same abortable analysis task.
- **UI:** the React notes pane renders the new `checking`, `check` and `notice` events.

**Tech Stack:** Rust 2021 (tokio, reqwest 0.12 with rustls, serde, futures-util, dotenvy 0.15), Tauri v2, React 19 + Tailwind v4, vitest.

**Spec:** `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md`. This plan is its phase 1 ("Hardening, then Jev locally"). Phases 2–5 get their own plans after this one lands, because phase 2's time budget depends on the latency this phase measures.

## Global Constraints

- Jev endpoint: `POST https://api.typesafe.ai/v1/systemone`, header `Authorization: Bearer <key>`, default model `jev-latest`.
- Key lookup order:
  1. Keychain (service `app.livebrainstorm.desktop`, account `typesafe-api-key`).
  2. Env `TYPESAFE_API_KEY`.
  3. Debug builds only: `TYPESAFE_API_KEY` in `<repo>/.env.local`.
- Key handling: never written to the process environment, never sent over IPC, never prefixed `VITE_`.
- `.env*.local` is already gitignored and `.env.example` already exists (both committed). Never print or commit the key.
- Check constants: `CHECK_MIN_CONFIDENCE = 0.8`; at most 6 claims (`MAX_CLAIMS`); evidence window ≤ 1,500 chars; `JEV_CONCURRENCY = 8` (independent of max agents); per-request timeout 5 s.
- Jev retries on 429/529: 2 retries, waiting 250 ms then 750 ms, or `retry-after` if it fits the deadline.
- Choice option order is fixed: `supports`, `contradicts`, `says_nothing`.
- Claude Code runs always pass `--restricted --strict-mcp-config`. Tools come only from `--tools`: `Read,Glob,Grep` for the brief, `""` for live analysis. There is no `--disallowedTools` denylist.
- `livebrainstorm-core` must not depend on Tauri. Testable logic lives in core; `src-tauri` has no tests.
- Do not run `cargo fmt` across the repo (it is hand-wrapped at about 120 columns). `cargo clippy --workspace --all-targets` must stay warning-free.
- UI uses the Tailwind semantic tokens (`text-accent`, `text-fault`, `text-faint`, `text-muted`, `bg-ink`, …) and `src/components/ui.tsx` primitives. Model output is rendered as React text only.
- Node 20.19+ is required for `vite` and `npm run build`. On this machine prefix `PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH"`. `npm test` and `npm run typecheck` work on any Node.
- Work on branch `feat/jev-phase-1`. Every commit message ends with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- UI copy: section title "Checked in code". Verdict labels: "Confirmed", "Contradicted", "Unclear", "Not shown in this code", "Not found in indexed code".

## Review Focus

1. **Short or ubiquitous code names** in a Fault (`id`, `self`, `data`) shouldn't pick an arbitrary file as "evidence". A claim naming only those is not checked. Pinned in Task 3 (`spans_drop_short_generic_names_and_line_numbers`, `names_found_almost_everywhere_are_too_generic`).
2. **`.env.local` spellings** (`export` prefix, quotes, CRLF line endings, comments, empty value, missing file) must yield the key, or cleanly no key, never a crash. Pinned in Task 5 (`reads_common_dotenv_spellings`, `empty_or_missing_means_no_key`).
3. **Typing while checks are still arriving:** rows from the previous draft must never appear under the new notes. Pinned in Task 7 (`a new run drops the previous checks and notice`); the hook also drops events by request id, and the backend aborts the task.
4. **Jev unreachable or key rejected mid-check:** the notes stay, one quiet notice appears, there's no error state, and the summary stops saying "Checking…". Pinned in Task 4 (`a_jev_failure_becomes_one_notice`) and Task 7 (`a notice never turns the notes into an error`, `checkSummary` stopped case).
5. **Dotted names that look like files** (`state.local`, `index.files`) must be searched as code, not as paths. Pinned in Task 3 (`dotted_names_are_code_not_paths`).

---

### Task 1: Harden the Claude Code invocation

**Files:**
- Modify: `crates/livebrainstorm-core/src/llm/claude_cli.rs` (constants, `stream`, `cli_error`, tests)
- Modify: `AGENTS.md` (the `ClaudeCli` bullet under "Providers")

**Interfaces:**
- Consumes: `ChatRequest { system, user, max_tokens, cwd, allow_repo_tools }` (unchanged, `crates/livebrainstorm-core/src/llm/mod.rs`).
- Produces: `fn args(model: &str, req: &ChatRequest) -> Vec<String>` (private to `claude_cli.rs`). Public API unchanged.

- [ ] **Step 0: Confirm the branch** (it was created with this plan)

```bash
git switch feat/jev-phase-1 && git branch --show-current
```
Expected: `feat/jev-phase-1`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `claude_cli.rs`)

```rust
    fn request(allow_repo_tools: bool) -> ChatRequest {
        ChatRequest { system: "sys".into(), user: "u".into(), max_tokens: 900, cwd: None, allow_repo_tools }
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str())
    }

    #[test]
    fn analysis_runs_with_no_tools_and_no_workspace_config() {
        let a = args("haiku", &request(false));
        assert_eq!(after(&a, "--tools"), Some(""));
        assert_eq!(after(&a, "--system-prompt"), Some("sys"));
        assert_eq!(after(&a, "--max-turns"), Some("1"));
        assert!(a.contains(&"--restricted".into()) && a.contains(&"--strict-mcp-config".into()));
        assert!(!a.iter().any(|x| x.contains("disallowed") || x.contains("Bash")));
    }

    #[test]
    fn brief_explorers_only_get_read_only_tools() {
        let a = args("sonnet", &request(true));
        assert_eq!(after(&a, "--tools"), Some("Read,Glob,Grep"));
        assert_eq!(after(&a, "--allowedTools"), Some("Read,Glob,Grep"));
        assert_eq!(after(&a, "--append-system-prompt"), Some("sys"));
        assert_eq!(after(&a, "--model"), Some("sonnet"));
        assert!(a.contains(&"--restricted".into()) && a.contains(&"--strict-mcp-config".into()));
    }

    #[test]
    fn an_old_cli_gets_an_update_hint() {
        assert!(cli_error(None, "error: unknown option '--restricted'").contains("claude update"));
        assert_eq!(cli_error(Some("boom"), ""), "Claude Code failed: boom");
    }
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p livebrainstorm-core claude_cli`
Expected: compile error, `cannot find function args in this scope`.

- [ ] **Step 3: Implement**

Replace the two tool constants at the top of `claude_cli.rs`:

```rust
/// The only tools a brief explorer gets. An allowlist (`--tools`), not a
/// denylist, so tools added to future Claude Code versions stay unavailable.
const READ_ONLY_TOOLS: &str = "Read,Glob,Grep";
```

Add this function above `impl ClaudeCli`:

```rust
/// Flags for one `claude -p` run. Opened projects may be untrusted clones,
/// so `--restricted` ignores settings files (no project hooks), confines file
/// tools to the project and refuses bypassPermissions, and
/// `--strict-mcp-config` without `--mcp-config` starts no MCP servers.
fn args(model: &str, req: &ChatRequest) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
        "--model", model, "--restricted", "--strict-mcp-config",
    ]
    .map(String::from)
    .to_vec();
    if req.allow_repo_tools {
        // Keep Claude Code's own system prompt: it knows how to explore repos.
        a.extend(["--append-system-prompt", req.system.as_str(), "--max-turns", "40"].map(String::from));
        a.extend(["--tools", READ_ONLY_TOOLS, "--allowedTools", READ_ONLY_TOOLS].map(String::from));
    } else {
        // Live analysis: replace the system prompt, single turn, no tools at all.
        // Smaller prompt = faster first token while the user types.
        a.extend(["--system-prompt", req.system.as_str(), "--max-turns", "1", "--tools", ""].map(String::from));
    }
    a
}
```

In `ClaudeCli::stream`, replace everything from `cmd.arg("-p")` through the closing brace of the `if req.allow_repo_tools { … } else { … }` block with:

```rust
        cmd.args(args(&self.model, req));
```

Replace `cli_error` with:

```rust
fn cli_error(result_text: Option<&str>, stderr: &str) -> String {
    if stderr.contains("unknown option") {
        return "Claude Code is too old for LiveBrainstorm's restricted mode. Update it with `claude update`.".into();
    }
    let detail = result_text.filter(|t| !t.trim().is_empty()).unwrap_or(stderr).trim();
    let detail = if detail.is_empty() { "no output" } else { detail };
    format!("Claude Code failed: {}", detail.chars().take(500).collect::<String>())
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test -p livebrainstorm-core claude_cli && cargo clippy --workspace --all-targets`
Expected: 6 `claude_cli` tests pass (3 existing and 3 new); clippy prints no warnings.

- [ ] **Step 5: Confirm the real CLI accepts the flags and still uses the subscription login**

Run:
```bash
echo "Reply with the single word ok." | claude -p --output-format stream-json --verbose --include-partial-messages --model haiku --restricted --strict-mcp-config --system-prompt "You answer tersely." --max-turns 1 --tools "" | tail -1 | cut -c1-200
```
Expected: a JSON line starting `{"type":"result"` that contains `"is_error":false`. If it instead reports an auth or unknown-flag error, stop and report it; don't weaken the flags.

- [ ] **Step 6: Update AGENTS.md**

In `AGENTS.md`, under "### Providers", replace the `**`ClaudeCli`**` bullet's sentence that begins "Live analysis *replaces* the system prompt" through "…so Claude Code keeps its own repo-exploration prompt." with:

```markdown
Every run passes `--restricted` (ignores settings files, so an opened repo's hooks never run; confines file tools to the project) and `--strict-mcp-config` (no MCP servers). Tools come from an allowlist via `--tools`: none (`""`) for live analysis, which also *replaces* the system prompt (`--system-prompt`, `--max-turns 1`); `Read,Glob,Grep` for the brief, which *appends* (`--append-system-prompt`) so Claude Code keeps its own repo-exploration prompt. Never reintroduce a `--disallowedTools` denylist: new tools would slip past it.
```

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/llm/claude_cli.rs AGENTS.md
git commit -m "fix(claude-cli): restricted mode and a tool allowlist for untrusted repos" -m "Opened projects can be untrusted clones and print mode skips workspace trust, so project hooks/MCP servers could run and the --disallowedTools denylist missed newer tools. Use --restricted, --strict-mcp-config and --tools instead." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Jev client

**Files:**
- Create: `crates/livebrainstorm-core/src/jev/mod.rs`
- Create: `crates/livebrainstorm-core/src/jev/test_server.rs`
- Create: `crates/livebrainstorm-core/src/jev/check.rs` (empty module for now: `//! Filled in by the next task.`)
- Modify: `crates/livebrainstorm-core/src/lib.rs` (add `pub mod jev;`)
- Modify: `crates/livebrainstorm-core/Cargo.toml` (dev-dependency for tokio `net`)

**Interfaces:**
- Produces (used by Tasks 3, 4, 5, 6, 8):
  - `pub const DEFAULT_MODEL: &str = "jev-latest";`
  - `pub enum Question { Noul { instructions: Value, criteria: Option<NoulCriteria> }, Choice { instructions: Value, criteria: Options } }`
  - `pub struct NoulCriteria { pub yes: Value, pub no: Value }` (serialized as `"true"`/`"false"`)
  - `pub struct Options(pub Vec<(String, Value)>)` (an order-preserving JSON object)
  - `pub enum Answer { Noul { noul: f64 }, Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 } }`
  - `pub struct Response { pub model: String, pub answers: BTreeMap<String, Answer> }`, with `fn noul(&self, id) -> Option<f64>` and `fn choice(&self, id) -> Option<(&str, f64)>`
  - `pub struct Jev { pub api_key: String, pub model: String, pub http: reqwest::Client, pub base_url: String }`, with `Jev::new(api_key, model, http)` and `async fn ask(&self, state: &Value, questions: &BTreeMap<String, Question>, timeout: Duration) -> Result<Response>`
  - Test-only: `jev::test_server::{serve, Canned}`, where `serve(Vec<Canned>) -> (String /*base url*/, Arc<AtomicUsize> /*hits*/)` and `Canned::new(status: u16, body: &str)`

- [ ] **Step 1: Add the tokio `net` feature for tests**

In `crates/livebrainstorm-core/Cargo.toml`, under `[dev-dependencies]`, add:

```toml
tokio = { workspace = true, features = ["net"] }
```

- [ ] **Step 2: Write the test server** (`crates/livebrainstorm-core/src/jev/test_server.rs`)

```rust
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
```

- [ ] **Step 3: Write the failing tests** (the bottom of the new `crates/livebrainstorm-core/src/jev/mod.rs`; Step 5 adds the code above them)

```rust
#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use serde_json::json;

    use super::test_server::{serve, Canned};
    use super::*;

    const NOUL_OK: &str = r#"{"model":"jev-1.13.0","answers":{"is_urgent":{"type":"noul","noul":0.95}},"usage":{"input_tokens":307,"output_tokens":20}}"#;

    fn urgent() -> BTreeMap<String, Question> {
        BTreeMap::from([(
            "is_urgent".to_owned(),
            Question::Noul {
                instructions: json!("Does this convey urgency?"),
                criteria: Some(NoulCriteria { yes: json!("Explicitly time-sensitive"), no: json!("No urgency expressed") }),
            },
        )])
    }

    fn client(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    #[test]
    fn noul_request_matches_the_api_reference() {
        let state = json!("Help! My payouts have been failing for 3 days.");
        let questions = urgent();
        let request = Request { state: &state, model: "jev-latest", questions: &questions };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "state": "Help! My payouts have been failing for 3 days.",
                "model": "jev-latest",
                "questions": {"is_urgent": {
                    "type": "noul",
                    "instructions": "Does this convey urgency?",
                    "criteria": {"true": "Explicitly time-sensitive", "false": "No urgency expressed"}
                }}
            })
        );
    }

    #[test]
    fn choice_options_keep_their_order() {
        let q = Question::Choice {
            instructions: json!("Which team should handle this?"),
            criteria: Options(vec![
                ("sales".into(), json!("Pricing")),
                ("billing".into(), json!("Payments")),
                ("technical".into(), Value::Null),
            ]),
        };
        let text = serde_json::to_string(&q).unwrap();
        let pos = |s: &str| text.find(s).unwrap();
        assert!(pos("\"sales\"") < pos("\"billing\"") && pos("\"billing\"") < pos("\"technical\""), "{text}");
        assert_eq!(serde_json::to_value(&q).unwrap()["criteria"]["technical"], Value::Null);
    }

    #[test]
    fn parses_reference_answers() {
        let noul: Response = serde_json::from_str(NOUL_OK).unwrap();
        assert_eq!(noul.noul("is_urgent"), Some(0.95));
        let choice: Response = serde_json::from_str(
            r#"{"model":"jev-1.13.0","answers":{"department":{"type":"choice","choice":"billing","probabilities":{"billing":0.88,"technical":0.12,"sales":0.0},"confidence":0.81}},"usage":{"input_tokens":318,"output_tokens":34}}"#,
        )
        .unwrap();
        assert_eq!(choice.choice("department"), Some(("billing", 0.81)));
        assert_eq!(choice.noul("department"), None);
    }

    #[tokio::test]
    async fn retries_overload_then_answers() {
        let (url, hits) = serve(vec![Canned::new(529, "{}"), Canned::new(200, NOUL_OK)]).await;
        let r = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap();
        assert_eq!(r.noul("is_urgent"), Some(0.95));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_two_retries() {
        let (url, hits) = serve(vec![Canned::new(429, "slow down"); 3]).await;
        let err = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap_err();
        assert!(err.to_string().contains("Jev returned 429"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_rejected_key_is_a_precondition() {
        let (url, _) = serve(vec![Canned::new(401, "{}")]).await;
        let err = client(url).ask(&json!("x"), &urgent(), Duration::from_secs(5)).await.unwrap_err();
        assert!(matches!(err, Error::Precondition(_)), "{err}");
    }
}
```

- [ ] **Step 4: Run the tests and confirm they fail**

Add `pub mod jev;` to `crates/livebrainstorm-core/src/lib.rs` (after `pub mod error;`), and create `check.rs` containing only `//! Filled in by the next task.`
Run: `cargo test -p livebrainstorm-core jev::tests`
Expected: compile errors (`Question`, `Jev`, `Request` not found).

- [ ] **Step 5: Implement the client** (the top of `crates/livebrainstorm-core/src/jev/mod.rs`, above the tests)

```rust
//! TypeSafe Jev: fast, typed judgments (a yes-probability, or one option out
//! of a set) instead of generated text. One request carries one `state` and
//! any number of independent questions about it.
//! API reference: https://docs.typesafe.ai/api.md

pub mod check;
#[cfg(test)]
pub(crate) mod test_server;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::{Error, Result};

pub const DEFAULT_MODEL: &str = "jev-latest";
const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
/// Waits before the first and second retry of a 429/529.
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(250), Duration::from_millis(750)];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Noul {
        instructions: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    Choice { instructions: Value, criteria: Options },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: Value,
    #[serde(rename = "false")]
    pub no: Value,
}

/// Choice options, serialized as a JSON object in the order given. Jev 1.13
/// can lean towards the first option, so the order is a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Options(pub Vec<(String, Value)>);

impl Serialize for Options {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (option, description) in &self.0 {
            map.serialize_entry(option, description)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul { noul: f64 },
    Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    /// The versioned model that answered (e.g. `jev-1.13.0`), worth logging.
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
}

impl Response {
    pub fn noul(&self, id: &str) -> Option<f64> {
        match self.answers.get(id)? {
            Answer::Noul { noul } => Some(*noul),
            Answer::Choice { .. } => None,
        }
    }

    /// The chosen option and its confidence.
    pub fn choice(&self, id: &str) -> Option<(&str, f64)> {
        match self.answers.get(id)? {
            Answer::Choice { choice, confidence, .. } => Some((choice.as_str(), *confidence)),
            Answer::Noul { .. } => None,
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    state: &'a Value,
    model: &'a str,
    questions: &'a BTreeMap<String, Question>,
}

#[derive(Debug, Clone)]
pub struct Jev {
    pub api_key: String,
    pub model: String,
    pub http: reqwest::Client,
    /// `https://api.typesafe.ai`; tests point it at a local server.
    pub base_url: String,
}

impl Jev {
    pub fn new(api_key: String, model: String, http: reqwest::Client) -> Self {
        Self { api_key, model, http, base_url: DEFAULT_BASE_URL.to_owned() }
    }

    /// Asks every question about `state` in one request. Retries 429/529
    /// twice and never runs past `timeout` in total.
    pub async fn ask(&self, state: &Value, questions: &BTreeMap<String, Question>, timeout: Duration) -> Result<Response> {
        let deadline = Instant::now() + timeout;
        let url = format!("{}/v1/systemone", self.base_url.trim_end_matches('/'));
        let body = Request { state, model: &self.model, questions };
        let mut retries = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timed_out());
            }
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .timeout(remaining)
                .send()
                .await
                .map_err(|e| if e.is_timeout() { timed_out() } else { Error::Http(e) })?;
            if resp.status().is_success() {
                return Ok(resp.json::<Response>().await?);
            }
            let status = resp.status().as_u16();
            if status == 401 {
                return Err(Error::Precondition("TypeSafe key rejected. Check .env.local or Settings → Jev.".into()));
            }
            if (status == 429 || status == 529) && retries < RETRY_DELAYS.len() {
                let wait = retry_after(&resp).unwrap_or(RETRY_DELAYS[retries]);
                if Instant::now() + wait < deadline {
                    tokio::time::sleep(wait).await;
                    retries += 1;
                    continue;
                }
            }
            let detail: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
            return Err(Error::Provider(format!("Jev returned {status}: {detail}")));
        }
    }
}

fn timed_out() -> Error {
    Error::Provider("Jev timed out".into())
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs: f64 = resp.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test -p livebrainstorm-core jev::tests && cargo clippy --workspace --all-targets`
Expected: 6 tests pass; no clippy warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/Cargo.toml crates/livebrainstorm-core/src/lib.rs crates/livebrainstorm-core/src/jev
git commit -m "feat(jev): typed client for the TypeSafe System One API" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Locate evidence and map verdicts

**Files:**
- Modify: `crates/livebrainstorm-core/src/project/retrieve.rs` (split helpers out of `retrieve`/`snippet`, behaviour unchanged)
- Modify: `crates/livebrainstorm-core/src/project/terms.rs` (add `is_stopword`)
- Modify: `crates/livebrainstorm-core/src/project/mod.rs` (`mod terms;` → `pub(crate) mod terms;`)
- Modify: `crates/livebrainstorm-core/src/jev/check.rs` (replace the placeholder)

**Interfaces:**
- Consumes: `Question`, `Options` (Task 2); `ProjectIndex`, `Snippet`, and `scan` with its test `fixture()` (existing).
- Produces:
  - In `project::retrieve` (crate-visible): `file_score(index: &ProjectIndex, i: usize, weights: &HashMap<String, f64>) -> f64`, `query_weights(query: &str) -> HashMap<String, f64>`, `best_line(lines: &[&str], weights: &HashMap<String, f64>) -> usize`, and `window(path: &str, lines: &[&str], center: usize, max_chars: usize) -> Snippet`.
  - In `project::terms`: `pub(crate) fn is_stopword(word: &str) -> bool` (expects lowercase input).
  - In `jev::check`:
    - constants `MAX_CLAIMS`, `CHECK_MIN_CONFIDENCE`, `JEV_CONCURRENCY`
    - `enum Verdict { Confirmed, Contradicted, Unclear, NotShown, NotFound }` (serde camelCase)
    - `struct CheckResult { claim: String, verdict: Verdict, confidence: Option<f64>, path: Option<String>, line: Option<usize> }`
    - `enum CheckEvent { Checking { count, total }, Result(CheckResult), Notice(String) }`
    - `enum Evidence { Uncheckable, NotFound, Found(Snippet, usize) }`
    - functions `extract_claims(&str) -> (Vec<String>, usize)`, `code_spans(&str) -> Vec<String>`, `locate_evidence(&ProjectIndex, &str) -> Evidence`, `question() -> Question`, `state(&str, &Snippet) -> Value`, and `verdict(&str, f64) -> Verdict`

- [ ] **Step 1: Refactor `retrieve.rs`, keeping its tests green**

In `crates/livebrainstorm-core/src/project/retrieve.rs`, replace `retrieve`, `query_weights` and `snippet` (everything from `pub fn retrieve` down to the end of `fn snippet`) with:

```rust
pub fn retrieve(index: &ProjectIndex, query: &str, max_files: usize, budget_chars: usize) -> Vec<Snippet> {
    let weights = query_weights(query);
    if weights.is_empty() || index.files.is_empty() || max_files == 0 {
        return vec![];
    }
    let mut scored: Vec<(f64, usize)> = (0..index.files.len())
        .filter_map(|i| {
            let score = file_score(index, i, &weights);
            (score > 0.0).then_some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    let per_file = budget_chars / max_files;
    scored
        .into_iter()
        .take(max_files)
        .map(|(_, i)| snippet(&index.files[i].rel, &index.files[i].content, &weights, per_file))
        .collect()
}

/// TF-IDF relevance of file `i` to the weighted query terms; 0 means unrelated.
pub(crate) fn file_score(index: &ProjectIndex, i: usize, weights: &HashMap<String, f64>) -> f64 {
    let n = index.files.len() as f64;
    let f = &index.files[i];
    let path_lower = f.rel.to_lowercase();
    weights
        .iter()
        .map(|(t, w)| {
            let idf = (1.0 + n / f64::from(*index.doc_freq.get(t).unwrap_or(&0)).max(1.0)).ln();
            let tf = f.terms.get(t).copied().unwrap_or(0);
            let body = if tf > 0 { 1.0 + f64::from(tf).ln() } else { 0.0 };
            let path = if path_lower.contains(t.as_str()) { 2.0 } else { 0.0 };
            w * idf * (body + path)
        })
        .sum()
}

pub(crate) fn query_weights(query: &str) -> HashMap<String, f64> {
    let split = query.len().saturating_sub(RECENT_CHARS);
    let split = (split..=query.len()).find(|i| query.is_char_boundary(*i)).unwrap_or(query.len());
    let (older, recent) = query.split_at(split);
    let mut w: HashMap<String, f64> = HashMap::new();
    for t in terms::tokens(older) {
        w.entry(t).or_insert(1.0);
    }
    for t in terms::tokens(recent) {
        w.insert(t, RECENT_BOOST);
    }
    w
}

/// The window around the line with the most query hits.
fn snippet(path: &str, content: &str, weights: &HashMap<String, f64>, max_chars: usize) -> Snippet {
    let lines: Vec<&str> = content.lines().collect();
    window(path, &lines, best_line(&lines, weights), max_chars)
}

/// Index of the line with the most (weighted) query hits; the first line if none.
pub(crate) fn best_line(lines: &[&str], weights: &HashMap<String, f64>) -> usize {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let lower = l.to_lowercase();
            let hits: f64 = weights.iter().filter(|(t, _)| lower.contains(t.as_str())).map(|(_, w)| w).sum();
            (hits, i)
        })
        .fold((0.0, 0), |acc, x| if x.0 > acc.0 { x } else { acc })
        .1
}

/// Lines around `center` (8 before, 16 after), cut to `max_chars`.
pub(crate) fn window(path: &str, lines: &[&str], center: usize, max_chars: usize) -> Snippet {
    let start = center.saturating_sub(WINDOW_BEFORE);
    let end = (center + WINDOW_AFTER).min(lines.len());
    let mut text = lines[start.min(end)..end].join("\n");
    if text.len() > max_chars {
        let cut = (0..=max_chars).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        text.truncate(cut);
    }
    Snippet { path: path.to_owned(), start_line: start + 1, text }
}
```

In `crates/livebrainstorm-core/src/project/mod.rs`, change `mod terms;` to `pub(crate) mod terms;`. In `crates/livebrainstorm-core/src/project/terms.rs`, add below `fn keep`:

```rust
/// Common English and keyword noise; `word` must be lowercase.
pub(crate) fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}
```

Run: `cargo test -p livebrainstorm-core project::`
Expected: all existing `project::` tests pass (retrieve, scan, digest, terms) with no behaviour change.

- [ ] **Step 2: Write the failing tests** (replace the contents of `crates/livebrainstorm-core/src/jev/check.rs` with this test module; Step 4 adds the code above it)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::scan;
    use crate::project::scan::tests::fixture;

    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        dir
    }

    #[test]
    fn extracts_bullets_and_skips_nothing_so_far() {
        let (claims, total) = extract_claims("- `a` is wrong\n- Nothing so far.\n* second\n1. third\nprose");
        assert_eq!(claims, ["`a` is wrong", "second", "third"]);
        assert_eq!(total, 3);
    }

    #[test]
    fn spans_drop_short_generic_names_and_line_numbers() {
        assert_eq!(code_spans("`createInvoice` uses `id`, `self` and `src/cart/cart.ts:12`"), ["createInvoice", "src/cart/cart.ts"]);
        assert_eq!(code_spans("`Error::Provider` is returned"), ["Error::Provider"]);
    }

    #[test]
    fn finds_the_named_function() {
        let idx = scan(fixture().path()).unwrap();
        match locate_evidence(&idx, "`createInvoice` never applies discounts") {
            Evidence::Found(code, line) => {
                assert_eq!(code.path, "src/billing/invoice.ts");
                assert_eq!(line, 1);
                assert!(code.text.contains("createInvoice"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn finds_a_named_file() {
        let idx = scan(fixture().path()).unwrap();
        let found = locate_evidence(&idx, "`src/cart/cart.ts:3` has no removeItem");
        assert!(matches!(found, Evidence::Found(ref code, _) if code.path == "src/cart/cart.ts"), "{found:?}");
    }

    #[test]
    fn unknown_names_are_not_found_and_prose_is_uncheckable() {
        let idx = scan(fixture().path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`applyCoupon` is called twice"), Evidence::NotFound);
        assert_eq!(locate_evidence(&idx, "Discounts are never applied"), Evidence::Uncheckable);
    }

    #[test]
    fn names_found_almost_everywhere_are_too_generic() {
        let paths: Vec<String> = (0..8).map(|i| format!("src/f{i}.ts")).collect();
        let files: Vec<(&str, &str)> = paths.iter().map(|p| (p.as_str(), "const data = load();\n")).collect();
        let idx = scan(repo(&files).path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`data` is never validated"), Evidence::Uncheckable);
    }

    #[test]
    fn dotted_names_are_code_not_paths() {
        let dir = repo(&[("src/state.rs", "let guard = state.local.lock();\n"), ("src/other.rs", "fn main() {}\n")]);
        let idx = scan(dir.path()).unwrap();
        let found = locate_evidence(&idx, "`state.local` is held across an await");
        assert!(matches!(found, Evidence::Found(ref code, 1) if code.path == "src/state.rs"), "{found:?}");
    }

    #[test]
    fn maps_answers_to_verdicts() {
        assert_eq!(verdict("supports", 0.93), Verdict::Confirmed);
        assert_eq!(verdict("supports", 0.6), Verdict::Unclear);
        assert_eq!(verdict("contradicts", 0.85), Verdict::Contradicted);
        assert_eq!(verdict("says_nothing", 0.99), Verdict::NotShown);
        assert_eq!(verdict("unexpected", 0.99), Verdict::Unclear);
    }

    #[test]
    fn question_lists_supports_first() {
        let text = serde_json::to_string(&question()).unwrap();
        let pos = |s: &str| text.find(s).unwrap();
        assert!(pos("\"supports\"") < pos("\"contradicts\"") && pos("\"contradicts\"") < pos("\"says_nothing\""));
    }
}
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test -p livebrainstorm-core jev::check`
Expected: compile errors (`extract_claims`, `locate_evidence`, … not found).

- [ ] **Step 4: Implement** (the top of `crates/livebrainstorm-core/src/jev/check.rs`)

```rust
//! "Checked in code": verifies the Faults the notes raise against the
//! project's own code, after the notes have streamed. Plain code finds the
//! evidence; Jev only judges whether it supports the claim (adapted from
//! TypeSafe's citation-check cookbook). Verdicts are signals, not proof:
//! Jev reads literally and struggles with behaviour spread over many files.

use serde::Serialize;
use serde_json::{json, Value};

use super::{Options, Question};
use crate::project::retrieve::{best_line, file_score, query_weights, window};
use crate::project::{terms, ProjectIndex, Snippet};

pub const MAX_CLAIMS: usize = 6;
pub const CHECK_MIN_CONFIDENCE: f64 = 0.8;
/// Jev requests in flight at once; independent of the max-agents setting.
pub const JEV_CONCURRENCY: usize = 8;
const EVIDENCE_CHARS: usize = 1_500;
const MIN_SPAN_CHARS: usize = 3;
/// Extensions that make a backticked span a file path: `cart.ts` is a file,
/// `state.local` is code.
const SOURCE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "json", "toml", "md", "css", "html", "py", "go", "java", "kt",
    "swift", "c", "h", "cpp", "rb", "php", "yml", "yaml", "sh", "sql",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Confirmed,
    Contradicted,
    Unclear,
    NotShown,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub claim: String,
    pub verdict: Verdict,
    pub confidence: Option<f64>,
    pub path: Option<String>,
    /// 1-based line the evidence centres on.
    pub line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CheckEvent {
    /// `count` of the `total` Faults name code that can be checked.
    Checking { count: usize, total: usize },
    Result(CheckResult),
    /// Jev failed; the remaining claims are dropped. At most one per run.
    Notice(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
    /// The claim names no code, or only names too generic to locate.
    Uncheckable,
    /// It names code that isn't in the indexed files.
    NotFound,
    /// The window to judge, and the 1-based line it centres on.
    Found(Snippet, usize),
}

/// The Faults bullets (up to MAX_CLAIMS) and how many bullets there were,
/// skipping the prompt's "Nothing so far."
pub fn extract_claims(faults: &str) -> (Vec<String>, usize) {
    let bullets: Vec<String> = faults
        .lines()
        .filter_map(bullet_text)
        .filter(|t| !t.to_lowercase().starts_with("nothing so far"))
        .map(str::to_owned)
        .collect();
    let total = bullets.len();
    (bullets.into_iter().take(MAX_CLAIMS).collect(), total)
}

fn bullet_text(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = ["- ", "* ", "• "].iter().find_map(|m| t.strip_prefix(m)).or_else(|| {
        let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        (digits > 0).then(|| &t[digits..]).and_then(|r| r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")))
    })?;
    Some(rest.trim()).filter(|r| !r.is_empty())
}

/// Backticked names and paths in a claim, minus ones too short or generic
/// to locate. A trailing `:42` line number is dropped.
pub fn code_spans(claim: &str) -> Vec<String> {
    claim
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|s| strip_line_suffix(s.trim()))
        .filter(|s| s.chars().count() >= MIN_SPAN_CHARS && !terms::is_stopword(&s.to_lowercase()))
        .map(str::to_owned)
        .collect()
}

fn strip_line_suffix(s: &str) -> &str {
    match s.rsplit_once(':') {
        Some((head, line)) if !head.is_empty() && !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => s,
    }
}

fn is_path(span: &str) -> bool {
    span.contains('/') || span.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && SOURCE_EXTS.contains(&ext))
}

/// Finds the code a claim refers to, with no model involved.
pub fn locate_evidence(index: &ProjectIndex, claim: &str) -> Evidence {
    // A name found in more files than this is too generic to point anywhere.
    let generic_above = (index.files.len() / 4).max(3);
    let mut spans: Vec<(String, Vec<usize>)> = Vec::new();
    for span in code_spans(claim) {
        let path = is_path(&span);
        let hits: Vec<usize> = (0..index.files.len())
            .filter(|&i| {
                let f = &index.files[i];
                if path { f.rel.ends_with(span.as_str()) } else { f.content.contains(span.as_str()) }
            })
            .collect();
        if path || hits.len() <= generic_above {
            spans.push((span, hits));
        }
    }
    if spans.is_empty() {
        return Evidence::Uncheckable;
    }
    let mut candidates: Vec<usize> = spans.iter().flat_map(|(_, hits)| hits.iter().copied()).collect();
    candidates.sort_unstable();
    candidates.dedup();

    // Best TF-IDF match for the claim; ties keep path order.
    let weights = query_weights(claim);
    let mut best: Option<(f64, usize)> = None;
    for i in candidates {
        let score = file_score(index, i, &weights);
        if best.is_none_or(|(top, _)| score > top) {
            best = Some((score, i));
        }
    }
    let Some((_, i)) = best else { return Evidence::NotFound };

    let file = &index.files[i];
    let lines: Vec<&str> = file.content.lines().collect();
    let names: Vec<&str> = spans.iter().filter(|(s, _)| !is_path(s)).map(|(s, _)| s.as_str()).collect();
    let (hits, line) = lines
        .iter()
        .enumerate()
        .map(|(n, l)| (names.iter().filter(|s| l.contains(**s)).count(), n))
        .fold((0, 0), |acc, x| if x.0 > acc.0 { x } else { acc });
    let center = if hits > 0 { line } else { best_line(&lines, &weights) };
    Evidence::Found(window(&file.rel, &lines, center, EVIDENCE_CHARS), center + 1)
}

/// The question, worded as in TypeSafe's citation-check cookbook.
pub fn question() -> Question {
    Question::Choice {
        instructions: json!("How does `code` relate to `claim`?"),
        criteria: Options(vec![
            ("supports".into(), json!("The code shows the claim is true, or directly implies it.")),
            ("contradicts".into(), json!("The code shows the claim is false, or directly implies the opposite.")),
            ("says_nothing".into(), json!("The code does not address the claim either way.")),
        ]),
    }
}

pub fn state(claim: &str, code: &Snippet) -> Value {
    json!({ "claim": claim, "code": { "path": code.path, "start_line": code.start_line, "text": code.text } })
}

pub fn verdict(choice: &str, confidence: f64) -> Verdict {
    match choice {
        "says_nothing" => Verdict::NotShown,
        "supports" if confidence >= CHECK_MIN_CONFIDENCE => Verdict::Confirmed,
        "contradicts" if confidence >= CHECK_MIN_CONFIDENCE => Verdict::Contradicted,
        _ => Verdict::Unclear,
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 9 new `jev::check` tests and the unchanged `project::` tests; no clippy warnings. `JEV_CONCURRENCY` is unused until Task 4, but it's `pub`, so no warning.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/project crates/livebrainstorm-core/src/jev/check.rs
git commit -m "feat(jev): locate evidence for Faults and map Jev answers to verdicts" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Run the check

**Files:**
- Modify: `crates/livebrainstorm-core/src/jev/check.rs` (add `run` and its tests)

**Interfaces:**
- Consumes: `Jev::ask`, `Response::choice`, `DEFAULT_MODEL`, `test_server::{serve, Canned}` (Task 2); everything in `check.rs` (Task 3).
- Produces: `pub async fn run(jev: &Jev, index: &ProjectIndex, faults: &str, on_event: impl FnMut(CheckEvent))`. It emits nothing when no Fault names code. Otherwise it emits `Checking` first, then one `Result` per checkable claim (`NotFound` ones immediately), or stops after a single `Notice` if Jev fails.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `check.rs`)

```rust
    use crate::jev::test_server::{serve, Canned};
    use crate::jev::{Jev, DEFAULT_MODEL};

    const SUPPORTS: &str = r#"{"model":"jev-1.13.0","answers":{"relation":{"type":"choice","choice":"supports","probabilities":{"supports":0.95,"contradicts":0.03,"says_nothing":0.02},"confidence":0.93}},"usage":{"input_tokens":300,"output_tokens":20}}"#;

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    async fn events(jev: &Jev, faults: &str) -> Vec<CheckEvent> {
        let idx = scan(fixture().path()).unwrap();
        let mut out = Vec::new();
        run(jev, &idx, faults, |e| out.push(e)).await;
        out
    }

    #[tokio::test]
    async fn reports_verdicts_as_they_arrive() {
        let (url, _) = serve(vec![Canned::new(200, SUPPORTS)]).await;
        let faults = "- `createInvoice` never applies discounts\n- `applyCoupon` is called twice\n- Totals look fine";
        assert_eq!(
            events(&judge(url), faults).await,
            vec![
                CheckEvent::Checking { count: 2, total: 3 },
                CheckEvent::Result(CheckResult {
                    claim: "`applyCoupon` is called twice".into(),
                    verdict: Verdict::NotFound,
                    confidence: None,
                    path: None,
                    line: None,
                }),
                CheckEvent::Result(CheckResult {
                    claim: "`createInvoice` never applies discounts".into(),
                    verdict: Verdict::Confirmed,
                    confidence: Some(0.93),
                    path: Some("src/billing/invoice.ts".into()),
                    line: Some(1),
                }),
            ]
        );
    }

    #[tokio::test]
    async fn a_jev_failure_becomes_one_notice() {
        let (url, _) = serve(vec![Canned::new(401, "{}")]).await;
        let got = events(&judge(url), "- `createInvoice` never applies discounts").await;
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], CheckEvent::Checking { count: 1, total: 1 });
        assert!(matches!(&got[1], CheckEvent::Notice(m) if m.contains("TypeSafe key rejected")), "{got:?}");
    }

    #[tokio::test]
    async fn nothing_to_check_emits_nothing() {
        let (url, hits) = serve(vec![]).await;
        assert!(events(&judge(url.clone()), "- Nothing so far.").await.is_empty());
        assert!(events(&judge(url), "- Totals look fine").await.is_empty());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p livebrainstorm-core jev::check`
Expected: compile error, `cannot find function run`.

- [ ] **Step 3: Implement**

Add these imports at the top of `check.rs` (merge with the existing `use` lines):

```rust
use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};

use super::Jev;
```

Add the constant next to the others:

```rust
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
```

Add the function after `verdict`:

```rust
/// Checks each Fault and reports through `on_event` as verdicts arrive.
/// Emits nothing when no Fault names code; otherwise `Checking` first, then
/// one `Result` per checkable claim, or a single `Notice` if Jev fails.
pub async fn run(jev: &Jev, index: &ProjectIndex, faults: &str, mut on_event: impl FnMut(CheckEvent)) {
    let (claims, total) = extract_claims(faults);
    let located: Vec<(String, Evidence)> = claims
        .into_iter()
        .map(|claim| {
            let evidence = locate_evidence(index, &claim);
            (claim, evidence)
        })
        .filter(|(_, evidence)| *evidence != Evidence::Uncheckable)
        .collect();
    if located.is_empty() {
        return;
    }
    on_event(CheckEvent::Checking { count: located.len(), total });

    let mut to_judge = Vec::new();
    for (claim, evidence) in located {
        match evidence {
            Evidence::Found(code, line) => to_judge.push((claim, code, line)),
            _ => on_event(CheckEvent::Result(CheckResult {
                claim,
                verdict: Verdict::NotFound,
                confidence: None,
                path: None,
                line: None,
            })),
        }
    }

    let questions = BTreeMap::from([("relation".to_owned(), question())]);
    let questions = &questions;
    let mut answers = std::pin::pin!(stream::iter(to_judge)
        .map(|(claim, code, line)| async move {
            let answer = jev.ask(&state(&claim, &code), questions, REQUEST_TIMEOUT).await;
            (claim, code, line, answer)
        })
        .buffer_unordered(JEV_CONCURRENCY));
    while let Some((claim, code, line, answer)) = answers.next().await {
        let picked = match answer {
            Ok(response) => response.choice("relation").map(|(choice, confidence)| (verdict(choice, confidence), confidence)),
            Err(e) => return on_event(CheckEvent::Notice(format!("Couldn't check the notes against the code: {e}"))),
        };
        let Some((verdict, confidence)) = picked else {
            return on_event(CheckEvent::Notice(
                "Couldn't check the notes against the code: Jev returned an unexpected answer.".into(),
            ));
        };
        on_event(CheckEvent::Result(CheckResult {
            claim,
            verdict,
            confidence: Some(confidence),
            path: Some(code.path),
            line: Some(line),
        }));
    }
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test -p livebrainstorm-core jev:: && cargo clippy --workspace --all-targets`
Expected: all `jev::` tests pass (6 client, 12 check); no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/jev/check.rs
git commit -m "feat(jev): run the fact-check concurrently and stream verdicts" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Core plumbing — `.env.local` reader, Jev settings, Rust section parser

**Files:**
- Create: `crates/livebrainstorm-core/src/env_file.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (add `pub mod env_file;`)
- Modify: `crates/livebrainstorm-core/Cargo.toml` (add `dotenvy = "0.15"` under `[dependencies]`)
- Modify: `crates/livebrainstorm-core/src/settings.rs` (add `JevSettings`)
- Modify: `crates/livebrainstorm-core/src/prompt.rs` (add `section_body`)

**Interfaces:**
- Consumes: `jev::DEFAULT_MODEL` (Task 2); `prompt::SECTIONS` (existing).
- Produces:
  - `env_file::read_value(path: &Path, name: &str) -> Option<String>`
  - `settings::JevSettings { gate: bool, rerank: bool, check: bool, model: String }` (all true / `"jev-latest"` by default), and the field `Settings.jev`
  - `prompt::section_body(markdown: &str, name: &str) -> Option<String>`, where `name` is one of `SECTIONS`, e.g. `"Faults"`

- [ ] **Step 1: Write the failing tests**

Create `crates/livebrainstorm-core/src/env_file.rs` with only the tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn read(contents: &str) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env.local");
        std::fs::write(&path, contents).unwrap();
        read_value(&path, "TYPESAFE_API_KEY")
    }

    #[test]
    fn reads_common_dotenv_spellings() {
        assert_eq!(read("TYPESAFE_API_KEY=ts-123\n").as_deref(), Some("ts-123"));
        assert_eq!(read("export TYPESAFE_API_KEY=\"ts-123\"\r\nOTHER=1\r\n").as_deref(), Some("ts-123"));
        assert_eq!(read("# key for Jev\nTYPESAFE_API_KEY='ts-123'\n").as_deref(), Some("ts-123"));
    }

    #[test]
    fn empty_or_missing_means_no_key() {
        assert_eq!(read("TYPESAFE_API_KEY=\n"), None);
        assert_eq!(read("OTHER=1\n"), None);
        assert_eq!(read_value(Path::new("/definitely/not/here/.env.local"), "TYPESAFE_API_KEY"), None);
    }
}
```

Append to `mod tests` in `settings.rs`:

```rust
    #[test]
    fn files_from_before_jev_get_its_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"provider":{"kind":"local","modelId":"qwen3-4b"},"debounceMs":900}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.debounce_ms, 900);
        assert_eq!(s.jev, JevSettings::default());
        assert!(s.jev.check && s.jev.model == "jev-latest");
    }
```

In the existing `round_trips_and_tolerates_garbage` test, change the struct literal to:

```rust
        let s = Settings { provider: ProviderSettings::Local { model_id: "qwen3-4b".into() }, debounce_ms: 900, ..Default::default() };
```

Append to `mod tests` in `prompt.rs`:

```rust
    #[test]
    fn section_body_matches_the_ui_parser() {
        let md = "## Intent\n- add caching\n## Faults\n- `cache.ts` exists\n- second\n## Directions\n- reuse it";
        assert_eq!(section_body(md, "Faults").as_deref(), Some("- `cache.ts` exists\n- second"));
        assert_eq!(section_body("**Faults:**\n- x\n### Questions for you\n- y", "Faults").as_deref(), Some("- x"));
        assert_eq!(section_body("Sure, here you go\n## Intent\n- a", "Faults"), None);
    }
```

- [ ] **Step 2: Run the tests and confirm they fail**

Add `pub mod env_file;` to `lib.rs` (after `pub mod error;`) and `dotenvy = "0.15"` to core `[dependencies]`.
Run: `cargo test -p livebrainstorm-core env_file settings prompt`
Expected: compile errors (`read_value`, `JevSettings`, `section_body` not found).

- [ ] **Step 3: Implement**

At the top of `env_file.rs`, above the tests:

```rust
//! Reads one value from a dotenv file without touching the process
//! environment. Used for the repo's `.env.local` in development builds.

use std::path::Path;

/// `name`'s value in the dotenv file at `path`; None if the file, the line or
/// the value is missing. Handles `export`, quotes and comments (dotenvy);
/// Windows line endings are normalised first.
pub fn read_value(path: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?.replace("\r\n", "\n");
    dotenvy::from_read_iter(text.as_bytes())
        .flatten()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
```

In `settings.rs`, add `use crate::jev::DEFAULT_MODEL;` with the other imports. Add the field to `Settings`:

```rust
    pub jev: JevSettings,
```

Add `jev: JevSettings::default(),` to `impl Default for Settings`. Then add, after `impl Default for Settings`:

```rust
/// Which Jev judgments run. They only run when a TypeSafe key is found.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct JevSettings {
    /// Skip re-analysis when an edit doesn't change the idea.
    pub gate: bool,
    /// Rerank the keyword shortlist of code snippets.
    pub rerank: bool,
    /// Fact-check the Faults against the code.
    pub check: bool,
    /// Pin a versioned ID (e.g. `jev-1.13.0`) once thresholds are tuned.
    pub model: String,
}

impl Default for JevSettings {
    fn default() -> Self {
        Self { gate: true, rerank: true, check: true, model: DEFAULT_MODEL.into() }
    }
}
```

In `prompt.rs`, after `analysis_request`:

```rust
/// Which of `SECTIONS` a heading line opens, as tolerant as
/// `src/lib/sections.ts`: "## Faults", "### Questions for you", "**Faults:**".
fn heading_key(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    let rest = match t.strip_prefix("**") {
        Some(rest) => rest,
        None => {
            let hashes = t.len() - t.trim_start_matches('#').len();
            if !(1..=4).contains(&hashes) {
                return None;
            }
            &t[hashes..]
        }
    };
    let word = rest.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next()?.to_lowercase();
    SECTIONS.iter().copied().find(|s| {
        let s = s.to_lowercase();
        word == s || word == s.trim_end_matches('s')
    })
}

/// The body under one section heading (e.g. "Faults"), if present. Mirrors
/// `parseSections` in `src/lib/sections.ts`; keep the two in sync.
pub fn section_body(markdown: &str, name: &str) -> Option<String> {
    let mut body: Option<String> = None;
    let mut current = None;
    for line in markdown.lines() {
        if let Some(key) = heading_key(line) {
            current = Some(key);
            if key == name {
                body.get_or_insert_with(String::new);
            }
            continue;
        }
        if current == Some(name) {
            if let Some(b) = body.as_mut() {
                b.push_str(line);
                b.push('\n');
            }
        }
    }
    body.map(|b| b.trim().to_owned())
}
```

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: all tests pass (the 17 original, the 3 Task 1 tests, and the new ones); no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core
git commit -m "feat(core): .env.local reader, Jev settings and a Rust section parser" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Shell — resolve the key and run the check after each analysis

**Files:**
- Modify: `src-tauri/src/secrets.rs` (accounts, `jev_key`)
- Modify: `src-tauri/src/provider.rs` (add `jev`)
- Modify: `src-tauri/src/commands.rs` (new `UiEvent` variants, `From<CheckEvent>`, `analyze`)
- Modify: `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes: `env_file::read_value` and `JevSettings` (Task 5); `jev::{Jev, check::{self, CheckEvent, CheckResult}}` (Tasks 2–4); `prompt::section_body` (Task 5).
- Produces (the IPC contract Task 7 mirrors in TypeScript), as `UiEvent` variants with tag `type` in camelCase:
  - `checking { count, total }`
  - `check { claim, verdict, confidence, path, line }`, where verdict is one of `"confirmed" | "contradicted" | "unclear" | "notShown" | "notFound"`
  - `notice { message }`
  - Check events arrive **after** `done`.

- [ ] **Step 1: Generalise the keychain helpers and add `jev_key`**

Replace the body of `src-tauri/src/secrets.rs` with:

```rust
//! Secrets live in the OS keychain (Keychain / Credential Manager / Secret
//! Service), never in settings.json or the webview.

use livebrainstorm_core::{Error, Result};

const SERVICE: &str = "app.livebrainstorm.desktop";
const ACCOUNT: &str = "anthropic-api-key";
const JEV_ACCOUNT: &str = "typesafe-api-key";
const JEV_ENV: &str = "TYPESAFE_API_KEY";

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account).map_err(|e| Error::Provider(format!("Keychain unavailable: {e}")))
}

fn keychain_get(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Provider(format!("Couldn't read the key from the keychain: {e}"))),
    }
}

pub fn api_key() -> Result<Option<String>> {
    keychain_get(ACCOUNT)
}

pub fn set_api_key(key: &str) -> Result<()> {
    let entry = entry(ACCOUNT)?;
    let result = if key.trim().is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(key.trim())
    };
    result.map_err(|e| Error::Provider(format!("Couldn't save the API key to the keychain: {e}")))
}

/// The TypeSafe key: Keychain, then `TYPESAFE_API_KEY`, then (debug builds
/// only) the repo's `.env.local`. It never crosses IPC.
pub fn jev_key() -> Option<String> {
    let present = |k: String| Some(k.trim().to_owned()).filter(|k| !k.is_empty());
    keychain_get(JEV_ACCOUNT)
        .ok()
        .flatten()
        .and_then(present)
        .or_else(|| std::env::var(JEV_ENV).ok().and_then(present))
        .or_else(dev_env_file_key)
}

/// Release builds never read a `.env` from the source tree.
#[cfg(debug_assertions)]
fn dev_env_file_key() -> Option<String> {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env.local"));
    livebrainstorm_core::env_file::read_value(path, JEV_ENV)
}

#[cfg(not(debug_assertions))]
fn dev_env_file_key() -> Option<String> {
    None
}
```

- [ ] **Step 2: Add the Jev client factory** (append to `src-tauri/src/provider.rs`; add `use livebrainstorm_core::jev::Jev;` to its imports)

```rust
/// A Jev client when a TypeSafe key is found; callers check the feature switches.
pub fn jev(app: &AppHandle) -> Option<Jev> {
    let state = app.state::<AppState>();
    let key = secrets::jev_key()?;
    Some(Jev::new(key, state.settings().jev.model, state.http.clone()))
}
```

- [ ] **Step 3: Extend `UiEvent` and run the check in `analyze`**

In `src-tauri/src/commands.rs`, add to the imports:

```rust
use livebrainstorm_core::jev::check::{self, CheckEvent, CheckResult};
```

Add the variants to `UiEvent` after `Progress`:

```rust
    /// `count` of the `total` Faults name code that's being checked.
    Checking { count: usize, total: usize },
    Check(CheckResult),
    /// Non-fatal: the notes are fine, something around them failed.
    Notice { message: String },
```

Add, after the `UiEvent` enum:

```rust
impl From<CheckEvent> for UiEvent {
    fn from(ev: CheckEvent) -> Self {
        match ev {
            CheckEvent::Checking { count, total } => UiEvent::Checking { count, total },
            CheckEvent::Result(result) => UiEvent::Check(result),
            CheckEvent::Notice(message) => UiEvent::Notice { message },
        }
    }
}
```

Replace the whole `analyze` command with:

```rust
#[tauri::command]
pub async fn analyze(app: AppHandle, draft: String, on_event: Channel<UiEvent>) -> Result<()> {
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let index = state.project();
        let result = async {
            let provider = provider::resolve(&task_app, Tier::Fast).await?;
            let brief = state.brief.read().expect("brief lock").as_ref().map(|b| b.text.clone());

            let budget = provider.context_budget_chars();
            let max_files = if budget < 40_000 { 3 } else { 5 };
            let snippets = index
                .as_deref()
                .map(|i| project::retrieve(i, &draft, max_files, budget / 4))
                .unwrap_or_default();

            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider);
            provider.stream(&req, &mut on_chunk(&on_event)).await
        }
        .await;
        let text = match result {
            Ok(text) => text,
            Err(e) => return send(&on_event, UiEvent::Error { message: e.to_string() }),
        };
        send(&on_event, UiEvent::Done { text: text.clone() });

        // "Checked in code" fills in after the notes. The next keystroke
        // aborts this task, checks included.
        if !state.settings().jev.check {
            return;
        }
        if let (Some(index), Some(faults)) = (index, prompt::section_body(&text, "Faults")) {
            if let Some(jev) = provider::jev(&task_app) {
                check::run(&jev, &index, &faults, |ev| send(&on_event, ev.into())).await;
            }
        }
    });
    replace_task(&app.state::<AppState>().analysis, task);
    Ok(())
}
```

- [ ] **Step 4: Build and lint the shell**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: no warnings; all core tests pass. (`src-tauri` has no tests by design.)

- [ ] **Step 5: Document**

In `AGENTS.md`, add this subsection after "### Providers" (before "### Errors"):

```markdown
### Jev (TypeSafe)

`livebrainstorm-core::jev` is a typed client for TypeSafe's System One API (`POST https://api.typesafe.ai/v1/systemone`): one `state`, any number of Noul/Choice questions, typed answers with probabilities. It judges; it never writes text. Choice options serialize in the order given (`jev::Options`) because Jev 1.13 can lean towards the first option.

- **Fact-check ("Checked in code"):** after `done`, `commands::analyze` takes the Faults section (`prompt::section_body`) and runs `jev::check::run`. Plain code finds the evidence: backticked names and paths in each bullet, matched against the in-memory index (names found in more than a quarter of the files are too generic). Jev answers supports / contradicts / says_nothing per claim; ✓/✗ need confidence ≥ 0.8. Check events arrive after `done`, and the next keystroke aborts them with the rest of the task.
- **Key lookup** (`secrets::jev_key`): Keychain account `typesafe-api-key`, then `TYPESAFE_API_KEY` in the environment, then, in debug builds only, `TYPESAFE_API_KEY` in the repo's `.env.local` (read by `env_file::read_value`, never exported). The key never crosses IPC. **Never prefix it `VITE_`:** Vite reads `.env.local` too and ships `VITE_*` variables to the webview bundle.
- Jev failures never fail an analysis; they surface as one `notice` event.
```

In `AGENTS.md`, under "## Cross-language contracts", add:

```markdown
- **Section parsing:** `prompt::section_body` (Rust) mirrors `parseSections` in `src/lib/sections.ts`; change both together.
- **Check events:** `CheckResult` / `Verdict` in `jev/check.rs` ↔ `CheckResult` / `Verdict` in `src/lib/ipc.ts`.
```

In `README.md`, add after the provider table (before "## How it works"):

```markdown
### Optional: fact-check notes with Jev

With a [TypeSafe](https://typesafe.ai) API key, LiveBrainstorm checks every Fault in its notes against your code and marks it ✓ confirmed, ✗ contradicted, or unclear. For local development, put the key in `.env.local` at the repo root (git ignores it):

    TYPESAFE_API_KEY=your-key

Development builds read it on each analysis, so edits apply without a restart. Packaged builds ignore `.env.local`; set `TYPESAFE_API_KEY` in the environment instead.
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src AGENTS.md README.md
git commit -m "feat: run Jev fact-checks after each analysis, key from .env.local in dev" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Frontend — "Checked in code"

**Files:**
- Modify: `src/lib/ipc.ts` (event and settings types)
- Create: `src/lib/analysisState.ts`, `src/lib/analysisState.test.ts`
- Create: `src/lib/checks.ts`, `src/lib/checks.test.ts`
- Modify: `src/lib/useLiveAnalysis.ts` (use the reducer)
- Modify: `src/components/NotesPane.tsx` (notice line and the "Checked in code" section)

**Interfaces:**
- Consumes: the `UiEvent` additions from Task 6.
- Produces:
  - `applyEvent(state: AnalysisState, ev: UiEvent): AnalysisState`
  - `startAnalysis(draft: string): AnalysisState` and `initialAnalysis`
  - `VERDICT`, `checkSummary(checking, done, stopped)`, `checkLocation(result)` and `plainClaim(claim)`

- [ ] **Step 1: Mirror the IPC types** (`src/lib/ipc.ts`)

Replace the `UiEvent` type with:

```ts
export type Verdict = "confirmed" | "contradicted" | "unclear" | "notShown" | "notFound";

export interface CheckResult {
  claim: string;
  verdict: Verdict;
  confidence: number | null;
  path: string | null;
  line: number | null;
}

export type UiEvent =
  | { type: "text"; text: string }
  | { type: "activity"; label: string }
  | { type: "done"; text: string }
  | { type: "error"; message: string }
  | { type: "progress"; downloaded: number; total: number | null }
  | { type: "checking"; count: number; total: number }
  | ({ type: "check" } & CheckResult)
  | { type: "notice"; message: string };
```

Replace the `Settings` interface with:

```ts
export interface JevSettings {
  gate: boolean;
  rerank: boolean;
  check: boolean;
  model: string;
}

export interface Settings {
  provider: ProviderSettings;
  debounceMs: number;
  jev: JevSettings;
}
```

- [ ] **Step 2: Write the failing tests**

`src/lib/analysisState.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { applyEvent, startAnalysis } from "./analysisState";
import type { UiEvent } from "./ipc";

const run = (events: UiEvent[]) => events.reduce(applyEvent, startAnalysis("draft"));
const check: UiEvent = { type: "check", claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("applyEvent", () => {
  it("streams text, then completes", () => {
    const s = run([{ type: "text", text: "## Intent\n" }, { type: "text", text: "- a" }, { type: "done", text: "" }]);
    expect(s.status).toBe("done");
    expect(s.markdown).toBe("## Intent\n- a");
  });

  it("keeps applying checks after done", () => {
    const s = run([{ type: "done", text: "## Faults\n- `x` is wrong" }, { type: "checking", count: 1, total: 2 }, check]);
    expect(s.status).toBe("done");
    expect(s.checking).toEqual({ count: 1, total: 2 });
    expect(s.checks).toEqual([{ claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 }]);
  });

  it("a notice never turns the notes into an error", () => {
    const s = run([{ type: "done", text: "## Intent\n- a" }, { type: "notice", message: "Couldn't check" }]);
    expect(s.status).toBe("done");
    expect(s.markdown).toBe("## Intent\n- a");
    expect(s.notice).toBe("Couldn't check");
  });

  it("a new run drops the previous checks and notice", () => {
    const before = run([{ type: "checking", count: 1, total: 1 }, check, { type: "notice", message: "x" }]);
    expect(before.checks).toHaveLength(1);
    const next = startAnalysis("new draft");
    expect(next.checks).toEqual([]);
    expect(next.checking).toBeNull();
    expect(next.notice).toBeNull();
    expect(next.status).toBe("thinking");
  });
});
```

`src/lib/checks.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { checkLocation, checkSummary, plainClaim } from "./checks";

describe("checkSummary", () => {
  it("counts down while verdicts arrive", () => {
    expect(checkSummary({ count: 2, total: 3 }, 1, false)).toBe("Checking 2 of 3 faults…");
    expect(checkSummary({ count: 2, total: 3 }, 2, false)).toBe("2 of 3 faults checked");
  });

  it("stops claiming progress after a failure", () => {
    expect(checkSummary({ count: 2, total: 3 }, 1, true)).toBe("1 of 3 faults checked");
  });

  it("uses the singular for one fault", () => {
    expect(checkSummary({ count: 1, total: 1 }, 1, false)).toBe("1 of 1 fault checked");
  });
});

describe("display helpers", () => {
  it("formats where the evidence is", () => {
    expect(checkLocation({ claim: "c", verdict: "confirmed", confidence: 0.9, path: "src/a.ts", line: 4 })).toBe("src/a.ts:4");
    expect(checkLocation({ claim: "c", verdict: "notFound", confidence: null, path: null, line: null })).toBeNull();
  });

  it("strips inline markdown from claims", () => {
    expect(plainClaim("**Bold** `createInvoice` ignores discounts")).toBe("Bold createInvoice ignores discounts");
  });
});
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `npm test`
Expected: FAIL, `Failed to resolve import "./analysisState"` and `"./checks"`.

- [ ] **Step 4: Implement the pure modules**

`src/lib/analysisState.ts`:

```ts
// Pure state transitions for the live analysis, testable without React.
// Check events may arrive after `done`: the notes are complete before
// "Checked in code" fills in.
import type { CheckResult, UiEvent } from "./ipc";

export type AnalysisStatus = "idle" | "thinking" | "done" | "error";

export interface AnalysisState {
  status: AnalysisStatus;
  markdown: string;
  error: string | null;
  analyzedDraft: string;
  checking: { count: number; total: number } | null;
  checks: CheckResult[];
  notice: string | null;
}

export const initialAnalysis: AnalysisState = {
  status: "idle",
  markdown: "",
  error: null,
  analyzedDraft: "",
  checking: null,
  checks: [],
  notice: null,
};

/** A new run starts clean: no notes, checks or notices from the previous draft. */
export const startAnalysis = (draft: string): AnalysisState => ({ ...initialAnalysis, status: "thinking", analyzedDraft: draft });

export function applyEvent(s: AnalysisState, ev: UiEvent): AnalysisState {
  switch (ev.type) {
    case "text":
      return { ...s, markdown: s.markdown + ev.text };
    case "done":
      return { ...s, status: "done", markdown: ev.text || s.markdown };
    case "error":
      return { ...s, status: "error", error: ev.message };
    case "checking":
      return { ...s, checking: { count: ev.count, total: ev.total } };
    case "check":
      return {
        ...s,
        checks: [...s.checks, { claim: ev.claim, verdict: ev.verdict, confidence: ev.confidence, path: ev.path, line: ev.line }],
      };
    case "notice":
      return { ...s, notice: ev.message };
    default:
      return s;
  }
}
```

`src/lib/checks.ts`:

```ts
// Display rules for "Checked in code". Verdicts are signals, not proof.
import type { CheckResult, Verdict } from "./ipc";

export const VERDICT: Record<Verdict, { mark: string; label: string; tone: "accent" | "fault" | "faint" }> = {
  confirmed: { mark: "✓", label: "Confirmed", tone: "accent" },
  contradicted: { mark: "✗", label: "Contradicted", tone: "fault" },
  unclear: { mark: "?", label: "Unclear", tone: "faint" },
  notShown: { mark: "–", label: "Not shown in this code", tone: "faint" },
  notFound: { mark: "–", label: "Not found in indexed code", tone: "faint" },
};

/** `done` verdicts so far; `stopped` once Jev failed and the rest were dropped. */
export function checkSummary(checking: { count: number; total: number }, done: number, stopped: boolean): string {
  const faults = checking.total === 1 ? "fault" : "faults";
  if (!stopped && done < checking.count) return `Checking ${checking.count} of ${checking.total} ${faults}…`;
  return `${done} of ${checking.total} ${faults} checked`;
}

export const checkLocation = (r: CheckResult) => (r.path ? `${r.path}${r.line ? `:${r.line}` : ""}` : null);

/** One-line display text: inline `code` and **bold** markers removed. */
export const plainClaim = (claim: string) => claim.replace(/\*\*|`/g, "");
```

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `npm test`
Expected: PASS: `sections.test.ts` (3), `analysisState.test.ts` (4), `checks.test.ts` (5).

- [ ] **Step 6: Use the reducer in the hook** (`src/lib/useLiveAnalysis.ts`, full replacement)

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./ipc";
import { applyEvent, initialAnalysis, startAnalysis, type AnalysisState } from "./analysisState";

const MIN_CHARS = 24;
const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/**
 * Re-analyzes the draft after the user pauses typing.
 *
 * Like a colleague reading over your shoulder: they wait until you stop
 * typing, then comment; if you keep going, their last comment stays visible
 * but marked as out of date. Old responses are dropped by request id; the
 * backend also aborts the superseded model call (and its fact-checks).
 */
export function useLiveAnalysis(draft: string, debounceMs: number, contextKey: string) {
  const [state, setState] = useState<AnalysisState>(initialAnalysis);
  const requestId = useRef(0);
  const lastSent = useRef("");

  const run = useCallback((text: string) => {
    const id = ++requestId.current;
    lastSent.current = normalize(text);
    setState(startAnalysis(text));
    const mine = () => id === requestId.current;

    api
      .analyze(text, (ev) => {
        if (mine()) setState((s) => applyEvent(s, ev));
      })
      .catch((e) => mine() && setState((s) => ({ ...s, status: "error", error: errorMessage(e) })));
  }, []);

  // New project or brief: the same draft deserves a fresh read.
  useEffect(() => {
    lastSent.current = "";
  }, [contextKey]);

  useEffect(() => {
    const norm = normalize(draft);
    if (norm.length < MIN_CHARS || norm === lastSent.current) return;
    const timer = setTimeout(() => run(draft), debounceMs);
    return () => clearTimeout(timer);
  }, [draft, debounceMs, contextKey, run]);

  return {
    ...state,
    tooShort: normalize(draft).length < MIN_CHARS,
    stale: state.status !== "idle" && normalize(state.analyzedDraft) !== normalize(draft),
    rerun: () => run(draft),
  };
}
```

- [ ] **Step 7: Render it** (`src/components/NotesPane.tsx`)

Add imports:

```tsx
import type { CheckResult } from "../lib/ipc";
import { VERDICT, checkLocation, checkSummary, plainClaim } from "../lib/checks";
```

Add, after the `Rule` component:

```tsx
const TONE = { accent: "text-accent", fault: "text-fault", faint: "text-faint" } as const;

function CheckRow({ check }: { check: CheckResult }) {
  const v = VERDICT[check.verdict];
  const where = checkLocation(check);
  return (
    <li className="grid grid-cols-[1rem_minmax(0,1fr)] gap-x-2 text-[13px] leading-snug">
      <span className={cx("font-semibold", TONE[v.tone])} aria-hidden>
        {v.mark}
      </span>
      <span className="min-w-0">
        <span className="block truncate" title={plainClaim(check.claim)}>
          {plainClaim(check.claim)}
        </span>
        <span className="text-xs text-muted">
          {v.label}
          {where && (
            <>
              {" · "}
              <code className="font-mono text-[0.85em]">{where}</code>
            </>
          )}
        </span>
      </span>
    </li>
  );
}
```

Directly after the status `<p … aria-live="polite">…</p>`, add:

```tsx
        {a.notice && <p className="mb-4 text-xs text-muted">{a.notice}</p>}
```

Inside the notes `<div className={cx("grid gap-6 …")}>`, after the `{a.status === "thinking" && !a.markdown && …}` line, add:

```tsx
            {a.checking && (
              <section className="grid gap-1.5">
                <h2>
                  <Eyebrow dot="bg-ink">Checked in code</Eyebrow>
                </h2>
                <p className="text-xs text-faint">{checkSummary(a.checking, a.checks.length, !!a.notice)}</p>
                <ul className="grid gap-2">
                  {a.checks.map((c, i) => (
                    <CheckRow key={i} check={c} />
                  ))}
                </ul>
              </section>
            )}
```

- [ ] **Step 8: Verify**

Run: `npm test && npm run typecheck && PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH" npm run build`
Expected: 12 tests pass; no type errors; the build prints `✓ built in`.

- [ ] **Step 9: Commit**

```bash
git add src
git commit -m "feat(ui): Checked in code section with verdicts and a quiet notice line" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Measure Jev and verify end to end

**Files:**
- Create: `crates/livebrainstorm-core/tests/jev_live.rs`
- Modify: `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (add a "Measurements" section)
- Modify: `AGENTS.md` (the live-test command)

**Interfaces:**
- Consumes: `env_file::read_value`, `jev::{Jev, NoulCriteria, Question, DEFAULT_MODEL}`, `jev::check::{question, state}`, `project::Snippet`.
- Produces: measured Jev latency numbers in the spec (phase 2 sets `PRE_BUDGET` from them).

- [ ] **Step 1: Write the opt-in live test** (`crates/livebrainstorm-core/tests/jev_live.rs`)

```rust
//! Calls the real TypeSafe API: a gate-style Noul and a check-style Choice,
//! three rounds each, printing latency so phase 2 can size its time budget.
//! Needs TYPESAFE_API_KEY in the environment or in the repo's .env.local:
//!   cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use livebrainstorm_core::env_file;
use livebrainstorm_core::jev::{check, Jev, NoulCriteria, Question, DEFAULT_MODEL};
use livebrainstorm_core::project::Snippet;
use serde_json::json;

fn key() -> Option<String> {
    std::env::var("TYPESAFE_API_KEY").ok().filter(|k| !k.trim().is_empty()).or_else(|| {
        env_file::read_value(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env.local")), "TYPESAFE_API_KEY")
    })
}

#[tokio::test]
#[ignore = "calls the TypeSafe API"]
async fn noul_and_choice_round_trip() {
    let Some(key) = key() else {
        eprintln!("skipped: no TYPESAFE_API_KEY in the environment or .env.local");
        return;
    };
    let jev = Jev::new(key, DEFAULT_MODEL.into(), reqwest::Client::new());

    let gate = BTreeMap::from([(
        "changed".to_owned(),
        Question::Noul {
            instructions: json!("Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?"),
            criteria: Some(NoulCriteria {
                yes: json!("At least one idea, goal, constraint, or question was added, removed, or changed in meaning."),
                no: json!("Only wording, spelling, punctuation, or formatting changed; the ideas are the same."),
            }),
        },
    )]);
    let drafts = json!({
        "previous_draft": "Cache the project index between launches.",
        "current_draft": "Cache the project index between launches, and keep one brief per git branch."
    });
    for round in 1..=3 {
        let t = Instant::now();
        let r = jev.ask(&drafts, &gate, Duration::from_secs(10)).await.unwrap();
        let noul = r.noul("changed").expect("a noul answer");
        assert!((0.0..=1.0).contains(&noul));
        eprintln!("gate noul   round {round}: {:>5} ms  noul={noul:.2}  model={}", t.elapsed().as_millis(), r.model);
    }

    let relation = BTreeMap::from([("relation".to_owned(), check::question())]);
    let code = Snippet {
        path: "src/billing/invoice.ts".into(),
        start_line: 1,
        text: "export function createInvoice(cartId: string) {\n  return { cartId, total: 0 };\n}".into(),
    };
    let claim_state = check::state("`createInvoice` ignores the cart's items when computing the total.", &code);
    for round in 1..=3 {
        let t = Instant::now();
        let r = jev.ask(&claim_state, &relation, Duration::from_secs(10)).await.unwrap();
        let (choice, confidence) = r.choice("relation").expect("a choice answer");
        assert!(["supports", "contradicts", "says_nothing"].contains(&choice));
        eprintln!("check choice round {round}: {:>5} ms  {choice} ({confidence:.2})", t.elapsed().as_millis());
    }
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture 2>&1 | grep -E "round|skipped|test result"`
Expected: six `round` lines with latencies, then `test result: ok. 1 passed`. If it prints `skipped`, the key isn't in `.env.local`; stop and ask the user. Never print the key itself.

- [ ] **Step 3: Record the measurements in the spec**

Append to `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md`, using the real numbers from Step 2:

```markdown
## Measurements (phase 1, 2026-10-08)

`cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture`, model as reported (e.g. `jev-1.13.0`):

| Request | Round 1 (cold) | Rounds 2–3 (warm) |
|---|---|---|
| Gate Noul (two short drafts) | <ms> | <ms>, <ms> |
| Check Choice (claim + 3-line snippet) | <ms> | <ms>, <ms> |

Phase 2 sizes `PRE_BUDGET` from the warm numbers (budget ≈ 3 × the slower warm round, capped at 2,000 ms).
```

Replace each `<ms>` with the measured value. Don't leave any placeholders in the committed file.

- [ ] **Step 4: Note the live test in AGENTS.md**

Append to the "### Jev (TypeSafe)" bullets in `AGENTS.md`:

```markdown
- Live test against the real API (needs the key; prints latency): `cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture`.
```

- [ ] **Step 5: End-to-end check in the running app**

1. Ask the user to restart their `npm run tauri dev`. `secrets.rs` changed, so the Rust side must rebuild; `.env.local` is read on each analysis.
2. Open this repo as the project in LiveBrainstorm and type a draft that names real code, e.g. "Every pause spawns `claude -p` from `ClaudeCli::stream`; I want to keep one process alive and reuse it across `analyze` calls."
3. Expected: notes stream in as before, and then a **Checked in code** section shows "Checking k of m faults…" turning into rows with ✓/✗/unclear and `path:line`.
4. Type again mid-check. Expected: the old rows disappear with the old notes.
5. Then run `pgrep -fl "claude -p" || echo none`. Expected: `none` once the new analysis finishes, with no orphaned processes.

- [ ] **Step 6: Final verification and commit**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm test && npm run typecheck`
Expected: no warnings; all tests pass.

```bash
git add crates/livebrainstorm-core/tests/jev_live.rs docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md AGENTS.md
git commit -m "test(jev): opt-in live latency test and recorded phase 1 measurements" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
