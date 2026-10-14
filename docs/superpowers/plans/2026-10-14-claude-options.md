# Phase A: Claude options (API key, model, effort) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** With the Claude Code provider, the user can sign in with their own Anthropic API key instead of their Claude Code login, pick any model (an alias or a full id), and set an effort level per tier. The Anthropic API provider gets the same effort settings.

**Architecture:**
- **Core:** `settings.rs` gains `Effort` and `ClaudeAuth` plus per-tier effort fields. It reads them leniently, so a bad value never resets settings.json.
- **`llm`:**
  - New `Secret` wrapper whose `Debug` never prints the key.
  - `ClaudeCli` passes `--effort` and puts the key in the child's environment (`ANTHROPIC_API_KEY`), never in argv.
  - `AnthropicApi` sends `output_config.effort`.
- **Shell:** `provider::resolve` picks the tier's effort and reads the shared keychain key when `auth` is `apiKey`.
- **UI:**
  - New pure helpers in `src/lib/providerOptions.ts`.
  - The Settings dialog gains a sign-in choice, a shared `ApiKeyField`, a `ModelPicker` with *Custom…*, and an `EffortSelect` per tier.

**Tech Stack:** Rust 2021 (serde, serde_json, tokio::process), Tauri v2, React 19 + Tailwind v4, vitest 3.

**Spec:** `docs/superpowers/specs/2026-10-14-session-instructions-and-claude-options-design.md`, Part A ("Claude options"), plus the shared sections "Testing", "Implementation order" and "Documentation updates".

## Global Constraints

- **`Effort`** values: `low | medium | high | xhigh | max` (serde lowercase). `None` means "Default": no `--effort` flag and no `output_config`.
- **`ClaudeAuth`** values: `login` (default, today's behaviour) | `apiKey` (serde camelCase).
- **New fields** are all `#[serde(default)]`:
  - `ClaudeCli`: `fast_effort`, `deep_effort`, `auth`.
  - `AnthropicApi`: `fast_effort`, `deep_effort`.

  A `settings.json` from before this phase loads unchanged.
- **Key storage:** the keychain (service `app.livebrainstorm.desktop`, account `anthropic-api-key`). Shared with the Anthropic API provider.
- **Key handling:** the key reaches `claude` only as the `ANTHROPIC_API_KEY` environment variable. It never goes into argv, settings.json, IPC or `Debug` output.
- **Copy, verbatim:**

  | Where | Text |
  |---|---|
  | Missing key (Precondition) | "Add your Anthropic API key in Settings, or switch Claude Code back to your login." |
  | Old CLI | "Claude Code is too old for LiveBrainstorm. Update it with `claude update`." |
  | Rejected key | "Claude Code rejected the Anthropic API key. Check the key in Settings, or switch Claude Code back to your login." |
  | Billing note | "Usage is billed to your API account, not your Claude subscription." |
  | No key yet | "No key saved yet." |
  | Blank model | "Enter a model name for While you type" (or "Project brief") |

- **Model aliases** in this order: `haiku`, `sonnet`, `opus`, `fable`, then *Custom…*.
- **Effort labels:** Default, Low, Medium, High, Extra high, Max.
- **Labels:**
  - Top bar: "Claude Code (API key), haiku" (otherwise "Claude Code, haiku").
  - `ProviderSettings::label()`: "Claude Code (API key, haiku)" (otherwise "Claude Code (haiku)").
- **Process:**
  - Branch `feat/instructions-and-claude-options`, cut from `main` (feat/parallel-notes was merged).
  - Commit trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
  - Never run `cargo fmt` across the repo. Match the hand-wrapped ~120-column style.
  - `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests.
  - UI uses the semantic Tailwind tokens and `ui.tsx` primitives.
  - `npm run tauri dev` is run by the user, because the agent shell's Node 20.11 is too old for Vite 7. `npm run typecheck` and `npm test` work on it.

## Review Focus

1. **A hand-edited or future value** such as `"fastEffort": "extreme"` or `"auth": "sso"` must not reset every setting. `Settings::load` falls back to defaults on *any* error. Expected: only that field falls back. Pinned in Task 1 (`an_unknown_effort_or_auth_never_resets_other_settings`).
2. **A custom model with stray spaces, or left blank.** Expected: trimmed before saving, and a blank one blocks saving with a message naming the tier. Pinned in Task 4 (`trims model names before saving`, `names the tier whose model is blank`).
3. **A wrong or revoked key.** Expected: the notes pane gives an instruction, not raw CLI text. Pinned in Task 2 (`errors_say_what_to_do`).
4. **The key in a process listing or a debug print.** Expected: it appears in neither. Pinned in Task 2 (`the_api_key_goes_in_the_environment_never_in_args`, `a_secret_never_prints`).
5. **"My Anthropic API key" chosen with no key saved.** Expected: Settings says so before saving, and analysis then shows the missing-key Precondition, not a CLI failure. Pinned by Task 3's `resolve` arm and Task 5's "No key saved yet." line (manual check 2).

---

### Task 1: Core: effort and sign-in settings

**Files:**
- Modify: `crates/livebrainstorm-core/src/settings.rs`
- Modify: `src-tauri/src/provider.rs` (two match patterns gain `..`, only so the shell still compiles)
- Commit: `docs/superpowers/specs/2026-10-14-session-instructions-and-claude-options-design.md`, `docs/superpowers/plans/2026-10-14-claude-options.md`, `docs/superpowers/plans/2026-10-14-session-instructions.md`

**Interfaces:**
- Produces:
  - `settings::Effort { Low, Medium, High, Xhigh, Max }` with `pub fn as_str(self) -> &'static str` (`"low"` … `"max"`)
  - `settings::ClaudeAuth { Login, ApiKey }` (`Default` = `Login`)
  - `ProviderSettings::ClaudeCli { binary_path: Option<String>, fast_model: String, deep_model: String, fast_effort: Option<Effort>, deep_effort: Option<Effort>, auth: ClaudeAuth }`
  - `ProviderSettings::AnthropicApi { fast_model: String, deep_model: String, fast_effort: Option<Effort>, deep_effort: Option<Effort> }`

- [ ] **Step 1: Branch and commit the design docs**

```bash
git switch feat/parallel-notes && git status --short   # expect only the three untracked docs
git switch -c feat/instructions-and-claude-options
git add docs/superpowers/specs/2026-10-14-session-instructions-and-claude-options-design.md \
        docs/superpowers/plans/2026-10-14-claude-options.md \
        docs/superpowers/plans/2026-10-14-session-instructions.md
git commit -m "docs: session instructions and Claude options design and plans

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 2: Write the failing tests**

Append inside `mod tests` at the end of `crates/livebrainstorm-core/src/settings.rs`, before its closing `}`:

```rust
    #[test]
    fn files_from_before_effort_get_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"provider":{"kind":"claudeCli","binaryPath":null,"fastModel":"haiku","deepModel":"sonnet"},"debounceMs":900}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.debounce_ms, 900);
        assert_eq!(
            s.provider,
            ProviderSettings::ClaudeCli {
                binary_path: None,
                fast_model: "haiku".into(),
                deep_model: "sonnet".into(),
                fast_effort: None,
                deep_effort: None,
                auth: ClaudeAuth::Login,
            }
        );
        std::fs::write(&path, r#"{"provider":{"kind":"anthropicApi","fastModel":"a","deepModel":"b"}}"#).unwrap();
        assert_eq!(
            Settings::load(&path).provider,
            ProviderSettings::AnthropicApi { fast_model: "a".into(), deep_model: "b".into(), fast_effort: None, deep_effort: None }
        );
    }

    #[test]
    fn effort_and_auth_serialize_as_the_ui_expects() {
        let p = ProviderSettings::ClaudeCli {
            binary_path: None,
            fast_model: "haiku".into(),
            deep_model: "opus".into(),
            fast_effort: Some(Effort::Low),
            deep_effort: Some(Effort::Xhigh),
            auth: ClaudeAuth::ApiKey,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "claudeCli", "binaryPath": null, "fastModel": "haiku", "deepModel": "opus",
                               "fastEffort": "low", "deepEffort": "xhigh", "auth": "apiKey"})
        );
        assert_eq!(serde_json::from_value::<ProviderSettings>(json).unwrap(), p);
        for e in [Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh, Effort::Max] {
            assert_eq!(serde_json::to_value(e).unwrap(), e.as_str(), "the CLI flag and the JSON agree");
        }
    }

    #[test]
    fn an_unknown_effort_or_auth_never_resets_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"provider":{"kind":"claudeCli","binaryPath":null,"fastModel":"opus","deepModel":"sonnet",
                "fastEffort":"extreme","deepEffort":"max","auth":"sso"},"debounceMs":900}"#,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.debounce_ms, 900, "the rest of the file still loads");
        let ProviderSettings::ClaudeCli { fast_model, fast_effort, deep_effort, auth, .. } = s.provider else {
            panic!("kept the provider")
        };
        assert_eq!((fast_model.as_str(), fast_effort, deep_effort, auth), ("opus", None, Some(Effort::Max), ClaudeAuth::Login));
    }

    #[test]
    fn the_label_says_when_claude_code_uses_an_api_key() {
        let mut s = Settings::default();
        assert_eq!(s.provider.label(), "Claude Code (haiku)");
        if let ProviderSettings::ClaudeCli { auth, .. } = &mut s.provider {
            *auth = ClaudeAuth::ApiKey;
        }
        assert_eq!(s.provider.label(), "Claude Code (API key, haiku)");
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core settings::tests`
Expected: compile error, `cannot find type 'Effort' in this scope` (and `ClaudeAuth`).

- [ ] **Step 4: Implement the types, fields, label and default**

In `crates/livebrainstorm-core/src/settings.rs`, replace the `ProviderSettings` enum and its `impl` (from `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]` above `pub enum ProviderSettings` through the end of `impl ProviderSettings`) with:

```rust
/// How hard the model thinks: `claude --effort`, or `output_config.effort` in
/// the Messages API. `None` in settings means the model's own default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// How the Claude Code CLI authenticates.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeAuth {
    /// Claude Code's own login (the subscription).
    #[default]
    Login,
    /// The Anthropic API key from the keychain, given to `claude` as ANTHROPIC_API_KEY.
    ApiKey,
}

/// Reads a field that must never reset the whole file: `Settings::load` falls
/// back to defaults on any error, so an unknown value (a typo, or a level a
/// newer version wrote) becomes the field's default instead.
fn lenient<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProviderSettings {
    /// Your installed Claude Code CLI, with its login or your API key.
    #[serde(rename_all = "camelCase")]
    ClaudeCli {
        binary_path: Option<String>,
        fast_model: String,
        deep_model: String,
        #[serde(default, deserialize_with = "lenient")]
        fast_effort: Option<Effort>,
        #[serde(default, deserialize_with = "lenient")]
        deep_effort: Option<Effort>,
        #[serde(default, deserialize_with = "lenient")]
        auth: ClaudeAuth,
    },
    /// Anthropic API key (stored in the OS keychain, never in this file).
    #[serde(rename_all = "camelCase")]
    AnthropicApi {
        fast_model: String,
        deep_model: String,
        #[serde(default, deserialize_with = "lenient")]
        fast_effort: Option<Effort>,
        #[serde(default, deserialize_with = "lenient")]
        deep_effort: Option<Effort>,
    },
    /// A downloaded GGUF model served by the managed llama.cpp runtime.
    #[serde(rename_all = "camelCase")]
    Local { model_id: String },
}

impl ProviderSettings {
    pub fn label(&self) -> String {
        match self {
            Self::ClaudeCli { fast_model, auth: ClaudeAuth::ApiKey, .. } => format!("Claude Code (API key, {fast_model})"),
            Self::ClaudeCli { fast_model, .. } => format!("Claude Code ({fast_model})"),
            Self::AnthropicApi { fast_model, .. } => format!("Anthropic API ({fast_model})"),
            Self::Local { model_id } => format!("Local ({model_id})"),
        }
    }
}
```

In `impl Default for Settings`, replace the `provider:` initializer with:

```rust
            provider: ProviderSettings::ClaudeCli {
                binary_path: None,
                // Claude Code accepts aliases, which track the latest models.
                fast_model: "haiku".into(),
                deep_model: "sonnet".into(),
                fast_effort: None,
                deep_effort: None,
                auth: ClaudeAuth::Login,
            },
```

- [ ] **Step 5: Keep the shell compiling**

In `src-tauri/src/provider.rs`, inside `resolve`, change the two patterns so they ignore the new fields (Task 3 uses them):

```rust
        ProviderSettings::ClaudeCli { binary_path, fast_model, deep_model, .. } => {
```

```rust
        ProviderSettings::AnthropicApi { fast_model, deep_model, .. } => {
```

- [ ] **Step 6: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core settings::tests && cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: all settings tests PASS (the four new ones plus the existing ones), the shell compiles, and clippy prints no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/settings.rs src-tauri/src/provider.rs
git commit -m "feat(core): per-tier effort and API-key sign-in for Claude Code

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Core: the effort flag, the key in the environment, and errors that say what to do

**Files:**
- Modify: `crates/livebrainstorm-core/src/llm/mod.rs` (add `Secret` and its test)
- Modify: `crates/livebrainstorm-core/src/llm/claude_cli.rs`
- Modify: `crates/livebrainstorm-core/src/llm/anthropic_api.rs`
- Modify: `crates/livebrainstorm-core/src/prompt.rs` (test helpers `claude()` and `api()` only)
- Modify: `crates/livebrainstorm-core/tests/notes_live.rs:21`, `crates/livebrainstorm-core/tests/brief_live.rs:23`
- Modify: `src-tauri/src/provider.rs` (constructors only, so the shell compiles)

**Interfaces:**
- Consumes: `settings::Effort` and `Effort::as_str` (Task 1).
- Produces:
  - `llm::Secret`: `Secret::new(impl Into<String>)`, `fn expose(&self) -> &str`, `Debug` prints `Secret(***)`.
  - `llm::ClaudeCli { binary: PathBuf, model: String, effort: Option<Effort>, api_key: Option<Secret>, path_env: Option<String> }` and `ClaudeCli::new(binary: PathBuf, model: impl Into<String>, path_env: Option<String>) -> Self`, with `effort` and `api_key` set to `None`.
  - `llm::AnthropicApi { api_key: Secret, model: String, effort: Option<Effort>, http: reqwest::Client }`.

- [ ] **Step 1: Write the failing tests**

At the end of `crates/livebrainstorm-core/src/llm/mod.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints() {
        let s = Secret::new("sk-ant-api03-abc");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(s.expose(), "sk-ant-api03-abc");
    }
}
```

In `crates/livebrainstorm-core/src/llm/claude_cli.rs`, inside `mod tests`, replace the whole `an_old_cli_gets_an_update_hint` test with the tests below. Then change every existing `args("…", &request(…))` / `args("…", &req)` call in that module to pass `None` as the new second argument: `analysis_runs_with_no_tools_and_no_workspace_config`, `brief_explorers_only_get_read_only_tools`, and both calls in `a_turn_limit_overrides_the_default`, for example `args("haiku", None, &request(false))`.

```rust
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
```

In `crates/livebrainstorm-core/src/llm/anthropic_api.rs`, add at the end:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ChatRequest {
        ChatRequest { system: "sys".into(), user: "u".into(), max_tokens: 900, cwd: None, allow_repo_tools: false, max_turns: None }
    }

    #[test]
    fn effort_goes_in_output_config_only_when_set() {
        let plain = body("claude-haiku-5-5", None, &request());
        assert!(plain.get("output_config").is_none(), "Default sends exactly the body sent before effort existed");
        assert_eq!(plain["model"], "claude-haiku-5-5");
        assert_eq!(plain["system"], "sys");
        assert_eq!(plain["max_tokens"], 900);
        let low = body("claude-haiku-5-5", Some(Effort::Low), &request());
        assert_eq!(low["output_config"], json!({"effort": "low"}));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core llm::`
Expected: compile errors: `cannot find type 'Secret'`, `cannot find function 'body'`, and `args` takes 2 arguments but 3 were supplied.

- [ ] **Step 3: Add `Secret` to `llm/mod.rs`**

Below the `pub use openai_compat::OpenAiCompat;` line in `crates/livebrainstorm-core/src/llm/mod.rs`, add:

```rust
/// An API key. `Debug` never prints it, so a derived `Debug` on a provider or
/// a stray `{:?}` can't leak it into a log.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}
```

- [ ] **Step 4: Implement effort and the key in `claude_cli.rs`**

Replace the import `use super::{ChatRequest, Chunk, OnChunk};` with:

```rust
use super::{ChatRequest, Chunk, OnChunk, Secret};
use crate::settings::Effort;
```

Replace the `ClaudeCli` struct with:

```rust
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
```

Replace the `fn args(model: &str, req: &ChatRequest) -> Vec<String> {` signature and its base list, up to and including `.to_vec();`, with the code below. Leave the `if req.allow_repo_tools { … } else { … }` and `a` lines that follow it unchanged.

```rust
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
```

At the top of `impl ClaudeCli`, add the constructor and the key in `command()`. Replace the existing `fn command(&self) -> Command { … }` with:

```rust
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
```

In `stream`, change `cmd.args(args(&self.model, req));` to:

```rust
        cmd.args(args(&self.model, self.effort, req));
```

Replace `fn cli_error` with:

```rust
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
```

- [ ] **Step 5: Implement effort in `anthropic_api.rs`**

Replace the import `use super::{sse, ChatRequest, Chunk, OnChunk};` with:

```rust
use super::{sse, ChatRequest, Chunk, OnChunk, Secret};
use crate::settings::Effort;
```

Replace the struct with:

```rust
#[derive(Debug, Clone)]
pub struct AnthropicApi {
    pub api_key: Secret,
    pub model: String,
    /// `output_config.effort`; None leaves the model's default.
    pub effort: Option<Effort>,
    pub http: reqwest::Client,
}

/// The request body. `output_config` only when an effort is set, so "Default"
/// sends exactly what LiveBrainstorm sent before effort existed.
fn body(model: &str, effort: Option<Effort>, req: &ChatRequest) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "stream": true,
        "system": req.system,
        "messages": [{ "role": "user", "content": req.user }],
    });
    if let Some(e) = effort {
        body["output_config"] = json!({ "effort": e.as_str() });
    }
    body
}
```

In `stream`, delete the `let body = json!({ … });` statement and change the request lines to:

```rust
        let resp = self
            .http
            .post(ENDPOINT)
            .header("x-api-key", self.api_key.expose())
            .header("anthropic-version", API_VERSION)
            .json(&body(&self.model, self.effort, req))
            .send()
            .await?;
```

- [ ] **Step 6: Update every constructor**

In `crates/livebrainstorm-core/src/prompt.rs` (test module), change the import line `use crate::llm::{AnthropicApi, ClaudeCli};` to `use crate::llm::{AnthropicApi, ClaudeCli, Secret};`, and the two helpers to:

```rust
    fn claude() -> Provider {
        Provider::ClaudeCli(ClaudeCli::new("claude".into(), "sonnet", None))
    }

    fn api() -> Provider {
        Provider::AnthropicApi(AnthropicApi { api_key: Secret::new("k"), model: "m".into(), effort: None, http: reqwest::Client::new() })
    }
```

In `crates/livebrainstorm-core/tests/notes_live.rs`:

```rust
    let provider = Provider::ClaudeCli(ClaudeCli::new(binary, "haiku", path_env));
```

In `crates/livebrainstorm-core/tests/brief_live.rs`:

```rust
    let provider = Provider::ClaudeCli(ClaudeCli::new(binary, "sonnet", path_env));
```

In `src-tauri/src/provider.rs`, change `use livebrainstorm_core::llm::{AnthropicApi, ClaudeCli, OpenAiCompat, Provider};` to include `Secret`. Change the `AnthropicApi { … }` literal in `resolve` to:

```rust
            Ok(Provider::AnthropicApi(AnthropicApi {
                api_key: Secret::new(api_key),
                model: pick(fast_model, deep_model),
                effort: None,
                http: state.http.clone(),
            }))
```

Change the last line of `claude_cli` to:

```rust
    Ok(ClaudeCli::new(binary, String::new(), path_env))
```

- [ ] **Step 7: Run all core tests and checks**

Run: `cargo test -p livebrainstorm-core && cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: all tests PASS, including `a_secret_never_prints`, `errors_say_what_to_do`, `effort_is_passed_only_when_set`, `the_api_key_goes_in_the_environment_never_in_args` and `effort_goes_in_output_config_only_when_set`, plus every pre-existing test (`all_four_sections_is_todays_prompt` still passes). Clippy prints no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/livebrainstorm-core/src/llm crates/livebrainstorm-core/src/prompt.rs crates/livebrainstorm-core/tests src-tauri/src/provider.rs
git commit -m "feat(core): --effort and output_config.effort; Claude Code API key via the environment

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Shell: resolve passes the tier's effort and the key

**Files:**
- Modify: `src-tauri/src/provider.rs`

**Interfaces:**
- Consumes:
  - `ProviderSettings` fields and `ClaudeAuth` (Task 1)
  - `ClaudeCli.effort` / `.api_key`, `AnthropicApi.effort`, `Secret::new` (Task 2)
  - `secrets::api_key() -> Result<Option<String>>` (existing)
- Produces: `provider::resolve(app, tier)` returns providers carrying the tier's effort. With `ClaudeAuth::ApiKey` it also carries the key, or fails with the missing-key Precondition.

`src-tauri` has no tests (project rule). This task is verified by `cargo check`, clippy and the manual check below.

- [ ] **Step 1: Replace the first two arms of `resolve`**

In `src-tauri/src/provider.rs`, change the settings import to `use livebrainstorm_core::settings::{ClaudeAuth, ProviderSettings, Tier};`. Replace the whole `pub async fn resolve` with:

```rust
/// The fast or the deep half of a per-tier setting.
fn pick<T>(tier: Tier, fast: T, deep: T) -> T {
    if tier == Tier::Fast {
        fast
    } else {
        deep
    }
}

pub async fn resolve(app: &AppHandle, tier: Tier) -> Result<Provider> {
    let state = app.state::<AppState>();

    match state.settings().provider {
        ProviderSettings::ClaudeCli { binary_path, fast_model, deep_model, fast_effort, deep_effort, auth } => {
            let mut cli = claude_cli(binary_path).await?;
            cli.model = pick(tier, fast_model, deep_model);
            cli.effort = pick(tier, fast_effort, deep_effort);
            if auth == ClaudeAuth::ApiKey {
                let key = secrets::api_key()?.ok_or_else(|| {
                    Error::Precondition(
                        "Add your Anthropic API key in Settings, or switch Claude Code back to your login.".into(),
                    )
                })?;
                cli.api_key = Some(Secret::new(key));
            }
            Ok(Provider::ClaudeCli(cli))
        }
        ProviderSettings::AnthropicApi { fast_model, deep_model, fast_effort, deep_effort } => {
            let api_key = secrets::api_key()?
                .ok_or_else(|| Error::Precondition("Add your Anthropic API key in Settings.".into()))?;
            Ok(Provider::AnthropicApi(AnthropicApi {
                api_key: Secret::new(api_key),
                model: pick(tier, fast_model, deep_model),
                effort: pick(tier, fast_effort, deep_effort),
                http: state.http.clone(),
            }))
        }
        ProviderSettings::Local { model_id } => {
            let (base_url, context_tokens) = ensure_local(app, &model_id).await?;
            Ok(Provider::OpenAiCompat(OpenAiCompat {
                base_url,
                model: model_id,
                context_tokens,
                http: state.http.clone(),
            }))
        }
    }
}
```

- [ ] **Step 2: Check, lint, and run the core tests**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: compiles, no clippy warnings, all core tests PASS.

- [ ] **Step 3: Live check that the real models accept `--effort` (one small call on the user's subscription; ask the user first)**

Run from the scratchpad directory, not the repo:

```bash
echo "hi" | claude -p --output-format stream-json --verbose --restricted --strict-mcp-config \
  --model haiku --effort low --max-turns 1 --tools "" --system-prompt "Reply with OK." \
  | grep -o '"type":"result".*"result":"[^"]*"'
```

Expected: `"is_error":false` and `"result":"OK"` (or similar). Repeat with `--model sonnet --effort high`. If either model rejects effort, report the exact error. Don't change any code. The spec's Risks section already covers this: the user picks Default.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/provider.rs
git commit -m "feat: resolve passes each tier's effort and Claude Code's API key

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Frontend: types, model and effort rules, and the label

**Files:**
- Modify: `src/lib/ipc.ts`
- Create: `src/lib/providerOptions.ts`
- Test: `src/lib/providerOptions.test.ts`
- Modify: `src/App.tsx` (use the shared `providerLabel`)
- Modify: `src/components/SettingsDialog.tsx` (`DEFAULTS` and the shared `CLAUDE_ALIASES` only)

**Interfaces:**
- Consumes: the JSON shape from Task 1 (`fastEffort`, `deepEffort`, `auth`).
- Produces:
  - `ipc.ts`: `type Effort = "low" | "medium" | "high" | "xhigh" | "max"`, `type ClaudeAuth = "login" | "apiKey"`, and the widened `ProviderSettings`.
  - `providerOptions.ts`:
    - `CLAUDE_ALIASES: string[]`
    - `EFFORT_OPTIONS: { value: Effort | null; label: string }[]`
    - `isCustomModel(model: string): boolean`
    - `checkModels(p: ProviderSettings): { provider: ProviderSettings; error: string | null }`
    - `providerLabel(s: Settings | null): string`

- [ ] **Step 1: Write the failing tests**

Create `src/lib/providerOptions.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { ProviderSettings, Settings } from "./ipc";
import { CLAUDE_ALIASES, EFFORT_OPTIONS, checkModels, isCustomModel, providerLabel } from "./providerOptions";

const cli: Extract<ProviderSettings, { kind: "claudeCli" }> = {
  kind: "claudeCli",
  binaryPath: null,
  fastModel: "haiku",
  deepModel: "sonnet",
  fastEffort: null,
  deepEffort: null,
  auth: "login",
};

const settings = (provider: ProviderSettings): Settings => ({
  provider,
  debounceMs: 1200,
  jev: { gate: true, rerank: true, check: true, model: "jev-latest" },
  agents: { maxAgents: 4, parallelBrief: false, parallelNotes: false },
});

describe("models", () => {
  it("offers fable and treats anything else as custom", () => {
    expect(CLAUDE_ALIASES).toEqual(["haiku", "sonnet", "opus", "fable"]);
    expect(isCustomModel("fable")).toBe(false);
    expect(isCustomModel("claude-opus-5-5")).toBe(true);
  });

  it("trims model names before saving", () => {
    const { provider, error } = checkModels({ ...cli, deepModel: "  claude-opus-5-5 " });
    expect(error).toBeNull();
    expect(provider).toEqual({ ...cli, deepModel: "claude-opus-5-5" });
  });

  it("names the tier whose model is blank", () => {
    expect(checkModels({ ...cli, fastModel: "  " }).error).toBe("Enter a model name for While you type");
    expect(checkModels({ kind: "anthropicApi", fastModel: "a", deepModel: "", fastEffort: null, deepEffort: null }).error).toBe(
      "Enter a model name for Project brief",
    );
    expect(checkModels({ kind: "local", modelId: "qwen3-4b-instruct" }).error).toBeNull();
  });
});

describe("effort", () => {
  it("lists Default first, then the CLI's levels in order", () => {
    expect(EFFORT_OPTIONS.map((o) => o.value)).toEqual([null, "low", "medium", "high", "xhigh", "max"]);
    expect(EFFORT_OPTIONS.map((o) => o.label)).toEqual(["Default", "Low", "Medium", "High", "Extra high", "Max"]);
  });
});

describe("providerLabel", () => {
  it("says when Claude Code uses an API key", () => {
    expect(providerLabel(settings(cli))).toBe("Claude Code, haiku");
    expect(providerLabel(settings({ ...cli, auth: "apiKey" }))).toBe("Claude Code (API key), haiku");
    expect(providerLabel(settings({ kind: "local", modelId: "qwen3-4b-instruct" }))).toBe("Local, qwen3-4b-instruct");
    expect(providerLabel(null)).toBe("Settings");
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/providerOptions.test.ts`
Expected: FAIL, `Failed to resolve import "./providerOptions"`.

- [ ] **Step 3: Widen the IPC types**

In `src/lib/ipc.ts`, replace the `ProviderSettings` type with:

```ts
/** Mirrors `Effort` in settings.rs; null means the model's default. */
export type Effort = "low" | "medium" | "high" | "xhigh" | "max";

/** Mirrors `ClaudeAuth`: Claude Code's own login, or the keychain API key. */
export type ClaudeAuth = "login" | "apiKey";

export type ProviderSettings =
  | {
      kind: "claudeCli";
      binaryPath: string | null;
      fastModel: string;
      deepModel: string;
      fastEffort: Effort | null;
      deepEffort: Effort | null;
      auth: ClaudeAuth;
    }
  | { kind: "anthropicApi"; fastModel: string; deepModel: string; fastEffort: Effort | null; deepEffort: Effort | null }
  | { kind: "local"; modelId: string };
```

- [ ] **Step 4: Create `src/lib/providerOptions.ts`**

```ts
// Model, effort and label rules for the provider settings, kept pure so they're
// testable. Mirrors `Effort` / `ClaudeAuth` in crates/livebrainstorm-core/src/settings.rs.
import type { Effort, ProviderSettings, Settings } from "./ipc";

/** Claude Code accepts these aliases, which track the latest models. */
export const CLAUDE_ALIASES = ["haiku", "sonnet", "opus", "fable"];

export const EFFORT_OPTIONS: { value: Effort | null; label: string }[] = [
  { value: null, label: "Default" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "Extra high" },
  { value: "max", label: "Max" },
];

/** A saved model that isn't an alias opens the picker on "Custom…". */
export const isCustomModel = (model: string) => !CLAUDE_ALIASES.includes(model);

const TIERS = [
  ["fastModel", "While you type"],
  ["deepModel", "Project brief"],
] as const;

/** Trims model names; a blank one blocks saving with a message naming its tier. */
export function checkModels(p: ProviderSettings): { provider: ProviderSettings; error: string | null } {
  if (p.kind === "local") return { provider: p, error: null };
  const provider = { ...p, fastModel: p.fastModel.trim(), deepModel: p.deepModel.trim() };
  const blank = TIERS.find(([field]) => !provider[field]);
  return { provider, error: blank ? `Enter a model name for ${blank[1]}` : null };
}

export function providerLabel(s: Settings | null): string {
  if (!s) return "Settings";
  const p = s.provider;
  if (p.kind === "claudeCli") return `Claude Code${p.auth === "apiKey" ? " (API key)" : ""}, ${p.fastModel}`;
  if (p.kind === "anthropicApi") return `API, ${p.fastModel}`;
  return `Local, ${p.modelId}`;
}
```

- [ ] **Step 5: Use the shared label and defaults**

In `src/App.tsx`, delete the local `function providerLabel(…) { … }` and add the import:

```ts
import { providerLabel } from "./lib/providerOptions";
```

In `src/components/SettingsDialog.tsx`, delete the line `const CLAUDE_ALIASES = ["haiku", "sonnet", "opus"];`, add `import { CLAUDE_ALIASES } from "../lib/providerOptions";`, and replace the first two `DEFAULTS` entries with:

```ts
  claudeCli: { kind: "claudeCli", binaryPath: null, fastModel: "haiku", deepModel: "sonnet", fastEffort: null, deepEffort: null, auth: "login" },
  anthropicApi: { kind: "anthropicApi", fastModel: "claude-haiku-5-5", deepModel: "claude-sonnet-5-5", fastEffort: null, deepEffort: null },
```

- [ ] **Step 6: Run the tests and the typecheck**

Run: `npx vitest run src/lib/providerOptions.test.ts && npm test && npm run typecheck`
Expected: the five new tests PASS, the whole vitest suite passes, and `tsc --noEmit` reports no errors.

- [ ] **Step 7: Commit**

```bash
git add src/lib/ipc.ts src/lib/providerOptions.ts src/lib/providerOptions.test.ts src/App.tsx src/components/SettingsDialog.tsx
git commit -m "feat(ui): effort and sign-in types, model rules and the API-key label

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Settings UI for sign-in, model and effort, plus the docs

**Files:**
- Modify: `src/components/SettingsDialog.tsx`
- Modify: `README.md`, `AGENTS.md`

**Interfaces:**
- Consumes:
  - `CLAUDE_ALIASES`, `EFFORT_OPTIONS`, `isCustomModel`, `checkModels` (Task 4)
  - `api.hasApiKey`, `api.setApiKey` (existing)
  - `type Effort` (Task 4)
- Produces:
  - In `SettingsDialog.tsx`: `ApiKeyField`, `ModelPicker`, `EffortSelect`, and the reworked `ClaudeCliFields` and `ApiFields`.
  - `save()` validates models with `checkModels`.

- [ ] **Step 1: Imports**

In `src/components/SettingsDialog.tsx`, add `type Effort,` to the `../lib/ipc` import list. Change the providerOptions import to:

```ts
import { CLAUDE_ALIASES, EFFORT_OPTIONS, checkModels, isCustomModel } from "../lib/providerOptions";
```

- [ ] **Step 2: Validate models on save**

Replace `const save = async () => { … };` in `SettingsDialog` with:

```ts
  const save = async () => {
    const { provider, error: modelError } = checkModels(draft.provider);
    if (modelError) return setError(modelError);
    setSaving(true);
    setError(null);
    try {
      await onSave({ ...draft, provider, agents: clampedAgents(draft.agents) });
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };
```

- [ ] **Step 3: Replace `ClaudeCliFields`, `ModelSelect` and `ApiFields`**

Delete the existing `ClaudeCliFields`, `ModelSelect` and `ApiFields` functions. Put the following in their place, between `ProviderOption` and `LocalFields`:

```tsx
function ClaudeCliFields({ p, patch }: { p: Extract<ProviderSettings, { kind: "claudeCli" }>; patch: (f: Partial<ProviderSettings>) => void }) {
  const [check, setCheck] = useState<string | null>(null);
  const run = async () => {
    setCheck("Checking…");
    try {
      setCheck(`Found Claude Code ${await api.checkClaude(p.binaryPath)}`);
    } catch (e) {
      setCheck(errorMessage(e));
    }
  };
  return (
    <div className="grid gap-3">
      <Field label="Claude Code location (leave empty to find it automatically)">
        <div className="flex gap-2">
          <input className={inputClass} value={p.binaryPath ?? ""} placeholder="claude" onChange={(e) => patch({ binaryPath: e.target.value || null })} />
          <Button className="shrink-0" onClick={run}>Check</Button>
        </div>
        {check && <small className="text-xs text-muted">{check}</small>}
      </Field>
      <fieldset className="grid gap-1.5">
        <legend className="mb-1.5 text-xs text-muted">Sign in with</legend>
        <div className="flex flex-wrap gap-x-5 gap-y-1.5 text-[13px]">
          <label className="flex cursor-pointer items-center gap-2">
            <input type="radio" name="claude-auth" className="accent-accent" checked={p.auth === "login"} onChange={() => patch({ auth: "login" })} />
            My Claude Code login
          </label>
          <label className="flex cursor-pointer items-center gap-2">
            <input type="radio" name="claude-auth" className="accent-accent" checked={p.auth === "apiKey"} onChange={() => patch({ auth: "apiKey" })} />
            My Anthropic API key
          </label>
        </div>
      </fieldset>
      {p.auth === "apiKey" && <ApiKeyField note="Usage is billed to your API account, not your Claude subscription." />}
      <div className="grid grid-cols-2 gap-3">
        <div className="grid content-start gap-2">
          <ModelPicker label="While you type" value={p.fastModel} onChange={(fastModel) => patch({ fastModel })} />
          <EffortSelect label="Effort while you type" value={p.fastEffort} onChange={(fastEffort) => patch({ fastEffort })} />
        </div>
        <div className="grid content-start gap-2">
          <ModelPicker label="Project brief" value={p.deepModel} onChange={(deepModel) => patch({ deepModel })} />
          <EffortSelect label="Effort for the brief" value={p.deepEffort} onChange={(deepEffort) => patch({ deepEffort })} />
        </div>
      </div>
    </div>
  );
}

/** An alias, or "Custom…" with a text field for a full model id. */
function ModelPicker(props: { label: string; value: string; onChange: (v: string) => void }) {
  const [custom, setCustom] = useState(isCustomModel(props.value));
  return (
    <Field label={props.label}>
      <select
        className={inputClass}
        value={custom ? "custom" : props.value}
        onChange={(e) => {
          const v = e.target.value;
          setCustom(v === "custom");
          props.onChange(v === "custom" ? "" : v);
        }}
      >
        {CLAUDE_ALIASES.map((o) => (
          <option key={o} value={o}>
            {o[0].toUpperCase() + o.slice(1)}
          </option>
        ))}
        <option value="custom">Custom…</option>
      </select>
      {custom && (
        <input
          className={inputClass}
          value={props.value}
          placeholder="claude-opus-5-5"
          aria-label={`${props.label}: model id`}
          onChange={(e) => props.onChange(e.target.value)}
        />
      )}
    </Field>
  );
}

function EffortSelect(props: { label: string; value: Effort | null; onChange: (v: Effort | null) => void }) {
  return (
    <Field label={props.label}>
      <select className={inputClass} value={props.value ?? ""} onChange={(e) => props.onChange((e.target.value || null) as Effort | null)}>
        {EFFORT_OPTIONS.map((o) => (
          <option key={o.label} value={o.value ?? ""}>
            {o.label}
          </option>
        ))}
      </select>
    </Field>
  );
}

/** The Anthropic API key in the system keychain; shared by both Claude providers. */
function ApiKeyField({ note: extra }: { note?: string }) {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [key, setKey] = useState("");
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => void api.hasApiKey().then(setHasKey), []);

  const saveKey = async () => {
    try {
      await api.setApiKey(key);
      setHasKey(!!key.trim());
      setKey("");
      setNote(key.trim() ? "Key saved to your system keychain." : "Key removed.");
    } catch (e) {
      setNote(errorMessage(e));
    }
  };

  return (
    <Field label={`Anthropic API key${hasKey ? " (saved in your keychain)" : ""}`}>
      <div className="flex gap-2">
        <input
          type="password"
          className={inputClass}
          value={key}
          placeholder={hasKey ? "Paste a new key to replace it" : "sk-ant-…"}
          onChange={(e) => setKey(e.target.value)}
        />
        <Button className="shrink-0" onClick={saveKey} disabled={!key.trim() && !hasKey}>
          {key.trim() || !hasKey ? "Save key" : "Remove key"}
        </Button>
      </div>
      {note ? <small className="text-xs text-muted">{note}</small> : hasKey === false && <small className="text-xs text-muted">No key saved yet.</small>}
      {extra && <small className="text-xs text-muted">{extra}</small>}
    </Field>
  );
}

function ApiFields({ p, patch }: { p: Extract<ProviderSettings, { kind: "anthropicApi" }>; patch: (f: Partial<ProviderSettings>) => void }) {
  return (
    <div className="grid gap-3">
      <ApiKeyField />
      <div className="grid grid-cols-2 gap-3">
        <div className="grid content-start gap-2">
          <Field label="While you type">
            <input className={inputClass} value={p.fastModel} onChange={(e) => patch({ fastModel: e.target.value })} />
          </Field>
          <EffortSelect label="Effort while you type" value={p.fastEffort} onChange={(fastEffort) => patch({ fastEffort })} />
        </div>
        <div className="grid content-start gap-2">
          <Field label="Project brief">
            <input className={inputClass} value={p.deepModel} onChange={(e) => patch({ deepModel: e.target.value })} />
          </Field>
          <EffortSelect label="Effort for the brief" value={p.deepEffort} onChange={(deepEffort) => patch({ deepEffort })} />
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Typecheck and run the tests**

Run: `npm run typecheck && npm test`
Expected: no type errors, and every vitest test passes.

- [ ] **Step 5: Update the README**

In `README.md`, replace the provider table's first row with:

```markdown
| **Claude subscription** (default) | Install [Claude Code](https://docs.claude.com/en/docs/claude-code/overview), run `claude` once to log in, or sign it in with your own API key (below) | Your own machine, your own subscription |
```

Replace the paragraph under `### Requirements for the Claude subscription option` with:

```markdown
LiveBrainstorm runs Claude Code in restricted mode (`--restricted`, so a project you open can't run its own hooks or servers). That mode ignores every Claude Code settings file, including your own `~/.claude/settings.json`, so authentication must come from your normal Claude Code login, from environment variables, or from **Sign in with: My Anthropic API key** in Settings. That option passes the key saved in your system keychain to Claude Code as `ANTHROPIC_API_KEY`, and usage is billed to your API account instead of your subscription. Use it if you relied on `apiKeyHelper`. Setups that depend on an `env` block in settings (Bedrock, Vertex, a proxy base URL) need those exported in the environment instead. It also needs a Claude Code version that supports `--restricted`; if yours is older, update it with `claude update`.

Both Claude options have a model and an effort level for the notes (*While you type*) and for the brief (*Project brief*). Claude Code takes an alias (`haiku`, `sonnet`, `opus`, `fable`) or any full model id through *Custom…*. Effort *Default* leaves the model's own level; lower effort usually means faster notes.
```

- [ ] **Step 6: Update AGENTS.md**

In `AGENTS.md`, in the `**`ClaudeCli`**` bullet under "Providers", replace the sentence `Known cost: `--restricted` also ignores the user's own settings files, so auth via `apiKeyHelper` or settings `env` no longer works (documented in the README).` with:

```markdown
Known cost: `--restricted` also ignores the user's own settings files, so auth via `apiKeyHelper` or settings `env` no longer works (documented in the README). The replacement is `ProviderSettings::ClaudeCli { auth: apiKey }`: the keychain key reaches the child only as `ANTHROPIC_API_KEY` in its environment, never in argv, which any process can list. `--effort` is passed only when a level is set, per tier (`Effort`; `None` means the model's default).
```

In the `**`AnthropicApi` / `OpenAiCompat`**` bullet, append:

```markdown
 `AnthropicApi` sends `output_config.effort` only when a level is set.
```

In "Persistence", replace the `- API key:` line with:

```markdown
- API key: OS keychain (service `app.livebrainstorm.desktop`, account `anthropic-api-key`), shared by the Anthropic API provider and Claude Code's API-key sign-in. Never written to settings.json, never sent to the webview. In Rust it travels as `llm::Secret`, whose `Debug` prints `Secret(***)`.
```

In "Cross-language contracts", replace the `- **Settings defaults:**` line with:

```markdown
- **Settings defaults:** `Settings::default()` in `settings.rs` ↔ `DEFAULTS` in `src/components/SettingsDialog.tsx`, including `fastEffort` / `deepEffort` (`null`) and `auth` (`"login"`). `Effort` serializes lowercase, `ClaudeAuth` camelCase (↔ `Effort` / `ClaudeAuth` in `ipc.ts`). Both are read through `settings::lenient`, so an unknown value falls back to its default instead of resetting the whole file.
```

In the `- **New provider** touches:` line, change `and `providerLabel` in `App.tsx`` to `and `providerLabel` in `src/lib/providerOptions.ts``.

- [ ] **Step 7: Manual check in the running app (the user runs `npm run tauri dev`)**

Ask the user to run `npm run tauri dev` and check:
1. Settings → Claude subscription: the "Sign in with" choice shows. The model pickers list Haiku, Sonnet, Opus, Fable and Custom…. Each tier has an effort picker that starts at Default.
2. Choose "My Anthropic API key" with no key saved: the field says "No key saved yet." Save the settings and type a draft: the notes pane shows "Add your Anthropic API key in Settings, or switch Claude Code back to your login."
3. Save a deliberately wrong key (`sk-ant-wrong`): the notes pane shows "Claude Code rejected the Anthropic API key. …". The top bar reads "Claude Code (API key), haiku".
4. Remove the key, switch back to "My Claude Code login", and set "Effort while you type" to Low: notes still arrive. While they stream, `ps -o args= -p $(pgrep -n claude)` shows `--effort low` and no `sk-ant`.
5. Pick Custom… for Project brief and leave it blank: Save shows "Enter a model name for Project brief" and the dialog stays open. Type ` claude-sonnet-5-5 ` (with spaces) and save, then reopen Settings: it shows Custom with `claude-sonnet-5-5`, trimmed.
6. Anthropic API key provider: the key field and the two effort pickers show. Settings saved before this branch still load.

- [ ] **Step 8: Final checks and commit**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets && npm run typecheck && npm test`
Expected: everything passes, with no clippy warnings.

```bash
git add src/components/SettingsDialog.tsx README.md AGENTS.md
git commit -m "feat(ui): sign in Claude Code with an API key; any model; effort per tier

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
