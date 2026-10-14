# Phase 4: Usage ledger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every model call (each notes lane, each brief explorer and the brief merge) is counted in a local usage ledger. The sidebar footer shows today's tokens and estimated cost, and a 30-day panel breaks them down by day, project and kind.

**Architecture:**
- **Core (`livebrainstorm-core`):**
  - `llm::Chunk` gains `Usage(Usage)`. Each provider emits it at the end of any call it has usage for, failed CLI runs included. `Provider::model()` returns the model the provider was asked for.
  - New module `usage.rs`: the price table (`PRICES`, `price`), the `UsageLedger` (hourly UTC buckets in `<app_data>/usage.json`), the `Meter` that records exact usage or an estimate, and `metered`, which wraps one `provider.stream` call in a `Meter`.
- **Shell:** `AppState.usage`, `usage::metered` around every notes lane, explorer and merge call, the `usage_report` command, a background task that emits `usage-changed` (at most once a second) and saves the ledger every 5 s, and a final save on exit.
- **Frontend:** pure helpers in `src/lib/usage.ts` (local-day grouping, totals, formatting), `UsageFooter` in the sidebar footer, and `UsageDialog`, a native `<dialog>`.

**Tech Stack:** Rust 2021 (serde, serde_json, tokio, tempfile for tests), Tauri v2, React 19 + Tailwind v4, vitest 3.

**Spec:** `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md`, Section 5 ("Usage ledger"), plus the rows of Section 6 that mention the ledger, pre-implementation check 3, and "Documentation updates".

**Prerequisites:** Phases 1, 2 and 3 are complete on branch `feat/tabs-windows-usage`, and this plan's work happens on that same branch (`git switch feat/tabs-windows-usage`). This plan relies on what they leave behind:
- **Phase 1:**
  - `crates/livebrainstorm-core/src/store.rs` with `write_atomic(path: &Path, bytes: &[u8]) -> Result<()>`, `set_aside(path: &Path) -> Option<PathBuf>`, `enum Loaded<T> { Missing, Parsed(T), Corrupt { kept: Option<PathBuf> } }` and `load_json<T: DeserializeOwned>(path: &Path) -> Result<Loaded<T>>` (Err only for read errors other than NotFound, and those files are left in place; a parse failure renames the file to the first free `<name>.bad`, `<name>.bad.1`, …).
  - `AppState` (in `src-tauri/src/state.rs`) built by `AppState::new(data_dir: &Path, config_dir: &Path)`, with the fields `projects`, `analyses`, `brief_tasks` and `sessions`.
  - `analyze(tab_id, draft, previous_draft, instructions, context, on_event)` in `src-tauri/src/commands.rs`. The command body binds the tab's root before spawning, `let root = app.state::<AppState>().sessions.open_tab_root(&tab_id)?;` (`root: Option<String>`), and moves it into the spawned task, which binds `let state = task_app.state::<AppState>();`. The task then runs today's lane loop: one `async move` block per lane, each with a `sink` closure that `match`es `Chunk::Text` / `Chunk::Activity` and a call `provider.stream(&req, &mut sink).await`.
  - `build_brief(root, on_event)`, whose spawned task holds `index: Arc<ProjectIndex>` (from the registry entry for `root`) and makes today's three kinds of call: the single explorer `provider.stream(&req, &mut on_chunk(&on_event))`, the explorer loop (`sink` with `if let Chunk::Activity`), and the merge `provider.stream(&req, &mut on_chunk(&on_event))`. The `on_chunk(ch)` helper still `match`es on `Chunk`.
  - `src-tauri/src/events.rs` (declared as `mod events;` in `src-tauri/src/lib.rs`) holding the event-name constants, and `src/lib/events.ts` with typed `listen` wrappers that each return `Promise<UnlistenFn>`.
- **Phase 2:** `src/components/Sidebar.tsx` takes `footer?: ReactNode` and renders it above the provider label. `src/App.tsx` renders `<Sidebar … />` and still renders `SettingsDialog` behind a `showSettings` flag.
- **Phase 3:** `src-tauri/src/lib.rs`'s `.run(|app, event| …)` closure still has a `RunEvent::Exit` arm (today it stops the local server), and `setup` still calls `app.manage(AppState::new(…))` before it builds the windows.

If any of these names differ in the code you find, use the code's names and keep this plan's behaviour.

## Global Constraints

- **What is counted:** every model call: each notes lane, each brief explorer, and the brief merge. Jev/TypeSafe calls are not counted. Kinds are `notes` and `brief` (serde lowercase).
- **Price table** (`usage::PRICES`, list $ per million tokens, from the Claude API reference model table cached 2026-10-06):

  | Model id | Input | Output |
  |---|---|---|
  | `claude-fable-5-1`, `claude-fable-5` | 10.00 | 50.00 |
  | `claude-opus-5-5` | 4.00 | 20.00 |
  | `claude-opus-5`, `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6` | 5.00 | 25.00 |
  | `claude-sonnet-5-5`, `claude-sonnet-5` | 2.00 | 10.00 |
  | `claude-sonnet-4-6` | 3.00 | 15.00 |
  | `claude-haiku-5-5` | 0.10 | 0.50 |
  | `claude-haiku-4-5` | 1.00 | 5.00 |

- **Price lookup order:** exact id first. Otherwise the CLI aliases map to the newest id of their family: `fable` → `claude-fable-5-1`, `opus` → `claude-opus-5-5`, `sonnet` → `claude-sonnet-5-5`, `haiku` → `claude-haiku-5-5`. Otherwise a local catalog model id (`local::catalog::find`) costs $0. Otherwise the price is unknown: tokens are still counted, the run counts as unpriced, and its cost shows as "—", never $0.
- **Haiku 5.5** uses the lower rate only ($0.10 / $0.50). Cache writes and cache reads are priced at the input rate.
- **Sources of usage:**
  - Claude Code: `result.modelUsage[model]` (`inputTokens`, `cacheCreationInputTokens`, `cacheReadInputTokens`, `outputTokens`, `costUSD`), one `Usage` per model. Without `modelUsage`: the `usage` totals with `total_cost_usd`, under the requested model. A record whose event reported no cost is priced from `PRICES` (the lookup order below). A failed `result` that carries usage still emits it.
  - Anthropic API: `message_start.message.usage` (input and cache fields) plus the last `message_delta.usage.output_tokens`, priced with `PRICES`.
  - Local (OpenAI-compatible): the final chunk's `usage` (`prompt_tokens`, `completion_tokens`), requested with `stream_options: { include_usage: true }`, at $0.
- **The `Meter` table:**

  | How the call ended | What is recorded |
  |---|---|
  | A `Chunk::Usage` arrived | That usage, exactly |
  | `stream` returned `Ok` without usage | An estimate |
  | `stream` returned `Err` without usage | Nothing |
  | Dropped before `stream` returned | An estimate, from `Drop` |

- **An estimate:** tokens = `ceil(chars / 4)`, where chars are Unicode scalars (not bytes). Sent = `system` + `user`; received = every `Chunk::Text`. All sent tokens are input (no cache), priced with `price(provider.model())`, and the run is marked `estimated`.
- **The meter is created immediately before `provider.stream`, after the pre-phase**, so a run aborted during the Jev pre-phase records nothing. `Drop` writes through the ledger's `std` mutex and never awaits.
- **Ledger storage:** `<app_data>/usage.json`, `{ "version": 1, "buckets": [...] }`. `hour = unix_secs / 3600` (UTC). Bucket key `(hour, root, kind, model)`. In memory; written atomically at most every 5 s (when changed) and on `RunEvent::Exit`. Buckets older than 90 days are dropped on load. A corrupt file is kept as `.bad` and the ledger starts empty. A file that exists but can't be read is never overwritten.
- **Events:** `usage-changed` (no payload), emitted at most once a second after a record.
- **Ledger failures** are one `eprintln!` and are otherwise ignored. The ledger never affects an analysis.
- **Prompts and `claude` arguments stay byte-identical.** This phase adds no CLI flag. The existing `prompt::tests` and `claude_cli::tests` must stay green unchanged.
- **Copy, verbatim:**
  - Footer: "Today · {tokens} tokens · {cost}", for example "Today · 182k tokens · ~$0.41". The cost has a "~" prefix when any of it comes from an estimate.
  - Token counts: "182k", "1.2M".
  - Panel labels: "incl. ~$0.03 from 12 cancelled runs", "price unknown for N runs", "Last 30 days", "No usage in the last 30 days."
  - The note at the bottom of the panel: "On a Claude subscription you aren't billed per token. This is what the same usage would cost on the API."
- **Grouping by day happens in the UI** (`src/lib/usage.ts`), from each bucket's start hour on the user's clock. The panel has one row per local day for every one of the last 30 days, today first, under a 30-day total. A day with no runs still gets its row, with "—" in the tokens, cost and runs columns.
- **Process:**
  - Branch `feat/tabs-windows-usage`. One commit per task. Every commit message ends with:
    ```
    Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
    Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47
    ```
  - Never run `cargo fmt` across the repo; match the hand-wrapped ~120-column style.
  - `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests: Rust tests go in `livebrainstorm-core`, and shell changes are verified with `cargo check -p livebrainstorm` and clippy.
  - `std` locks are never held across `.await`. Futures awaited inside the shell's spawned tasks use `for` loops, not closures over borrowed arguments (`.map(|(i, &x)| …)`), which trip rustc's Send check.
  - UI uses the semantic colour tokens (`text-muted`, `text-fault`, `border-line`, …) and the `ui.tsx` primitives (`Button`, `cx`, `Eyebrow`). No `alert`, `confirm` or `prompt`.
  - The agent shell has Node 20.11, so `npm run tauri dev` can't run there. Manual checks are a checklist for the user.

## Review Focus

1. **A keystroke aborts a run mid-stream, or during the Jev pre-phase.** Expected: a run cut short mid-stream adds exactly one estimated run, never two, and a run cancelled before its request was sent adds nothing. Pinned in Task 6 (`a_call_dropped_mid_stream_records_one_estimate`, `a_run_cancelled_before_the_request_records_nothing`, `a_meter_dropped_mid_call_records_one_estimate`).
2. **A Claude Code run that fails but reports usage** (an error `result` after the model already ran). Expected: its usage is counted exactly, while a call that failed without usage (bad key, CLI missing) counts nothing. Pinned in Task 3 (`an_error_result_still_reports_its_usage`) and Task 6 (`reported_usage_is_recorded_exactly_once`, `a_failed_call_without_usage_records_nothing`).
3. **"Today" for a user far from UTC, including half-hour offsets.** Expected: the footer and the panel follow the user's calendar day, not UTC's, and the first local hour of the day is never missed. Pinned in Task 8 (`puts each hour on the user's calendar day, not UTC's`, `handles half-hour offsets`, `starts an hour before local midnight…`).
4. **A corrupt or unreadable `usage.json`.** Expected: startup and analyses never fail, a corrupt file survives as `usage.json.bad`, and a file that can't be read at all is never overwritten. Pinned in Task 5 (`a_corrupt_ledger_is_kept_as_bad_and_starts_empty`, `an_unreadable_ledger_is_never_overwritten`).
5. **A model with no price** (a custom model id, or a family newer than the table). Expected: its tokens are counted, the cost reads "—" and "price unknown for N runs", never "$0.00". Pinned in Task 2 (`unknown_models_have_no_price`), Task 6 (`an_estimate_for_an_unpriced_model_counts_tokens_but_no_cost`) and Task 8 (`shows a dash when no run has a price`).
6. **A month with quiet days.** Expected: the panel always lists all 30 local days, today first, and a day with no runs shows "—" in every column instead of disappearing or reading "0" and "$0.00". Pinned in Task 8 (`lists every one of the last 30 local days, today first, quiet days included`, `shows a dash in every column of a row with no runs`).

## File Structure

| File | Responsibility |
|---|---|
| `crates/livebrainstorm-core/src/llm/mod.rs` (modify) | `Usage`, `Chunk::Usage`, `Provider::model()` |
| `crates/livebrainstorm-core/src/llm/claude_cli.rs` (modify) | `result.modelUsage` → one `Usage` per model, emitted even for an error result |
| `crates/livebrainstorm-core/src/llm/anthropic_api.rs` (modify) | `message_start` / `message_delta` usage, priced from the table |
| `crates/livebrainstorm-core/src/llm/openai_compat.rs` (modify) | `stream_options.include_usage` and the final chunk's `usage`, at $0 |
| `crates/livebrainstorm-core/src/usage.rs` (create) | Prices, `UsageLedger`, `Meter`, `metered`, `sent_chars` |
| `crates/livebrainstorm-core/src/lib.rs` (modify) | `pub mod usage;` |
| `src-tauri/src/state.rs` (modify) | `AppState.usage: Arc<UsageLedger>` |
| `src-tauri/src/commands.rs` (modify) | `Usage` arms, `metered` around every call, `usage_report` |
| `src-tauri/src/events.rs` (modify) | `USAGE_CHANGED` |
| `src-tauri/src/lib.rs` (modify) | Command registration, the 1 s event / 5 s flush task, the flush on exit |
| `src/lib/ipc.ts` (modify) | `UsageKind`, `UsageBucket`, `api.usageReport` |
| `src/lib/events.ts` (modify) | `onUsageChanged` |
| `src/lib/usage.ts`, `src/lib/usage.test.ts` (create) | Pure helpers: local days, totals, formatting, labels |
| `src/components/UsageFooter.tsx` (create) | The "Today · …" line |
| `src/components/UsageDialog.tsx` (create) | The 30-day panel |
| `src/App.tsx` (modify) | Footer into `Sidebar`, the dialog, Esc guard |
| `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md` (modify) | Records pre-implementation check 3 (Task 1) |
| `AGENTS.md`, `README.md` (modify) | Phase 4 documentation |

Task order: 1 (check) → 2 (types and prices) → 3, 4 (providers) → 5 (ledger) → 6 (meter) → 7 (shell and IPC) → 8 (frontend helpers) → 9 (UI and docs).

---

### Task 1: Pre-implementation check 3: does the pinned `llama-server` report usage when streaming?

The spec asks for this check before Phase 4. Its result decides one line of Task 4 (whether `OpenAiCompat` sends `stream_options`) and one sentence of the docs in Task 9 (whether local runs are recorded exactly or as estimates). Nothing else depends on it: the parser in Task 4 reads `usage` whenever it appears, and the `Meter` (Task 6) estimates whenever it doesn't.

**Files:**
- Modify: `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md` (the "Pre-implementation checks" list, item 3)

**Interfaces:**
- Consumes: `LLAMA_CPP_BUILD` in `crates/livebrainstorm-core/src/local/runtime.rs` (today `"b11480"`); the request body `OpenAiCompat::stream` sends today (`model`, `stream: true`, `max_tokens`, `temperature: 0.4`, a system and a user message).
- Produces: one of three recorded outcomes, which Task 4 and Task 9 branch on:
  - **A:** with `stream_options.include_usage`, the reply is HTTP 200 and a `data:` chunk before `data: [DONE]` carries `"usage":{…"prompt_tokens":…,"completion_tokens":…}`.
  - **B:** HTTP 200, but no chunk carries a `usage` object (missing or `null`).
  - **C:** the request with `stream_options` fails (not HTTP 200) while the same request without it succeeds.

The check needs the network (a ~30–60 MB runtime download if the app hasn't installed one, and a ~1 MB model). If this machine has no network, stop here and ask the user to run Steps 1–2 and paste the output; never guess the outcome.

- [ ] **Step 1: Write the check script outside the repo**

Save this as `llama-usage-check.sh` in your scratchpad directory (or `$TMPDIR`), not in the repo:

```bash
#!/bin/bash
# Pre-implementation check 3: does llama-server (LLAMA_CPP_BUILD) report usage in a streamed reply?
# Run from the repo root: bash <path>/llama-usage-check.sh
set -euo pipefail
BUILD="$(grep -o 'LLAMA_CPP_BUILD: &str = "b[0-9]*"' crates/livebrainstorm-core/src/local/runtime.rs | grep -o 'b[0-9]*')"
WORK="${TMPDIR:-/tmp}/llama-usage-check"
mkdir -p "$WORK"
echo "pinned build: $BUILD"

# 1. The pinned server: the app's own install if it has one, else the release asset.
SERVER="$(find "$HOME/Library/Application Support/app.livebrainstorm.desktop/local/runtime" \
  "$HOME/.local/share/app.livebrainstorm.desktop/local/runtime" -name llama-server -type f 2>/dev/null \
  | grep "llama-$BUILD" | head -1 || true)"
if [ -z "$SERVER" ]; then
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) ASSET="llama-$BUILD-bin-macos-arm64.tar.gz" ;;
    Darwin-x86_64) ASSET="llama-$BUILD-bin-macos-x64.tar.gz" ;;
    Linux-x86_64) ASSET="llama-$BUILD-bin-ubuntu-x64.tar.gz" ;;
    Linux-aarch64) ASSET="llama-$BUILD-bin-ubuntu-arm64.tar.gz" ;;
    *) echo "unsupported platform for this script"; exit 1 ;;
  esac
  curl -fL --retry 2 -o "$WORK/$ASSET" "https://github.com/ggml-org/llama.cpp/releases/download/$BUILD/$ASSET"
  mkdir -p "$WORK/runtime" && tar -xzf "$WORK/$ASSET" -C "$WORK/runtime"
  SERVER="$(find "$WORK/runtime" -name llama-server -type f | head -1)"
fi
LIB="$(dirname "$SERVER")"
echo "server: $SERVER"
LD_LIBRARY_PATH="$LIB" "$SERVER" --version 2>&1 | head -2 || true

# 2. A tiny model (about 1 MB): enough to exercise the API, not to say anything useful.
MODEL="$WORK/stories260K.gguf"
[ -f "$MODEL" ] || curl -fL --retry 2 -o "$MODEL" \
  "https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories260K.gguf"

# 3. Start it and wait for /health.
PORT=18089
LD_LIBRARY_PATH="$LIB" "$SERVER" -m "$MODEL" --host 127.0.0.1 --port "$PORT" -c 512 >"$WORK/server.log" 2>&1 &
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 120); do
  curl -fs "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
  sleep 0.5
done
curl -fs "http://127.0.0.1:$PORT/health" >/dev/null || { echo "server never became healthy:"; tail -20 "$WORK/server.log"; exit 1; }

# 4. The body OpenAiCompat sends, with and without stream_options.
BASE='"model":"check","stream":true,"max_tokens":8,"temperature":0.4,"messages":[{"role":"system","content":"Be brief."},{"role":"user","content":"Say hi."}]'
for variant in with without; do
  if [ "$variant" = with ]; then BODY="{$BASE,\"stream_options\":{\"include_usage\":true}}"; else BODY="{$BASE}"; fi
  CODE="$(curl -sN -o "$WORK/$variant.txt" -w '%{http_code}' "http://127.0.0.1:$PORT/v1/chat/completions" \
    -H 'content-type: application/json' -d "$BODY")"
  echo "--- $variant stream_options: HTTP $CODE"
  grep -o '"usage":{[^}]*}' "$WORK/$variant.txt" | tail -1 || echo "no usage object"
  tail -c 600 "$WORK/$variant.txt"; echo
done
```

- [ ] **Step 2: Run it**

Run: `bash <scratchpad>/llama-usage-check.sh` from the repo root.
Expected: it prints the pinned build (`b11480`), the server path and version, then two blocks, `--- with stream_options: HTTP …` and `--- without stream_options: HTTP …`, each with the last `usage` object found (or `no usage object`) and the tail of the raw stream.

Classify the `with` block:
- HTTP 200 and a `"usage":{…}` with `prompt_tokens` and `completion_tokens` → **outcome A**.
- HTTP 200 and `no usage object` → **outcome B**.
- Not 200 (while the `without` block is 200) → **outcome C**.

Note too whether the `without` block carries usage (for the record; it changes nothing).

- [ ] **Step 3: Record the result in the spec**

In `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md`, under `### Pre-implementation checks`, add an indented line directly under item 3 (the one starting `3. **Phase 4:**`), filled in from Step 2. Phase 3 inserted its own block, `**Result for checks 1 and 2 (…):**`, after item 3; this line goes between item 3 and that block, so it stays part of item 3. For outcome A it reads, for example:

```markdown
   **Result (2026-10-14, b11480):** outcome A. With `stream_options.include_usage` the reply is 200 and the chunk before `[DONE]` carries `"usage":{"completion_tokens":8,"prompt_tokens":24,"total_tokens":32}`. Without it, usage is <present|absent>. Local runs are recorded from that usage, at $0.
```

For outcome B, replace the middle with "the reply is 200 but no chunk carries `usage`. Local runs use the `Meter` estimate, at $0; `stream_options` is still sent (harmless)." For outcome C, write "the server rejects `stream_options` (HTTP <code>), so `OpenAiCompat` doesn't send it. Local runs use the `Meter` estimate, at $0." Use the date you ran it and the exact usage line you saw.

- [ ] **Step 4: Stop the server and clean up**

The script's `trap` stops `llama-server` on exit. Run `pgrep -f "llama-server.*18089" || echo stopped`.
Expected: `stopped`. Leave `$TMPDIR/llama-usage-check` alone or delete it; it's outside the repo.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md
git commit -m "docs: record whether llama-server reports usage when streaming

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 2: Core: `Usage`, `Chunk::Usage`, `Provider::model()` and the price table

**Files:**
- Modify: `crates/livebrainstorm-core/src/llm/mod.rs`
- Create: `crates/livebrainstorm-core/src/usage.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod usage;`)
- Modify: `src-tauri/src/commands.rs` (a `Chunk::Usage` arm in every `match` on `Chunk`)

**Interfaces:**
- Produces, in `livebrainstorm_core::llm`:
  ```rust
  pub struct Usage { pub model: String, pub input: u64, pub cache_write: u64, pub cache_read: u64, pub output: u64,
                     pub cost_usd: Option<f64> }          // Debug, Clone, PartialEq
  pub enum Chunk { Text(String), Activity(String), Usage(Usage) }
  impl Provider { pub fn model(&self) -> &str }
  ```
  `cost_usd: None` means the price is unknown; the ledger then counts the run as unpriced.
- Produces, in `livebrainstorm_core::usage`:
  ```rust
  pub enum UsageKind { Notes, Brief }                      // serde lowercase; Copy, Eq, Ord, Hash
  pub struct UsageTag { pub root: Option<String>, pub kind: UsageKind }   // Debug, Clone, PartialEq
  pub struct Price { pub input_per_mtok: f64, pub output_per_mtok: f64 } // Debug, Clone, Copy, PartialEq
  impl Price { pub fn cost(&self, u: &Usage) -> f64 }      // input + cache_write + cache_read at the input rate
  pub const PRICES: &[(&str, f64, f64)];                   // (model id, input $/MTok, output $/MTok)
  pub fn price(model: &str) -> Option<Price>;
  ```
- Consumes: `local::catalog::find(id) -> Option<&'static ModelSpec>` (exists today).

- [ ] **Step 1: Write the failing tests**

In `crates/livebrainstorm-core/src/llm/mod.rs`, inside `mod tests`, after `a_secret_never_prints`, add:

```rust
    #[test]
    fn model_is_what_the_provider_was_asked_for() {
        let cli = Provider::ClaudeCli(ClaudeCli::new("claude".into(), "haiku", None));
        assert_eq!(cli.model(), "haiku");
        let api = Provider::AnthropicApi(AnthropicApi {
            api_key: Secret::new("k"),
            model: "claude-haiku-5-5".into(),
            effort: None,
            http: reqwest::Client::new(),
        });
        assert_eq!(api.model(), "claude-haiku-5-5");
        let local = Provider::OpenAiCompat(OpenAiCompat {
            base_url: "http://127.0.0.1:1".into(),
            model: "qwen3-4b-instruct".into(),
            context_tokens: 4096,
            http: reqwest::Client::new(),
        });
        assert_eq!(local.model(), "qwen3-4b-instruct");
    }
```

Create `crates/livebrainstorm-core/src/usage.rs` with the types, stub bodies and the tests:

```rust
//! The usage ledger: what each model call cost, in tokens and in list-price
//! dollars, kept per hour, project, kind and model in `<app_data>/usage.json`.
//! Prices are a pinned table: updating it is a deliberate one-line change.

use serde::{Deserialize, Serialize};

use crate::llm::Usage;

/// What a call was for. The UI breaks a day down by project and kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageKind {
    Notes,
    Brief,
}

/// Where a call's usage is filed. `root: None` is a tab with no project.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageTag {
    pub root: Option<String>,
    pub kind: UsageKind,
}

/// List price in dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl Price {
    /// Cache writes and reads are priced at the input rate. LiveBrainstorm sets
    /// no `cache_control`, so the API reports none today; if any appear, this errs high.
    pub fn cost(&self, _u: &Usage) -> f64 {
        unimplemented!()
    }
}

/// List prices, $ per million tokens: (model id, input, output). Source: the
/// Claude API reference model table, cached 2026-10-06. Haiku 5.5 costs more
/// above 100K prompt tokens, but analysis prompts stay near 30K, so the lower
/// rate is used. Updating this table is a deliberate one-line change.
pub const PRICES: &[(&str, f64, f64)] = &[];

pub fn price(_model: &str) -> Option<Price> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn p(input_per_mtok: f64, output_per_mtok: f64) -> Price {
        Price { input_per_mtok, output_per_mtok }
    }

    #[test]
    fn every_model_in_the_spec_table_has_its_list_price() {
        let table = [
            ("claude-fable-5-1", p(10.0, 50.0)),
            ("claude-fable-5", p(10.0, 50.0)),
            ("claude-opus-5-5", p(4.0, 20.0)),
            ("claude-opus-5", p(5.0, 25.0)),
            ("claude-opus-4-8", p(5.0, 25.0)),
            ("claude-opus-4-7", p(5.0, 25.0)),
            ("claude-opus-4-6", p(5.0, 25.0)),
            ("claude-sonnet-5-5", p(2.0, 10.0)),
            ("claude-sonnet-5", p(2.0, 10.0)),
            ("claude-sonnet-4-6", p(3.0, 15.0)),
            ("claude-haiku-5-5", p(0.10, 0.50)),
            ("claude-haiku-4-5", p(1.0, 5.0)),
        ];
        for (id, want) in table {
            assert_eq!(price(id), Some(want), "{id}");
        }
        assert_eq!(PRICES.len(), table.len(), "no row the spec doesn't list");
    }

    #[test]
    fn the_cli_aliases_mean_the_newest_of_their_family() {
        assert_eq!(price("fable"), price("claude-fable-5-1"));
        assert_eq!(price("opus"), price("claude-opus-5-5"));
        assert_eq!(price("sonnet"), price("claude-sonnet-5-5"));
        assert_eq!(price("haiku"), price("claude-haiku-5-5"));
    }

    #[test]
    fn unknown_models_have_no_price() {
        assert_eq!(price("claude-sonnet-9"), None, "a newer family is unknown, not free");
        assert_eq!(price("Haiku"), None, "ids and aliases are exact");
        assert_eq!(price(""), None);
    }

    #[test]
    fn local_catalog_models_are_free() {
        assert_eq!(price("qwen3-4b-instruct"), Some(p(0.0, 0.0)));
        assert_eq!(price("llama-3.2-3b-instruct"), Some(p(0.0, 0.0)));
    }

    #[test]
    fn cache_tokens_are_priced_at_the_input_rate() {
        let u = Usage {
            model: "claude-haiku-5-5".into(),
            input: 1_000_000,
            cache_write: 1_000_000,
            cache_read: 1_000_000,
            output: 1_000_000,
            cost_usd: None,
        };
        assert!((p(0.10, 0.50).cost(&u) - 0.80).abs() < 1e-12);
        let small = Usage { input: 1200, cache_write: 0, cache_read: 0, output: 310, ..u };
        assert!((p(0.10, 0.50).cost(&small) - 0.000275).abs() < 1e-15);
    }
}
```

In `crates/livebrainstorm-core/src/lib.rs`, add `pub mod usage;` as the last `pub mod` line (the list is alphabetical; Phase 1 added `sessions` and `store` before it).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core -- usage::tests llm::tests` (two filters go after `--`)
Expected: compile errors, `unresolved import 'crate::llm::Usage'` in `usage.rs` and `no method named 'model' found for enum 'Provider'` in `llm::tests`. (The `usage` tests would fail with `not implemented` panics once it compiles.)

- [ ] **Step 3: Add `Usage`, `Chunk::Usage` and `Provider::model()`**

In `crates/livebrainstorm-core/src/llm/mod.rs`, replace the `Chunk` enum with:

```rust
/// What one model call used, as the provider reported it. `cost_usd` is the
/// call's list-price cost, or None when the price isn't known.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub model: String,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub cost_usd: Option<f64>,
}

/// What a provider emits while streaming.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    /// Answer text, in order.
    Text(String),
    /// Side activity worth surfacing, e.g. "Read src/main.rs".
    Activity(String),
    /// What the call used, once per model at the end of any call the provider
    /// reports usage for (failed calls included). `usage::metered` records it;
    /// it is never shown.
    Usage(Usage),
}
```

In `impl Provider`, after `can_explore_repo`, add:

```rust
    /// The model this provider was asked for: an alias ("haiku") or a model id
    /// for Claude, a catalog id for local models. Usage estimates are priced by it.
    pub fn model(&self) -> &str {
        match self {
            Provider::ClaudeCli(p) => &p.model,
            Provider::AnthropicApi(p) => &p.model,
            Provider::OpenAiCompat(p) => &p.model,
        }
    }
```

- [ ] **Step 4: Implement the price table**

In `crates/livebrainstorm-core/src/usage.rs`, replace `Price::cost`'s body with:

```rust
    pub fn cost(&self, u: &Usage) -> f64 {
        let input = (u.input + u.cache_write + u.cache_read) as f64;
        (input * self.input_per_mtok + u.output as f64 * self.output_per_mtok) / 1_000_000.0
    }
```

Replace the `PRICES` stub and the `price` stub with:

```rust
pub const PRICES: &[(&str, f64, f64)] = &[
    ("claude-fable-5-1", 10.00, 50.00),
    ("claude-fable-5", 10.00, 50.00),
    ("claude-opus-5-5", 4.00, 20.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-opus-4-8", 5.00, 25.00),
    ("claude-opus-4-7", 5.00, 25.00),
    ("claude-opus-4-6", 5.00, 25.00),
    ("claude-sonnet-5-5", 2.00, 10.00),
    ("claude-sonnet-5", 2.00, 10.00),
    ("claude-sonnet-4-6", 3.00, 15.00),
    ("claude-haiku-5-5", 0.10, 0.50),
    ("claude-haiku-4-5", 1.00, 5.00),
];

/// The Claude Code aliases, each the newest model of its family.
const ALIASES: [(&str, &str); 4] = [
    ("fable", "claude-fable-5-1"),
    ("opus", "claude-opus-5-5"),
    ("sonnet", "claude-sonnet-5-5"),
    ("haiku", "claude-haiku-5-5"),
];

/// Exact id first, then a CLI alias, then a local catalog model (free on this
/// machine). Anything else has no known price: its tokens still count.
pub fn price(model: &str) -> Option<Price> {
    let row = |id: &str| PRICES.iter().find(|(m, _, _)| *m == id);
    let alias = || ALIASES.iter().find(|(a, _)| *a == model).and_then(|&(_, id)| row(id));
    if let Some((_, input, output)) = row(model).or_else(alias) {
        return Some(Price { input_per_mtok: *input, output_per_mtok: *output });
    }
    crate::local::catalog::find(model).map(|_| Price { input_per_mtok: 0.0, output_per_mtok: 0.0 })
}
```

- [ ] **Step 5: Give every shell `match` on `Chunk` a `Usage` arm**

Adding a variant makes every exhaustive `match` on `Chunk` fail to compile. List them with `grep -n "Chunk::" src-tauri/src/commands.rs`. Phase 1 keeps two `match`es: the brief's `on_chunk` helper and the notes-lane `sink` in `analyze`. The explorer sink in `build_brief` uses `if let Chunk::Activity`, which needs nothing.

In `on_chunk`, add a third arm after `Chunk::Activity(label) => …,`:

```rust
        // Recorded by `usage::metered`, never shown.
        Chunk::Usage(_) => {}
```

In the notes-lane `sink` inside `analyze` (`let mut sink = |chunk: Chunk| match chunk { … };`), add after its `Chunk::Activity(label) => …,` arm:

```rust
                    Chunk::Usage(_) => {}
```

`crates/livebrainstorm-core/tests/notes_live.rs` (`matches!(chunk, Chunk::Text(_))`) and `brief_live.rs` (`|_: Chunk| {}`) don't match exhaustively and need no change.

- [ ] **Step 6: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core && cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: every core test passes, including the five `usage::tests` and `llm::tests::model_is_what_the_provider_was_asked_for`; `prompt::tests` and `claude_cli::tests` pass unchanged. The shell compiles, and clippy prints no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/llm/mod.rs crates/livebrainstorm-core/src/usage.rs crates/livebrainstorm-core/src/lib.rs src-tauri/src/commands.rs
git commit -m "feat(core): Chunk::Usage, Provider::model and the list-price table

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 3: Core: Claude Code reports usage from its `result` event

**Files:**
- Modify: `crates/livebrainstorm-core/src/llm/claude_cli.rs`

**Interfaces:**
- Consumes (Task 2): `Usage { model: String, input: u64, cache_write: u64, cache_read: u64, output: u64, cost_usd: Option<f64> }`, `Chunk::Usage(Usage)`, `usage::price(model: &str) -> Option<Price>` and `Price::cost(&self, u: &Usage) -> f64`.
- Produces:
  - `ClaudeCli::stream` emits one `Chunk::Usage` per model, after all text, whenever the `result` event carries usage, including when `is_error` is true. Its signature and return value are unchanged. A record whose event reported no cost (`costUSD` / `total_cost_usd` missing) is priced with `usage::price`, and stays `None` only for a model the table doesn't know.
  - Private: `parse_line(line: &str, model: &str) -> Vec<CliEvent>` (the requested model names the fallback record); `CliEvent::Result { text, is_error, usage: Vec<Usage> }`; `fn result_usage(v: &Value, requested: &str) -> Vec<Usage>`, sorted by model.

The fixture is the CLI `result` event recorded on 2026-10-14 (`--model haiku`, resolved to `claude-haiku-5-5`, `costUSD` 0.00046765, `costBasis: "list"`). Keep it byte for byte.

- [ ] **Step 1: Write the failing tests**

In `crates/livebrainstorm-core/src/llm/claude_cli.rs`, inside `mod tests`, add at the top of the module (after `use super::*;`, which brings in `Usage` once Step 3 imports it):

```rust
    /// The `result` event recorded on 2026-10-14 with `--model haiku`, verbatim.
    const FIXTURE: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Hi!","total_cost_usd":0.00046765,"usage":{"input_tokens":2,"cache_creation_input_tokens":2198,"cache_read_input_tokens":535,"output_tokens":45},"modelUsage":{"claude-haiku-5-5":{"inputTokens":2,"outputTokens":45,"cacheReadInputTokens":535,"cacheCreationInputTokens":2198,"costUSD":0.00046765,"canonicalModel":"claude-haiku-5-5","costBasis":"list"}}}"#;

    /// A run that used two models (for example a sonnet run whose sub-tasks used haiku).
    const TWO_MODELS: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok","total_cost_usd":0.01283,"usage":{"input_tokens":12,"cache_creation_input_tokens":4100,"cache_read_input_tokens":900,"output_tokens":610},"modelUsage":{"claude-sonnet-5-5":{"inputTokens":10,"outputTokens":580,"cacheReadInputTokens":900,"cacheCreationInputTokens":4100,"costUSD":0.0124,"costBasis":"list"},"claude-haiku-5-5":{"inputTokens":2,"outputTokens":30,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.00043,"costBasis":"list"}}}"#;

    fn haiku(input: u64, cache_write: u64, cache_read: u64, output: u64, cost: f64) -> Usage {
        Usage { model: "claude-haiku-5-5".into(), input, cache_write, cache_read, output, cost_usd: Some(cost) }
    }
```

Change the existing parser tests to the two-argument `parse_line` and the new `Result` shape. Replace `parses_text_delta`, `parses_tool_use_and_result` and `ignores_noise` with:

```rust
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
```

Add these tests after `ignores_noise`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core claude_cli::tests`
Expected: compile errors, such as `cannot find struct, variant or union type 'Usage' in this scope`, `this function takes 1 argument but 2 arguments were supplied` for `parse_line`, and `variant 'CliEvent::Result' does not have a field named 'usage'`.

- [ ] **Step 3: Parse `modelUsage` into `Usage`**

In `crates/livebrainstorm-core/src/llm/claude_cli.rs`, change the `use super::…` line to:

```rust
use super::{ChatRequest, Chunk, OnChunk, Secret, Usage};
```

and add `use crate::usage::price;` after `use crate::settings::Effort;`.

Replace the `CliEvent` enum, `parse_line`'s signature and its `Some("result")` arm. The enum becomes:

```rust
#[derive(Debug, PartialEq)]
enum CliEvent {
    Text(String),
    ToolUse(String),
    Result { text: Option<String>, is_error: bool, usage: Vec<Usage> },
}
```

The function's doc comment and signature become:

```rust
/// One NDJSON line from `--output-format stream-json`. Unknown shapes are
/// ignored so CLI upgrades that add event types don't break us. `model` is
/// the model we asked for; usage without a per-model breakdown is filed under it.
fn parse_line(line: &str, model: &str) -> Vec<CliEvent> {
```

and its `Some("result")` arm becomes:

```rust
        Some("result") => vec![CliEvent::Result {
            text: v["result"].as_str().map(str::to_owned),
            is_error: v["is_error"].as_bool().unwrap_or(false),
            usage: result_usage(&v, model),
        }],
```

Add this function after `parse_line`:

```rust
/// What a `result` event says the run used: one `Usage` per model in
/// `modelUsage`, priced by its `costUSD` (their sum is `total_cost_usd`).
/// Without `modelUsage`, the `usage` totals with `total_cost_usd`, under the
/// requested model. A record with no reported cost is priced from
/// `usage::PRICES` (None if the table doesn't know the model). Empty when the
/// event reports neither. Sorted by model.
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
    for u in &mut usage {
        if u.cost_usd.is_none() {
            let listed = price(&u.model).map(|p| p.cost(u));
            u.cost_usd = listed;
        }
    }
    usage.sort_by(|a, b| a.model.cmp(&b.model));
    usage
}
```

- [ ] **Step 4: Emit the usage at the end of `stream`, whatever the outcome**

In `ClaudeCli::stream`, change the declarations before the read loop and the loop itself to:

```rust
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
```

The `match result { … }` that follows is unchanged.

- [ ] **Step 5: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core claude_cli::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: all `claude_cli::tests` pass, including the five new ones (the two `#[cfg(unix)]` tests run the fake CLI script), and the existing argument tests pass unchanged. The whole core suite passes, and clippy prints no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/llm/claude_cli.rs
git commit -m "feat(core): Claude Code runs report usage per model, failed runs included

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 4: Core: usage from the Anthropic API and the local server

**Files:**
- Modify: `crates/livebrainstorm-core/src/llm/anthropic_api.rs`
- Modify: `crates/livebrainstorm-core/src/llm/openai_compat.rs`

**Interfaces:**
- Consumes:
  - `Usage`, `Chunk::Usage` (Task 2)
  - `usage::price(model: &str) -> Option<Price>` and `Price::cost(&self, u: &Usage) -> f64` (Task 2)
  - Task 1's recorded outcome (A, B or C)
  - The test server in `crate::jev::test_server` (exists today, `#[cfg(test)] pub(crate)`): `serve_with(handler: impl Fn(&str) -> Option<Canned> + Send + Sync + 'static) -> (String, Arc<AtomicUsize>)` gives the handler each request body and returns the base URL; `Canned::new(status: u16, body: &str)`.
- Produces:
  - `AnthropicApi::stream` emits one `Chunk::Usage` after the stream ends (also after a stream `error` event) when `message_start` or `message_delta` reported usage. `model` is `message_start.message.model`, else the requested model; `cost_usd` is `price(model)` × tokens, or None.
  - `OpenAiCompat::stream` sends `"stream_options": { "include_usage": true }` (outcomes A and B; not for C) and emits one `Chunk::Usage { model: <requested catalog id>, input: prompt_tokens, output: completion_tokens, cache_write: 0, cache_read: 0, cost_usd: Some(0.0) }` when any chunk carried a `usage` object (the last one wins).

- [ ] **Step 1: Write the failing tests**

In `crates/livebrainstorm-core/src/llm/anthropic_api.rs`, inside `mod tests`, after `effort_goes_in_output_config_only_when_set`, add:

```rust
    fn observe_all(events: &[&str]) -> UsageAcc {
        let mut acc = UsageAcc::default();
        for e in events {
            acc.observe(&serde_json::from_str(e).unwrap());
        }
        acc
    }

    const START: &str = r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-haiku-5-5","content":[],"stop_reason":null,"usage":{"input_tokens":1200,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#;
    const DELTA: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#;
    const MESSAGE_DELTA: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":310}}"#;

    #[test]
    fn usage_comes_from_message_start_and_the_last_message_delta() {
        let u = observe_all(&[START, DELTA, MESSAGE_DELTA]).usage("haiku").expect("usage was reported");
        assert_eq!((u.model.as_str(), u.input, u.cache_write, u.cache_read, u.output), ("claude-haiku-5-5", 1200, 0, 0, 310));
        assert!((u.cost_usd.unwrap() - 0.000275).abs() < 1e-15, "1200 × $0.10 + 310 × $0.50 per million");
    }

    #[test]
    fn a_stream_without_usage_reports_none() {
        assert_eq!(observe_all(&[DELTA]).usage("claude-haiku-5-5"), None);
    }

    #[test]
    fn an_unknown_model_is_counted_without_a_price() {
        let start = START.replace("claude-haiku-5-5", "claude-sonnet-9");
        let u = observe_all(&[start.as_str(), MESSAGE_DELTA]).usage("claude-sonnet-9").unwrap();
        assert_eq!((u.model.as_str(), u.output, u.cost_usd), ("claude-sonnet-9", 310, None));
    }
```

In `crates/livebrainstorm-core/src/llm/openai_compat.rs`, add a test module at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::jev::test_server::{serve_with, Canned};

    /// llama.cpp style: usage rides on the final chunk with `finish_reason`.
    const LLAMA: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n\
        data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"}}]}\n\n\
        data: {\"choices\":[{\"finish_reason\":\"stop\",\"index\":0,\"delta\":{}}],\"usage\":{\"completion_tokens\":2,\"prompt_tokens\":31,\"total_tokens\":33}}\n\n\
        data: [DONE]\n\n";
    /// OpenAI style: `usage: null` on every chunk, then a chunk with no choices.
    const OPENAI: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}],\"usage\":null}\n\n\
        data: {\"choices\":[],\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":2,\"total_tokens\":33}}\n\n\
        data: [DONE]\n\n";
    const NO_USAGE: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: [DONE]\n\n";

    /// Streams one request against `reply`; returns the answer, the chunks and the request body.
    async fn run(reply: &'static str) -> (String, Vec<Chunk>, serde_json::Value) {
        let seen = Arc::new(Mutex::new(String::new()));
        let body = seen.clone();
        let (base_url, _) = serve_with(move |req: &str| {
            *body.lock().unwrap() = req.to_owned();
            Some(Canned::new(200, reply))
        })
        .await;
        let local = OpenAiCompat { base_url, model: "qwen3-4b-instruct".into(), context_tokens: 4096, http: reqwest::Client::new() };
        let req = ChatRequest { system: "s".into(), user: "u".into(), max_tokens: 50, cwd: None, allow_repo_tools: false, max_turns: None };
        let mut chunks = Vec::new();
        let text = local.stream(&req, &mut |c: Chunk| chunks.push(c)).await.unwrap();
        let sent = serde_json::from_str(&seen.lock().unwrap()).unwrap();
        (text, chunks, sent)
    }

    fn local_usage() -> Chunk {
        Chunk::Usage(Usage { model: "qwen3-4b-instruct".into(), input: 31, cache_write: 0, cache_read: 0, output: 2, cost_usd: Some(0.0) })
    }

    #[tokio::test]
    async fn asks_for_usage_and_reports_it_once_at_the_end() {
        let (text, chunks, sent) = run(LLAMA).await;
        assert_eq!(text, "Hello");
        assert_eq!(chunks, vec![Chunk::Text("Hel".into()), Chunk::Text("lo".into()), local_usage()]);
        assert_eq!(sent["stream_options"], serde_json::json!({ "include_usage": true }));
    }

    #[tokio::test]
    async fn reads_openai_style_usage_chunks_too() {
        let (_, chunks, _) = run(OPENAI).await;
        assert_eq!(chunks, vec![Chunk::Text("Hello".into()), local_usage()]);
    }

    #[tokio::test]
    async fn a_server_that_reports_no_usage_emits_none() {
        let (text, chunks, _) = run(NO_USAGE).await;
        assert_eq!((text.as_str(), chunks), ("Hello", vec![Chunk::Text("Hello".into())]));
    }
}
```

**If Task 1 recorded outcome C** (the server rejects `stream_options`), replace the last line of `asks_for_usage_and_reports_it_once_at_the_end` with:

```rust
        assert!(sent.get("stream_options").is_none(), "b11480 rejects stream_options (pre-implementation check 3)");
```

and rename the test `reports_usage_once_at_the_end_without_asking`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core -- anthropic_api::tests openai_compat::tests` (two filters go after `--`)
Expected: compile errors, such as `cannot find type 'UsageAcc' in this scope` and, in `openai_compat::tests`, `cannot find struct, variant or union type 'Usage'` (the parent module doesn't import it yet).

- [ ] **Step 3: Anthropic API: collect usage across the stream**

In `crates/livebrainstorm-core/src/llm/anthropic_api.rs`, change the imports to:

```rust
use serde_json::{json, Value};

use super::{sse, ChatRequest, Chunk, OnChunk, Secret, Usage};
use crate::settings::Effort;
use crate::usage::price;
use crate::{Error, Result};
```

Add this after the `body` function:

```rust
/// Usage spread over the stream: input and cache counts in `message_start`,
/// the running output count in each `message_delta` (the last one is final).
#[derive(Debug, Default)]
struct UsageAcc {
    model: Option<String>,
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
    seen: bool,
}

impl UsageAcc {
    fn observe(&mut self, v: &Value) {
        let n = |x: &Value| x.as_u64().unwrap_or(0);
        match v["type"].as_str() {
            Some("message_start") => {
                let (message, u) = (&v["message"], &v["message"]["usage"]);
                self.model = message["model"].as_str().map(str::to_owned);
                self.input = n(&u["input_tokens"]);
                self.cache_write = n(&u["cache_creation_input_tokens"]);
                self.cache_read = n(&u["cache_read_input_tokens"]);
                self.output = n(&u["output_tokens"]);
                self.seen = true;
            }
            Some("message_delta") => {
                if let Some(output) = v["usage"]["output_tokens"].as_u64() {
                    self.output = output;
                    self.seen = true;
                }
            }
            _ => {}
        }
    }

    /// None when the stream never reported usage. Priced from `usage::PRICES`.
    fn usage(self, requested: &str) -> Option<Usage> {
        if !self.seen {
            return None;
        }
        let mut u = Usage {
            model: self.model.unwrap_or_else(|| requested.to_owned()),
            input: self.input,
            cache_write: self.cache_write,
            cache_read: self.cache_read,
            output: self.output,
            cost_usd: None,
        };
        u.cost_usd = price(&u.model).map(|p| p.cost(&u));
        Some(u)
    }
}
```

In `AnthropicApi::stream`, replace everything from `let mut full = String::new();` to the end of the function with:

```rust
        let mut full = String::new();
        let mut acc = UsageAcc::default();
        let streamed = sse::for_each_data(resp, |data| {
            let v: Value = serde_json::from_str(data)?;
            acc.observe(&v);
            match v["type"].as_str() {
                Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                    if let Some(t) = v["delta"]["text"].as_str() {
                        full.push_str(t);
                        on_chunk(Chunk::Text(t.to_owned()));
                    }
                    Ok(true)
                }
                Some("message_stop") => Ok(false),
                Some("error") => Err(Error::Provider(format!(
                    "Anthropic API stream error: {}",
                    v["error"]["message"].as_str().unwrap_or("unknown")
                ))),
                _ => Ok(true),
            }
        })
        .await;
        // Even after a stream error: the input was already billed.
        if let Some(u) = acc.usage(&self.model) {
            on_chunk(Chunk::Usage(u));
        }
        streamed?;
        Ok(full)
```

- [ ] **Step 4: Local server: ask for usage and read it**

In `crates/livebrainstorm-core/src/llm/openai_compat.rs`, change the `use super::…` line to:

```rust
use super::{sse, ChatRequest, Chunk, OnChunk, Usage};
```

In `OpenAiCompat::stream`, add `stream_options` to the body, after `"temperature": 0.4,`:

```rust
            // The final chunk then carries `usage` (pre-implementation check 3).
            "stream_options": { "include_usage": true },
```

**If Task 1 recorded outcome C,** don't add those two lines: the parser below still reads `usage` if a server sends it, and the `Meter` estimates otherwise.

Replace everything from `let mut full = String::new();` to the end of the function with:

```rust
        let mut full = String::new();
        let mut usage = None;
        let streamed = sse::for_each_data(resp, |data| {
            if data.trim() == "[DONE]" {
                return Ok(false);
            }
            let v: Value = serde_json::from_str(data)?;
            if let Some(u) = usage_of(&v, &self.model) {
                usage = Some(u); // cumulative: the last report wins
            }
            if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                if !t.is_empty() {
                    full.push_str(t);
                    on_chunk(Chunk::Text(t.to_owned()));
                }
            }
            Ok(true)
        })
        .await;
        if let Some(u) = usage {
            on_chunk(Chunk::Usage(u));
        }
        streamed?;
        Ok(full)
```

Add this function after the `impl OpenAiCompat` block:

```rust
/// A chunk's `usage` object (llama.cpp puts it on the final chunk, OpenAI on a
/// chunk with no choices; both send `null` or nothing elsewhere). Filed under
/// the model we asked for, so exact and estimated runs share a bucket. Local
/// models cost nothing.
fn usage_of(v: &Value, model: &str) -> Option<Usage> {
    let u = v.get("usage").filter(|u| u.is_object())?;
    Some(Usage {
        model: model.to_owned(),
        input: u["prompt_tokens"].as_u64().unwrap_or(0),
        cache_write: 0,
        cache_read: 0,
        output: u["completion_tokens"].as_u64().unwrap_or(0),
        cost_usd: Some(0.0),
    })
}
```

- [ ] **Step 5: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core -- anthropic_api::tests openai_compat::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: the three `anthropic_api` and three `openai_compat` tests pass, `effort_goes_in_output_config_only_when_set` still passes, the whole core suite passes, and clippy prints no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/llm/anthropic_api.rs crates/livebrainstorm-core/src/llm/openai_compat.rs
git commit -m "feat(core): usage from the Anthropic API stream and the local server

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 5: Core: the usage ledger

**Files:**
- Modify: `crates/livebrainstorm-core/src/usage.rs`

**Interfaces:**
- Consumes:
  - `Usage` (Task 2), `UsageTag`, `UsageKind` (Task 2)
  - Phase 1's `crate::store`:
    ```rust
    pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()>;   // <file>.tmp then rename; creates the parent dir
    pub enum Loaded<T> { Missing, Parsed(T), Corrupt { kept: Option<PathBuf> } }
    /// Err only for read errors other than NotFound (those files are left in place).
    /// A file that doesn't parse is renamed to the first free `<name>.bad`, `<name>.bad.1`, … (`Corrupt`).
    pub fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Loaded<T>>;
    ```
- Produces, in `livebrainstorm_core::usage`:
  ```rust
  pub const KEEP_DAYS: u64 = 90;
  pub struct UsageBucket { pub hour: u64, pub root: Option<String>, pub kind: UsageKind, pub model: String,
    pub input: u64, pub cache_write: u64, pub cache_read: u64, pub output: u64, pub cost_usd: f64, pub runs: u64,
    pub estimated_runs: u64, pub estimated_cost_usd: f64, pub unpriced_runs: u64 }   // camelCase; Debug, Clone, PartialEq, Serialize, Deserialize
  pub struct UsageLedger;
  impl UsageLedger {
      pub fn new(path: PathBuf) -> Self;                                   // loads, prunes to KEEP_DAYS
      pub fn set_on_record(&self, f: Box<dyn Fn() + Send + Sync>);         // called after each record, outside the ledger's lock
      pub fn record(&self, tag: &UsageTag, usage: &Usage, estimated: bool);
      pub fn report(&self, since_hour: u64) -> Vec<UsageBucket>;           // hour >= since_hour, sorted by (hour, root, kind, model)
      pub fn flush_if_dirty(&self) -> Result<()>;                          // writes only when something changed
      pub fn flush(&self) -> Result<()>;                                   // writes now
  }
  ```
  `cost_usd` is the bucket's total cost, estimates included; `estimated_cost_usd` is the part that came from estimates. A record whose `usage.cost_usd` is None adds its tokens and a run, plus one `unpriced_runs`, and no cost.
  Private, used by tests and Task 6: `fn load_at(path: PathBuf, now_secs: u64) -> Self`, `fn record_at(&self, now_secs: u64, tag, usage, estimated)`, `UsageBucket::merge(&mut self, other: &UsageBucket)`.

- [ ] **Step 1: Add the types, stubs and failing tests**

In `crates/livebrainstorm-core/src/usage.rs`, replace the `use` lines at the top with:

```rust
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::llm::Usage;
use crate::store::{self, Loaded};
use crate::Result;
```

Add this section after the `price` function (before `#[cfg(test)]`):

```rust
// ---------- the ledger ----------

/// Buckets older than this are dropped when the ledger loads.
pub const KEEP_DAYS: u64 = 90;

/// One hour (UTC, `unix_secs / 3600`) of one project's usage, for one kind and
/// one model. `cost_usd` is the total, estimates included; `estimated_cost_usd`
/// is the part that came from estimates. Unpriced runs add tokens, not cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub hour: u64,
    pub root: Option<String>,
    pub kind: UsageKind,
    pub model: String,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub cost_usd: f64,
    pub runs: u64,
    pub estimated_runs: u64,
    pub estimated_cost_usd: f64,
    pub unpriced_runs: u64,
}

/// (hour, root, kind, model): one bucket per key.
type Key = (u64, Option<String>, UsageKind, String);

type OnRecord = Box<dyn Fn() + Send + Sync>;

#[derive(Serialize, Deserialize)]
struct LedgerFile {
    version: u32,
    buckets: Vec<UsageBucket>,
}

struct Inner {
    buckets: BTreeMap<Key, UsageBucket>,
    /// Changed since the last write.
    dirty: bool,
    /// False when usage.json exists but couldn't be read: it's never overwritten.
    persist: bool,
}

/// Every model call's usage, in memory, saved to `usage.json` by the shell
/// (`flush_if_dirty` every 5 s, `flush` on exit). `std` locks only, held for
/// synchronous work: `Meter::drop` records through it from aborted tasks.
pub struct UsageLedger {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// Serializes writers, so an older snapshot never lands after a newer one.
    writing: Mutex<()>,
    on_record: Mutex<Option<OnRecord>>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A panic elsewhere never stops the ledger: it holds plain numbers.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl UsageLedger {
    pub fn new(path: PathBuf) -> Self {
        Self::load_at(path, now_secs())
    }

    fn load_at(_path: PathBuf, _now: u64) -> Self {
        unimplemented!()
    }

    pub fn set_on_record(&self, _f: OnRecord) {
        unimplemented!()
    }

    pub fn record(&self, tag: &UsageTag, usage: &Usage, estimated: bool) {
        self.record_at(now_secs(), tag, usage, estimated)
    }

    fn record_at(&self, _now: u64, _tag: &UsageTag, _usage: &Usage, _estimated: bool) {
        unimplemented!()
    }

    pub fn report(&self, _since_hour: u64) -> Vec<UsageBucket> {
        unimplemented!()
    }

    pub fn flush_if_dirty(&self) -> Result<()> {
        unimplemented!()
    }

    pub fn flush(&self) -> Result<()> {
        unimplemented!()
    }
}
```

Inside `mod tests`, add after the price tests:

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 2025-10-14T00:00:00Z: the spec's example hour, 489000.
    const T: u64 = 1_760_400_000;
    const DAY: u64 = 86_400;

    fn tag(root: &str, kind: UsageKind) -> UsageTag {
        UsageTag { root: Some(root.into()), kind }
    }

    fn used(model: &str, input: u64, output: u64, cost_usd: Option<f64>) -> Usage {
        Usage { model: model.into(), input, cache_write: 0, cache_read: 0, output, cost_usd }
    }

    fn ledger() -> (tempfile::TempDir, UsageLedger) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::load_at(dir.path().join("usage.json"), T);
        (dir, ledger)
    }

    #[test]
    fn runs_add_up_in_hourly_buckets_per_project_kind_and_model() {
        let (_dir, l) = ledger();
        let notes = tag("/a", UsageKind::Notes);
        l.record_at(T + 10, &notes, &used("claude-haiku-5-5", 100, 10, Some(0.001)), false);
        l.record_at(T + 3599, &notes, &used("claude-haiku-5-5", 200, 20, Some(0.002)), false);
        l.record_at(T + 3600, &notes, &used("claude-haiku-5-5", 1, 1, Some(0.0)), false);
        l.record_at(T + 20, &tag("/b", UsageKind::Notes), &used("claude-haiku-5-5", 1, 1, Some(0.0)), false);
        l.record_at(T + 30, &tag("/a", UsageKind::Brief), &used("claude-sonnet-5-5", 1, 1, Some(0.0)), false);
        l.record_at(T + 40, &notes, &used("haiku", 1, 1, Some(0.0)), false);
        let all = l.report(0);
        assert_eq!(all.len(), 5, "{all:?}");
        let first = all
            .iter()
            .find(|b| b.hour == 489_000 && b.root.as_deref() == Some("/a") && b.kind == UsageKind::Notes && b.model == "claude-haiku-5-5")
            .unwrap();
        assert_eq!((first.input, first.output, first.runs), (300, 30, 2));
        assert!((first.cost_usd - 0.003).abs() < 1e-12);
        let later = l.report(489_001);
        assert_eq!(later.len(), 1, "report starts at the given hour");
        assert_eq!(later[0].hour, 489_001);
    }

    #[test]
    fn estimates_and_unpriced_runs_are_kept_apart() {
        let (_dir, l) = ledger();
        let t = tag("/a", UsageKind::Notes);
        l.record_at(T, &t, &used("m", 10, 0, Some(0.5)), false);
        l.record_at(T, &t, &used("m", 10, 0, Some(0.25)), true);
        l.record_at(T, &t, &used("m", 10, 0, None), false);
        let b = &l.report(0)[0];
        assert_eq!((b.runs, b.estimated_runs, b.unpriced_runs), (3, 1, 1));
        assert_eq!(b.cost_usd, 0.75, "the total includes estimates");
        assert_eq!(b.estimated_cost_usd, 0.25);
        assert_eq!(b.input, 30, "unpriced runs still count their tokens");
    }

    #[test]
    fn the_file_has_the_spec_shape() {
        let (dir, l) = ledger();
        let u = Usage { model: "claude-haiku-5-5".into(), input: 1200, cache_write: 22000, cache_read: 90000, output: 3100, cost_usd: Some(0.0123) };
        l.record_at(T, &tag("/Users/me/code/shop-api", UsageKind::Notes), &u, false);
        l.flush().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("usage.json")).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "version": 1, "buckets": [{
                "hour": 489000, "root": "/Users/me/code/shop-api", "kind": "notes", "model": "claude-haiku-5-5",
                "input": 1200, "cacheWrite": 22000, "cacheRead": 90000, "output": 3100,
                "costUsd": 0.0123, "runs": 1, "estimatedRuns": 0, "estimatedCostUsd": 0.0, "unpricedRuns": 0 }] })
        );
    }

    #[test]
    fn the_ledger_survives_a_restart_and_forgets_after_90_days() {
        let (dir, l) = ledger();
        let t = tag("/a", UsageKind::Notes);
        l.record_at(T - 91 * DAY, &t, &used("m", 1, 1, Some(0.0)), false);
        l.record_at(T - 89 * DAY, &t, &used("m", 2, 2, Some(0.0)), false);
        l.record_at(T, &tag("/b", UsageKind::Brief), &used("m", 3, 3, None), true);
        l.flush().unwrap();
        let again = UsageLedger::load_at(dir.path().join("usage.json"), T);
        let kept: Vec<(u64, u64)> = again.report(0).iter().map(|b| (b.hour, b.input)).collect();
        assert_eq!(kept, [((T - 89 * DAY) / 3600, 2), (T / 3600, 3)]);
        assert_eq!(again.report(0)[1].unpriced_runs, 1, "every field survives the round trip");
    }

    #[test]
    fn a_corrupt_ledger_is_kept_as_bad_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        std::fs::write(&path, "{ not json").unwrap();
        let l = UsageLedger::load_at(path.clone(), T);
        assert!(l.report(0).is_empty());
        let bad = dir.path().join("usage.json.bad");
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ not json");
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        l.flush().unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ not json", "later writes never touch the .bad copy");
        assert_eq!(UsageLedger::load_at(path, T).report(0).len(), 1);
    }

    #[test]
    fn an_unreadable_ledger_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        std::fs::create_dir(&path).unwrap(); // in the way: a read error, not a parse error
        let l = UsageLedger::load_at(path.clone(), T);
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        assert_eq!(l.report(0).len(), 1, "still counted for this session");
        l.flush().unwrap();
        l.flush_if_dirty().unwrap();
        assert!(path.is_dir(), "left in place");
        assert!(!dir.path().join("usage.json.bad").exists());
    }

    #[test]
    fn flush_if_dirty_writes_only_after_a_change() {
        let (dir, l) = ledger();
        let path = dir.path().join("usage.json");
        l.flush_if_dirty().unwrap();
        assert!(!path.exists(), "nothing recorded, nothing written");
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        l.flush_if_dirty().unwrap();
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
        l.flush_if_dirty().unwrap();
        assert!(!path.exists(), "clean after a write");
    }

    #[test]
    fn on_record_runs_after_every_record_and_may_read_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let l = Arc::new(UsageLedger::load_at(dir.path().join("usage.json"), T));
        let calls = Arc::new(AtomicUsize::new(0));
        let (seen, weak) = (calls.clone(), Arc::downgrade(&l));
        l.set_on_record(Box::new(move || {
            // Called outside the ledger's lock: reading it here must not deadlock.
            let runs: u64 = weak.upgrade().map_or(0, |l| l.report(0).iter().map(|b| b.runs).sum());
            seen.fetch_add(runs as usize, Ordering::SeqCst);
        }));
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        l.record_at(T, &tag("/a", UsageKind::Notes), &used("m", 1, 1, Some(0.0)), false);
        assert_eq!(calls.load(Ordering::SeqCst), 1 + 2);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core usage::tests`
Expected: the eight new ledger tests FAIL with `not implemented` panics; the five price tests still pass.

- [ ] **Step 3: Implement the ledger**

Add this block after the `UsageBucket` struct:

```rust
impl UsageBucket {
    fn empty(hour: u64, tag: &UsageTag, model: &str) -> Self {
        Self {
            hour,
            root: tag.root.clone(),
            kind: tag.kind,
            model: model.to_owned(),
            input: 0,
            cache_write: 0,
            cache_read: 0,
            output: 0,
            cost_usd: 0.0,
            runs: 0,
            estimated_runs: 0,
            estimated_cost_usd: 0.0,
            unpriced_runs: 0,
        }
    }

    fn key(&self) -> Key {
        (self.hour, self.root.clone(), self.kind, self.model.clone())
    }

    fn add(&mut self, u: &Usage, estimated: bool) {
        self.input += u.input;
        self.cache_write += u.cache_write;
        self.cache_read += u.cache_read;
        self.output += u.output;
        self.runs += 1;
        if estimated {
            self.estimated_runs += 1;
        }
        match u.cost_usd {
            Some(cost) => {
                self.cost_usd += cost;
                if estimated {
                    self.estimated_cost_usd += cost;
                }
            }
            None => self.unpriced_runs += 1,
        }
    }

    /// Adds another bucket's counts (same key on load; any key for totals in tests).
    fn merge(&mut self, o: &UsageBucket) {
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.output += o.output;
        self.cost_usd += o.cost_usd;
        self.runs += o.runs;
        self.estimated_runs += o.estimated_runs;
        self.estimated_cost_usd += o.estimated_cost_usd;
        self.unpriced_runs += o.unpriced_runs;
    }
}
```

Replace the whole `impl UsageLedger { … }` block with:

```rust
impl UsageLedger {
    pub fn new(path: PathBuf) -> Self {
        Self::load_at(path, now_secs())
    }

    /// Never fails: a missing or corrupt file starts an empty ledger (a corrupt
    /// one is kept as `.bad`), and one that can't be read is left alone and not
    /// saved over. Buckets older than `KEEP_DAYS` are dropped.
    fn load_at(path: PathBuf, now: u64) -> Self {
        let (loaded, persist) = match store::load_json::<LedgerFile>(&path) {
            Ok(Loaded::Parsed(file)) => (file.buckets, true),
            Ok(Loaded::Missing) => (Vec::new(), true),
            Ok(Loaded::Corrupt { kept }) => {
                let kept = kept.map_or_else(|| "nothing".to_owned(), |p| p.display().to_string());
                eprintln!("usage.json couldn't be parsed; kept it as {kept} and started a new ledger");
                (Vec::new(), true)
            }
            Err(e) => {
                eprintln!("Couldn't read {}: {e}. Usage is counted for this session but not saved.", path.display());
                (Vec::new(), false)
            }
        };
        let oldest = (now / 3600).saturating_sub(KEEP_DAYS * 24);
        let mut buckets = BTreeMap::new();
        for b in loaded {
            if b.hour < oldest {
                continue;
            }
            match buckets.entry(b.key()) {
                Entry::Occupied(mut have) => UsageBucket::merge(have.get_mut(), &b),
                Entry::Vacant(slot) => {
                    slot.insert(b);
                }
            }
        }
        Self {
            path,
            inner: Mutex::new(Inner { buckets, dirty: false, persist }),
            writing: Mutex::new(()),
            on_record: Mutex::new(None),
        }
    }

    /// The shell's hook, called after every record (outside the ledger's lock).
    pub fn set_on_record(&self, f: OnRecord) {
        *lock(&self.on_record) = Some(f);
    }

    /// Adds one call to its hour's bucket. Synchronous and never awaits, so
    /// `Meter::drop` can call it from a task that is being aborted.
    pub fn record(&self, tag: &UsageTag, usage: &Usage, estimated: bool) {
        self.record_at(now_secs(), tag, usage, estimated)
    }

    fn record_at(&self, now: u64, tag: &UsageTag, usage: &Usage, estimated: bool) {
        {
            let mut inner = lock(&self.inner);
            let hour = now / 3600;
            inner
                .buckets
                .entry((hour, tag.root.clone(), tag.kind, usage.model.clone()))
                .or_insert_with(|| UsageBucket::empty(hour, tag, &usage.model))
                .add(usage, estimated);
            inner.dirty = true;
        }
        if let Some(on_record) = lock(&self.on_record).as_ref() {
            on_record();
        }
    }

    /// Buckets from `since_hour` on, sorted by (hour, root, kind, model).
    pub fn report(&self, since_hour: u64) -> Vec<UsageBucket> {
        lock(&self.inner).buckets.values().filter(|b| b.hour >= since_hour).cloned().collect()
    }

    /// Writes the ledger if anything changed since the last write.
    pub fn flush_if_dirty(&self) -> Result<()> {
        self.write(false)
    }

    /// Writes the ledger now (on exit), which also saves the 90-day prune.
    pub fn flush(&self) -> Result<()> {
        self.write(true)
    }

    fn write(&self, always: bool) -> Result<()> {
        let _writing = lock(&self.writing);
        let bytes = {
            let mut inner = lock(&self.inner);
            if !inner.persist || !(always || inner.dirty) {
                return Ok(());
            }
            let file = LedgerFile { version: 1, buckets: inner.buckets.values().cloned().collect() };
            inner.dirty = false;
            serde_json::to_vec(&file)?
        };
        if let Err(e) = store::write_atomic(&self.path, &bytes) {
            lock(&self.inner).dirty = true; // try again at the next flush
            return Err(e);
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core usage::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: all 13 `usage::tests` pass, the whole core suite passes, and clippy prints no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/usage.rs
git commit -m "feat(core): usage ledger in hourly buckets, pruned to 90 days, corrupt files kept as .bad

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 6: Core: the `Meter`, and `metered` around one model call

**Files:**
- Modify: `crates/livebrainstorm-core/src/usage.rs`

**Interfaces:**
- Consumes:
  - `UsageLedger::record(&self, tag: &UsageTag, usage: &Usage, estimated: bool)` and the private `UsageBucket::merge` (Task 5)
  - `price(model: &str) -> Option<Price>`, `Price::cost` (Task 2)
  - `Provider::model(&self) -> &str`, `Provider::stream(&self, req: &ChatRequest, on_chunk: OnChunk<'_>) -> Result<String>`, `OnChunk<'a> = &'a mut (dyn FnMut(Chunk) + Send)` (Task 2 / today)
  - The test server `crate::jev::test_server::{serve, Canned}`: `serve(responses: Vec<Canned>) -> (String, Arc<AtomicUsize>)` answers each connection with the next response and counts connections; `Canned::new(status, body)`, `Canned::delayed(self, ms: u64) -> Self`.
- Produces, in `livebrainstorm_core::usage`:
  ```rust
  pub struct Meter;
  impl Meter {
      pub fn new(ledger: Arc<UsageLedger>, tag: UsageTag, model: String, sent_chars: usize) -> Self;
      pub fn observe(&mut self, chunk: &Chunk);   // counts Text chars; records each Usage exactly
      pub fn finish(self, ok: bool);              // Ok without usage → estimate; Err without usage → nothing
  }                                               // Drop before finish, without usage → estimate
  /// Characters sent: `system` + `user`, as Unicode scalars.
  pub fn sent_chars(req: &ChatRequest) -> usize;
  /// One metered call: a `Meter` created right before `provider.stream`, finished with its result.
  /// Forwards `Text` and `Activity` to `on_chunk`; never forwards `Usage`.
  pub async fn metered(provider: &Provider, req: &ChatRequest, ledger: Arc<UsageLedger>, tag: UsageTag,
                       on_chunk: OnChunk<'_>) -> Result<String>;
  ```

The shell (Task 7) calls `metered` for every notes lane, explorer and merge, so "the meter is created immediately before `provider.stream`, after the pre-phase" holds by construction.

- [ ] **Step 1: Add the stubs and failing tests**

In `crates/livebrainstorm-core/src/usage.rs`, change two `use` lines at the top:

```rust
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
```

```rust
use crate::llm::{ChatRequest, Chunk, OnChunk, Provider, Usage};
```

Add this section after the `impl UsageLedger` block (before `#[cfg(test)]`):

```rust
// ---------- metering one call ----------

/// Wraps one model call. A `Chunk::Usage` is recorded exactly; a call that
/// succeeds without one, or is dropped mid-stream (a keystroke aborted it), is
/// estimated from its characters; a call that fails without one records nothing.
pub struct Meter {
    ledger: Arc<UsageLedger>,
    tag: UsageTag,
    model: String,
    sent_chars: usize,
    received_chars: usize,
    /// A `Chunk::Usage` arrived and was recorded.
    reported: bool,
    /// `finish` ran, so `Drop` adds nothing.
    settled: bool,
}

impl Meter {
    pub fn new(ledger: Arc<UsageLedger>, tag: UsageTag, model: String, sent_chars: usize) -> Self {
        Self { ledger, tag, model, sent_chars, received_chars: 0, reported: false, settled: false }
    }

    pub fn observe(&mut self, _chunk: &Chunk) {
        unimplemented!()
    }

    pub fn finish(self, _ok: bool) {
        unimplemented!()
    }
}

/// Characters sent: `system` + `user`, as Unicode scalars (not bytes).
pub fn sent_chars(req: &ChatRequest) -> usize {
    req.system.chars().count() + req.user.chars().count()
}

pub async fn metered(
    _provider: &Provider,
    _req: &ChatRequest,
    _ledger: Arc<UsageLedger>,
    _tag: UsageTag,
    _on_chunk: OnChunk<'_>,
) -> Result<String> {
    unimplemented!()
}
```

In `mod tests`, `Arc`, `AtomicUsize` and `Ordering` are already imported (Task 5), and `Provider` comes in through `use super::*;`. Add these imports and tests after the ledger tests:

```rust
    use std::time::Duration;

    use crate::jev::test_server::{serve, Canned};
    use crate::llm::OpenAiCompat;

    fn shared() -> (tempfile::TempDir, Arc<UsageLedger>) {
        let (dir, l) = ledger();
        (dir, Arc::new(l))
    }

    /// Every bucket summed, so a test that straddles an hour boundary still passes.
    fn total(l: &UsageLedger) -> UsageBucket {
        let all = l.report(0);
        let mut sum = all.first().cloned().expect("something was recorded");
        for b in &all[1..] {
            sum.merge(b);
        }
        sum
    }

    #[test]
    fn a_meter_dropped_mid_call_records_one_estimate() {
        let (_dir, l) = shared();
        let mut m = Meter::new(l.clone(), tag("/a", UsageKind::Notes), "claude-haiku-5-5".into(), 4001);
        m.observe(&Chunk::Text("éééé".into())); // 4 characters, 8 bytes: one token
        m.observe(&Chunk::Activity("Read src/main.rs".into()));
        drop(m);
        let b = total(&l);
        assert_eq!((b.input, b.output, b.runs, b.estimated_runs), (1001, 1, 1, 1), "ceil(4001 / 4) and ceil(4 / 4)");
        let expected = (1001.0 * 0.10 + 1.0 * 0.50) / 1e6;
        assert!((b.cost_usd - expected).abs() < 1e-15);
        assert!((b.estimated_cost_usd - expected).abs() < 1e-15);
    }

    #[test]
    fn finishing_without_usage_estimates_only_when_the_call_succeeded() {
        let (_dir, l) = shared();
        Meter::new(l.clone(), tag("/a", UsageKind::Notes), "haiku".into(), 40).finish(true);
        Meter::new(l.clone(), tag("/b", UsageKind::Notes), "haiku".into(), 40).finish(false);
        let b = total(&l);
        assert_eq!((b.runs, b.estimated_runs, b.input), (1, 1, 10), "a failed call without usage is assumed unbilled");
        assert_eq!(l.report(0)[0].root.as_deref(), Some("/a"));
    }

    #[test]
    fn reported_usage_is_recorded_exactly_once() {
        let (_dir, l) = shared();
        let exact = Usage { model: "claude-haiku-5-5".into(), input: 2, cache_write: 2198, cache_read: 535, output: 45, cost_usd: Some(0.00046765) };
        let meter = || Meter::new(l.clone(), tag("/a", UsageKind::Notes), "haiku".into(), 4000);
        let mut m = meter();
        m.observe(&Chunk::Usage(exact.clone()));
        m.finish(true);
        let mut m = meter();
        m.observe(&Chunk::Usage(exact.clone()));
        drop(m);
        let mut m = meter();
        m.observe(&Chunk::Usage(exact));
        m.finish(false); // a failed run whose result still carried usage
        let b = total(&l);
        assert_eq!((b.runs, b.estimated_runs, b.input, b.cache_write), (3, 0, 6, 3 * 2198));
        assert!((b.cost_usd - 3.0 * 0.00046765).abs() < 1e-15);
    }

    #[test]
    fn an_estimate_for_an_unpriced_model_counts_tokens_but_no_cost() {
        let (_dir, l) = shared();
        drop(Meter::new(l.clone(), tag("/a", UsageKind::Notes), "claude-sonnet-9".into(), 400));
        let b = total(&l);
        assert_eq!((b.input, b.runs, b.unpriced_runs, b.cost_usd), (100, 1, 1, 0.0));
    }

    #[test]
    fn sent_chars_counts_characters_not_bytes() {
        let req = ChatRequest { system: "ééé".into(), user: "ab".into(), max_tokens: 1, cwd: None, allow_repo_tools: false, max_turns: None };
        assert_eq!(sent_chars(&req), 5);
    }

    fn local(base_url: String, model: &str) -> Provider {
        Provider::OpenAiCompat(OpenAiCompat { base_url, model: model.into(), context_tokens: 4096, http: reqwest::Client::new() })
    }

    /// 400 characters: an estimate of 100 input tokens.
    fn request() -> ChatRequest {
        ChatRequest { system: "s".repeat(40), user: "u".repeat(360), max_tokens: 100, cwd: None, allow_repo_tools: false, max_turns: None }
    }

    const WITH_USAGE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n\
        data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{}}],\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":2}}\n\n\
        data: [DONE]\n\n";
    const WITHOUT_USAGE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n";

    #[tokio::test]
    async fn a_reported_call_is_recorded_exactly_and_never_forwarded() {
        let (url, _) = serve(vec![Canned::new(200, WITH_USAGE)]).await;
        let (_dir, l) = shared();
        let mut seen = Vec::new();
        let provider = local(url, "qwen3-4b-instruct");
        let text = metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |c: Chunk| seen.push(c)).await.unwrap();
        assert_eq!(text, "Hi");
        assert_eq!(seen, vec![Chunk::Text("Hi".into())], "usage is recorded, never shown");
        let b = total(&l);
        assert_eq!((b.input, b.output, b.runs, b.estimated_runs, b.cost_usd), (31, 2, 1, 0, 0.0));
    }

    #[tokio::test]
    async fn a_call_without_usage_is_estimated_from_its_characters() {
        let (url, _) = serve(vec![Canned::new(200, WITHOUT_USAGE)]).await;
        let (_dir, l) = shared();
        let provider = local(url, "claude-haiku-5-5"); // priced, to check the arithmetic
        metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |_: Chunk| {}).await.unwrap();
        let b = total(&l);
        assert_eq!((b.input, b.output, b.estimated_runs), (100, 1, 1), "400 characters sent, 2 received");
        assert!((b.estimated_cost_usd - (100.0 * 0.10 + 1.0 * 0.50) / 1e6).abs() < 1e-15);
    }

    #[tokio::test]
    async fn a_failed_call_without_usage_records_nothing() {
        let (url, _) = serve(vec![Canned::new(500, "{}")]).await;
        let (_dir, l) = shared();
        let provider = local(url, "qwen3-4b-instruct");
        let result = metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |_: Chunk| {}).await;
        assert!(result.is_err());
        assert!(l.report(0).is_empty());
    }

    #[tokio::test]
    async fn a_call_dropped_mid_stream_records_one_estimate() {
        let (url, _) = serve(vec![Canned::new(200, WITH_USAGE).delayed(5_000)]).await;
        let (_dir, l) = shared();
        let (provider, req) = (local(url, "qwen3-4b-instruct"), request());
        let mut sink = |_: Chunk| {};
        let call = metered(&provider, &req, l.clone(), tag("/a", UsageKind::Notes), &mut sink);
        assert!(tokio::time::timeout(Duration::from_millis(200), call).await.is_err(), "the keystroke came first");
        let b = total(&l);
        assert_eq!((b.runs, b.estimated_runs, b.input, b.output, b.cost_usd), (1, 1, 100, 0, 0.0), "local models are free");
    }

    #[tokio::test]
    async fn a_run_cancelled_before_the_request_records_nothing() {
        let (url, hits) = serve(vec![Canned::new(200, WITH_USAGE)]).await;
        let (_dir, l) = shared();
        let provider = local(url, "qwen3-4b-instruct");
        let run = async {
            tokio::time::sleep(Duration::from_secs(5)).await; // the Jev pre-phase
            metered(&provider, &request(), l.clone(), tag("/a", UsageKind::Notes), &mut |_: Chunk| {}).await
        };
        assert!(tokio::time::timeout(Duration::from_millis(100), run).await.is_err());
        assert!(l.report(0).is_empty(), "no meter existed yet");
        assert_eq!(hits.load(Ordering::SeqCst), 0, "and no request was sent");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core usage::tests`
Expected: eight of the ten new tests FAIL. Seven panic with `not implemented`; `an_estimate_for_an_unpriced_model_counts_tokens_but_no_cost` panics with `something was recorded`, because `Meter` has no `Drop` yet. `sent_chars_counts_characters_not_bytes` and `a_run_cancelled_before_the_request_records_nothing` already pass (the latter never reaches `metered`). The Task 2 and Task 5 tests still pass.

- [ ] **Step 3: Implement the meter**

In `impl Meter`, replace `observe` and `finish` with the first block below (it also adds the private `record_estimate`). Add the second block, the `Drop` impl, after the `impl Meter` block:

```rust
    pub fn observe(&mut self, chunk: &Chunk) {
        match chunk {
            Chunk::Text(t) => self.received_chars += t.chars().count(),
            Chunk::Activity(_) => {}
            Chunk::Usage(u) => {
                self.ledger.record(&self.tag, u, false);
                self.reported = true;
            }
        }
    }

    /// Ends the call. A call that failed without reporting usage (bad key, CLI
    /// missing) is assumed unbilled.
    pub fn finish(mut self, ok: bool) {
        if ok && !self.reported {
            self.record_estimate();
        }
        self.settled = true;
    }

    /// `ceil(chars / 4)` tokens each way, all input at the input rate, priced by
    /// the model the provider was asked for.
    fn record_estimate(&self) {
        let tokens = |chars: usize| chars.div_ceil(4) as u64;
        let mut u = Usage {
            model: self.model.clone(),
            input: tokens(self.sent_chars),
            cache_write: 0,
            cache_read: 0,
            output: tokens(self.received_chars),
            cost_usd: None,
        };
        u.cost_usd = price(&u.model).map(|p| p.cost(&u));
        self.ledger.record(&self.tag, &u, true);
    }
```

```rust
/// Dropped before `finish`: the call was aborted mid-stream, after the request
/// went out. Records through the ledger's `std` mutex; never awaits.
impl Drop for Meter {
    fn drop(&mut self) {
        if !self.settled && !self.reported {
            self.record_estimate();
        }
    }
}
```

Replace the `metered` stub with:

```rust
/// One model call, metered. The meter is created here, immediately before
/// `provider.stream`, so a run cancelled earlier (during the Jev pre-phase)
/// records nothing; if this future is dropped mid-stream, the meter's `Drop`
/// records an estimate. `Text` and `Activity` go on to `on_chunk`; `Usage` is
/// recorded and not forwarded.
pub async fn metered(
    provider: &Provider,
    req: &ChatRequest,
    ledger: Arc<UsageLedger>,
    tag: UsageTag,
    on_chunk: OnChunk<'_>,
) -> Result<String> {
    let mut meter = Meter::new(ledger, tag, provider.model().to_owned(), sent_chars(req));
    let result = provider
        .stream(req, &mut |chunk: Chunk| {
            meter.observe(&chunk);
            if !matches!(chunk, Chunk::Usage(_)) {
                on_chunk(chunk);
            }
        })
        .await;
    meter.finish(result.is_ok());
    result
}
```

- [ ] **Step 4: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core usage::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: all 23 `usage::tests` pass, the whole core suite passes, and clippy prints no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/usage.rs
git commit -m "feat(core): Meter records exact usage or a character estimate; metered wraps one call

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 7: Shell and IPC: meter every call, `usage_report`, `usage-changed`, saving

**Files:**
- Modify: `src-tauri/src/state.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/events.rs`, `src-tauri/src/lib.rs`
- Modify: `src/lib/ipc.ts`, `src/lib/events.ts`

**Interfaces:**
- Consumes:
  - `UsageLedger::{new(path: PathBuf) -> Self, set_on_record(&self, f: Box<dyn Fn() + Send + Sync>), report(&self, since_hour: u64) -> Vec<UsageBucket>, flush_if_dirty(&self) -> Result<()>, flush(&self) -> Result<()>}` (Task 5)
  - `usage::metered(provider: &Provider, req: &ChatRequest, ledger: Arc<UsageLedger>, tag: UsageTag, on_chunk: OnChunk<'_>) -> Result<String>` (Task 6)
  - `UsageTag { root: Option<String>, kind: UsageKind }` (Clone), `UsageKind::{Notes, Brief}` (Task 2)
  - From Phase 1:
    - `analyze`: the command body binds `let root = app.state::<AppState>().sessions.open_tab_root(&tab_id)?;` (`Option<String>`) before spawning and moves it into the task; the task binds `let state = task_app.state::<AppState>();` and keeps the lane loop (`let groups = prompt::section_groups(settings.effective_note_agents());`, then per lane `let (provider, on_event) = (&provider, &on_event);`, `lanes.push(async move { let mut sink = |chunk: Chunk| match chunk { … }; let result = provider.stream(&req, &mut sink).await; … })`).
    - `build_brief(app, root: String, on_event)`: the task binds `task_app`, `task_root` and `index: Arc<ProjectIndex>` (`ProjectIndex::root: PathBuf`, canonical), and inside `let result: Result<StoredBrief> = async { … }` makes the three `provider.stream` calls.
    - `src-tauri/src/events.rs` (event-name constants; it imports `tauri::{AppHandle, Emitter}`), declared `mod events;` in `lib.rs`.
    - `src/lib/events.ts`, which imports `listen` and `type UnlistenFn` from `@tauri-apps/api/event`.
  - `AppState` (Phase 1): `pub fn new(data_dir: &Path, config_dir: &Path) -> Self`, fields `projects`, `analyses`, `brief_tasks`, `sessions`, `downloads`, `local`; `Arc` is already imported in `state.rs`.
- Produces:
  - `AppState.usage: Arc<UsageLedger>`, stored at `<app_data>/usage.json`
  - `pub const USAGE_CHANGED: &str = "usage-changed";` in `src-tauri/src/events.rs` (no payload)
  - Command `usage_report(since_hour: u64) -> Vec<UsageBucket>`
  - TS: `export type UsageKind = "notes" | "brief"`, `export interface UsageBucket { hour: number; root: string | null; kind: UsageKind; model: string; input: number; cacheWrite: number; cacheRead: number; output: number; costUsd: number; runs: number; estimatedRuns: number; estimatedCostUsd: number; unpricedRuns: number }`, `api.usageReport(sinceHour: number): Promise<UsageBucket[]>`
  - TS: `export function onUsageChanged(fn: () => void): Promise<UnlistenFn>` in `src/lib/events.ts`

`src-tauri` has no tests. This task is verified by `cargo check`, clippy, the typecheck and vitest; the user checks the running app in Task 9.

- [ ] **Step 1: State**

In `src-tauri/src/state.rs`, add `use livebrainstorm_core::usage::UsageLedger;` to the imports (`Arc` is already imported). Add this field after `sessions`:

```rust
    /// Every model call's tokens and cost (`usage.json`); shared with each `Meter`.
    pub usage: Arc<UsageLedger>,
```

In `AppState::new`, after the `sessions: …,` line, add:

```rust
            usage: Arc::new(UsageLedger::new(data_dir.join("usage.json"))),
```

- [ ] **Step 2: Meter the notes lanes**

In `src-tauri/src/commands.rs`, add to the imports:

```rust
use livebrainstorm_core::usage::{self, UsageBucket, UsageKind, UsageTag};
```

In `analyze`'s spawned task, just before the lane loop (`let groups = prompt::section_groups(…);`), add:

```rust
        // Each lane is metered from the moment its request starts (after the
        // pre-phase), so a keystroke during the Jev gate records nothing.
        let ledger = state.usage.clone();
        let tag = UsageTag { root: root.clone(), kind: UsageKind::Notes };
```

`root` is the tab's `Option<String>` that Phase 1's command body binds from `open_tab_root(&tab_id)` and moves into the task (its `match root.as_deref()` only borrows it, so it's still there); if Phase 1 named it differently, use that name.

Inside the lane loop, after `let (provider, on_event) = (&provider, &on_event);`, add:

```rust
            let (ledger, tag) = (ledger.clone(), tag.clone());
```

and in the lane's `async move` block replace

```rust
                let result = provider.stream(&req, &mut sink).await;
```

with

```rust
                let result = usage::metered(provider, &req, ledger, tag, &mut sink).await;
```

The `sink` closure (with its `Chunk::Usage(_) => {}` arm from Task 2) and everything after this line stay as they are.

- [ ] **Step 3: Meter the brief explorers and the merge**

In `build_brief`'s spawned task, inside the `let result: Result<StoredBrief> = async { … }` block, just before `let (text, agents) = if slices.len() <= 1 {`, add:

```rust
            let ledger = task_app.state::<AppState>().usage.clone();
            let tag = UsageTag { root: Some(index.root.display().to_string()), kind: UsageKind::Brief };
```

`index.root.display().to_string()` is the canonical root, the same string as `ProjectSummary.root`, the registry key and the root of every tab opened on that project, so a project's notes and brief runs land on one panel line each. (If Phase 1's task binds the app handle under another name than `task_app`, use it.) Then change the three calls:

1. The single explorer:

   ```rust
                   (usage::metered(&provider, &req, ledger, tag, &mut on_chunk(&on_event)).await?, 1)
   ```

2. Each explorer in the loop: after `let (provider, on_event) = (&provider, &on_event);` add `let (ledger, tag) = (ledger.clone(), tag.clone());`, and replace `provider.stream(&req, &mut sink).await` with:

   ```rust
                       usage::metered(provider, &req, ledger, tag, &mut sink).await
   ```

3. The merge:

   ```rust
                   (usage::metered(&provider, &req, ledger, tag, &mut on_chunk(&on_event)).await?, n)
   ```

The explorer loop must stay a `for` loop: closures over borrowed items inside this spawned task trip rustc's Send check.

- [ ] **Step 4: The `usage_report` command**

In `src-tauri/src/commands.rs`, add a section before `// ---------- local models ----------`:

```rust
// ---------- usage ----------

/// The ledger's hourly buckets (UTC) from `since_hour` on; the UI groups them
/// by the user's local day.
#[tauri::command]
pub fn usage_report(state: State<'_, AppState>, since_hour: u64) -> Vec<UsageBucket> {
    state.usage.report(since_hour)
}
```

- [ ] **Step 5: The event, the save loop and the save on exit**

In `src-tauri/src/events.rs`, add next to the other constants (the contract reserves this name for Phase 4; if an earlier phase already added it, keep that one):

```rust
/// After a ledger record, at most once a second. No payload: windows refetch `usage_report`.
pub const USAGE_CHANGED: &str = "usage-changed";
```

In `src-tauri/src/lib.rs`, add these imports above the existing `use tauri::…;` line:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
```

Phases 1–3 leave `use tauri::{Manager, RunEvent};` there (Phase 3 refers to `std::sync::atomic::Ordering` by its full path, so nothing clashes). Change that line to `use tauri::{AppHandle, Emitter, Manager, RunEvent};`, keeping any other name an earlier phase added to it. Skip any `std` import that's already there: a name imported twice is a compile error. Add this function after `run`:

```rust
/// Emits `usage-changed` at most once a second after a record, and saves the
/// ledger every 5 s when it changed. Ledger failures are logged once per
/// streak and never reach an analysis.
fn start_usage_tasks(app: &AppHandle) {
    let ledger = app.state::<AppState>().usage.clone();
    let changed = Arc::new(AtomicBool::new(false));
    let flag = changed.clone();
    ledger.set_on_record(Box::new(move || flag.store(true, Ordering::Relaxed)));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let (mut seconds, mut failing) = (0u32, false);
        loop {
            tick.tick().await;
            if changed.swap(false, Ordering::Relaxed) {
                let _ = app.emit(events::USAGE_CHANGED, ());
            }
            seconds += 1;
            if seconds < 5 {
                continue;
            }
            seconds = 0;
            match ledger.flush_if_dirty() {
                Ok(()) => failing = false,
                Err(e) if !failing => {
                    eprintln!("Couldn't save usage.json: {e}");
                    failing = true;
                }
                Err(_) => {}
            }
        }
    });
}
```

In `setup`, right after `app.manage(AppState::new(&data_dir, &config_dir));`, add:

```rust
            start_usage_tasks(app.handle());
```

In the `.run(|app, event| …)` closure, inside the `RunEvent::Exit` handling, after the local server is stopped, add:

```rust
                // The last few seconds of usage, and the 90-day prune.
                if let Err(e) = app.state::<AppState>().usage.flush() {
                    eprintln!("Couldn't save usage.json: {e}");
                }
```

In `generate_handler![…]`, add `commands::usage_report,` after `commands::cancel_analysis,`.

- [ ] **Step 6: Check the shell**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: compiles with no warnings. A Send error here ("FnOnce is not general enough" or "future cannot be sent between threads safely") means a closure over a borrowed argument crept into a future awaited in the spawned task; keep the `for` loops and the `(ledger.clone(), tag.clone())` bindings as written.

- [ ] **Step 7: IPC types, the wrapper and the event**

In `src/lib/ipc.ts`, add after the `LocalStatus` interface:

```ts
/** Mirrors `usage::UsageKind`. */
export type UsageKind = "notes" | "brief";

/** One hour (UTC: unix seconds / 3600) of one project's usage for one kind and model (`usage::UsageBucket`). */
export interface UsageBucket {
  hour: number;
  root: string | null;
  kind: UsageKind;
  model: string;
  input: number;
  cacheWrite: number;
  cacheRead: number;
  output: number;
  /** The total cost, estimates included; 0 for runs with no known price. */
  costUsd: number;
  runs: number;
  estimatedRuns: number;
  /** The part of `costUsd` that came from estimates (runs cut short, or that reported no usage). */
  estimatedCostUsd: number;
  unpricedRuns: number;
}
```

In `api`, add after `cancelAnalysis`:

```ts
  usageReport: (sinceHour: number) => invoke<UsageBucket[]>("usage_report", { sinceHour }),
```

In `src/lib/events.ts`, add after the other wrappers:

```ts
/** After a run is recorded in the usage ledger, at most once a second, in every window. */
export function onUsageChanged(fn: () => void): Promise<UnlistenFn> {
  return listen("usage-changed", () => fn());
}
```

- [ ] **Step 8: Typecheck and test**

Run: `npm run typecheck && npm test && cargo test -p livebrainstorm-core`
Expected: no type errors, and every vitest and core test passes.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src src/lib/ipc.ts src/lib/events.ts
git commit -m "feat: meter every notes lane, explorer and merge; usage_report and usage-changed

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 8: Frontend: usage helpers (local days, totals, formatting)

**Files:**
- Create: `src/lib/usage.ts`
- Test: `src/lib/usage.test.ts`

**Interfaces:**
- Consumes, from `src/lib/ipc.ts` (Task 7):
  ```ts
  export type UsageKind = "notes" | "brief";
  export interface UsageBucket { hour: number; root: string | null; kind: UsageKind; model: string; input: number;
    cacheWrite: number; cacheRead: number; output: number; costUsd: number; runs: number; estimatedRuns: number;
    estimatedCostUsd: number; unpricedRuns: number }   // hour = unix seconds / 3600, UTC
  ```
- Produces, all exported from `src/lib/usage.ts`:
  ```ts
  export const DAYS_SHOWN = 30;
  export type Offset = (ms: number) => number;            // minutes east of UTC at that instant
  export const localOffset: Offset;                        // -new Date(ms).getTimezoneOffset()
  export interface UsageTotals { tokens: number; costUsd: number; estimatedCostUsd: number; runs: number;
                                 estimatedRuns: number; unpricedRuns: number }
  export interface UsageLine extends UsageTotals { root: string | null; kind: UsageKind }
  export interface DayUsage extends UsageTotals { day: string; lines: UsageLine[] }   // day: local "2026-10-14"
  export function bucketTotals(b: UsageBucket): UsageTotals;
  export function sumTotals(items: UsageTotals[]): UsageTotals;
  export function sinceHourFor(now: number, days: number, offset?: Offset): number;
  /** Exactly DAYS_SHOWN entries, today first; a day with no runs has zero totals and no lines. */
  export function groupByLocalDay(buckets: UsageBucket[], now: number, offset?: Offset): DayUsage[];
  export function todayTotals(buckets: UsageBucket[], now: number, offset?: Offset): UsageTotals;
  export function formatTokens(n: number): string;         // "950", "1.2k", "182k", "1.2M"
  export function formatCost(usd: number, estimated: boolean): string;   // "$0.41", "~$0.41", "$0.0005"
  export function costText(t: UsageTotals): string;        // "—" when no run had a price
  /** A panel row's cells; "—" in all three when the row has no runs. */
  export function rowCells(t: UsageTotals): { tokens: string; cost: string; runs: string };
  export function usageNotes(t: UsageTotals): string[];    // "incl. ~$0.03 from 12 cancelled runs", "price unknown for 2 runs"
  export function footerText(t: UsageTotals): string;      // "Today · 182k tokens · ~$0.41"
  export function projectName(root: string | null): string;
  export function lineLabel(l: UsageLine): string;         // "shop-api · notes"
  export function formatDay(day: string): string;          // "Wed 14 Oct" in the user's locale
  ```
  `offset` defaults to `localOffset`; tests pass a fixed one, so they don't depend on the machine's time zone.

- [ ] **Step 1: Write the failing tests**

Create `src/lib/usage.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { UsageBucket } from "./ipc";
import {
  costText,
  footerText,
  formatCost,
  formatTokens,
  groupByLocalDay,
  lineLabel,
  projectName,
  rowCells,
  sinceHourFor,
  todayTotals,
  usageNotes,
  type DayUsage,
  type UsageTotals,
} from "./usage";

const HOUR = 3_600_000;
const UTC = () => 0;
const PDT = () => -420; // California in summer
const IST = () => 330; // India: a half-hour offset
const NOW = Date.parse("2026-10-14T20:00:00Z");

const bucket = (iso: string, fields: Partial<UsageBucket> = {}): UsageBucket => ({
  hour: Date.parse(iso) / HOUR,
  root: "/Users/me/code/shop-api",
  kind: "notes",
  model: "claude-haiku-5-5",
  input: 100,
  cacheWrite: 0,
  cacheRead: 0,
  output: 50,
  costUsd: 0.01,
  runs: 1,
  estimatedRuns: 0,
  estimatedCostUsd: 0,
  unpricedRuns: 0,
  ...fields,
});

const totals = (fields: Partial<UsageTotals>): UsageTotals => ({
  tokens: 0,
  costUsd: 0,
  estimatedCostUsd: 0,
  runs: 0,
  estimatedRuns: 0,
  unpricedRuns: 0,
  ...fields,
});

/** The days that had runs; the panel lists the quiet days too. */
const busy = (days: DayUsage[]) => days.filter((d) => d.runs > 0);

describe("groupByLocalDay", () => {
  it("puts each hour on the user's calendar day, not UTC's", () => {
    const b = [bucket("2026-10-14T05:00:00Z"), bucket("2026-10-14T08:00:00Z")];
    expect(busy(groupByLocalDay(b, NOW, UTC)).map((d) => d.day)).toEqual(["2026-10-14"]);
    // 05:00Z is 22:00 the evening before in California; 08:00Z is 01:00 the same day.
    expect(busy(groupByLocalDay(b, NOW, PDT)).map((d) => [d.day, d.runs])).toEqual([
      ["2026-10-14", 1],
      ["2026-10-13", 1],
    ]);
  });

  it("handles half-hour offsets", () => {
    // 20:00Z is 01:30 the next morning in India, where it's already the 15th at NOW.
    const days = groupByLocalDay([bucket("2026-10-13T20:00:00Z")], NOW, IST);
    expect(days[0].day).toBe("2026-10-15");
    expect(busy(days).map((d) => d.day)).toEqual(["2026-10-14"]);
  });

  it("splits a day by project and kind, merging models, largest first", () => {
    const [today, ...rest] = groupByLocalDay(
      [
        bucket("2026-10-14T09:00:00Z", { input: 10, output: 10 }),
        bucket("2026-10-14T10:00:00Z", { model: "claude-sonnet-5-5", input: 1000, cacheRead: 500, output: 100, costUsd: 0.2 }),
        bucket("2026-10-14T10:00:00Z", { kind: "brief", input: 5000, output: 900, costUsd: 0.5 }),
        bucket("2026-10-14T11:00:00Z", { root: null, input: 1, output: 1, costUsd: 0 }),
      ],
      NOW,
      UTC,
    );
    expect(today.day).toBe("2026-10-14");
    expect(busy(rest)).toEqual([]);
    expect(today.lines.map((l) => [l.root, l.kind, l.tokens, l.runs])).toEqual([
      ["/Users/me/code/shop-api", "brief", 5900, 1],
      ["/Users/me/code/shop-api", "notes", 1620, 2],
      [null, "notes", 2, 1],
    ]);
    expect(today.tokens).toBe(7522);
    expect(today.runs).toBe(4);
    expect(today.costUsd).toBeCloseTo(0.71, 10);
  });

  it("lists every one of the last 30 local days, today first, quiet days included", () => {
    const days = groupByLocalDay(
      [
        bucket("2026-09-14T12:00:00Z"), // 30 days back: dropped
        bucket("2026-09-15T12:00:00Z"), // 29 days back: kept
        bucket("2026-10-12T12:00:00Z"),
        bucket("2026-10-15T12:00:00Z"), // after today (a clock change): dropped
      ],
      NOW,
      UTC,
    );
    expect(days).toHaveLength(30);
    expect([days[0].day, days[1].day, days[29].day]).toEqual(["2026-10-14", "2026-10-13", "2026-09-15"]);
    expect(busy(days).map((d) => d.day)).toEqual(["2026-10-12", "2026-09-15"]);
    expect(days[1]).toEqual({ day: "2026-10-13", ...totals({}), lines: [] });
    expect(groupByLocalDay([], NOW, UTC).every((d) => d.runs === 0 && d.lines.length === 0)).toBe(true);
  });
});

describe("todayTotals", () => {
  it("sums today's local hours, cache tokens included", () => {
    const b = [
      bucket("2026-10-14T05:00:00Z", { input: 1200, cacheWrite: 22000, cacheRead: 90000, output: 3100, estimatedRuns: 1 }),
      bucket("2026-10-14T08:00:00Z", { input: 100, output: 0 }),
    ];
    expect(todayTotals(b, NOW, UTC)).toMatchObject({ tokens: 116400, runs: 2, estimatedRuns: 1 });
    expect(todayTotals(b, NOW, PDT)).toMatchObject({ tokens: 100, runs: 1, estimatedRuns: 0 });
    expect(todayTotals([], NOW, UTC)).toEqual(totals({}));
  });
});

describe("sinceHourFor", () => {
  it("starts an hour before local midnight, so the first local hour is never missed", () => {
    expect(sinceHourFor(NOW, 1, UTC)).toBe(Date.parse("2026-10-13T23:00:00Z") / HOUR);
    expect(sinceHourFor(NOW, 1, PDT)).toBe(Date.parse("2026-10-14T06:00:00Z") / HOUR);
    expect(sinceHourFor(NOW, 1, IST)).toBe(Date.parse("2026-10-14T17:00:00Z") / HOUR);
    expect(sinceHourFor(NOW, 30, UTC)).toBe(Date.parse("2026-09-14T23:00:00Z") / HOUR);
  });
});

describe("formatting", () => {
  it("formats token counts like 182k and 1.2M", () => {
    const cases: [number, string][] = [
      [0, "0"],
      [950, "950"],
      [1000, "1k"],
      [1234, "1.2k"],
      [9960, "10k"],
      [182_400, "182k"],
      [999_499, "999k"],
      [999_500, "1M"],
      [1_234_567, "1.2M"],
      [12_345_678, "12M"],
    ];
    for (const [n, text] of cases) expect(formatTokens(n), String(n)).toBe(text);
  });

  it("formats costs, with ~ for estimates and four places below a cent", () => {
    expect(formatCost(0.41, false)).toBe("$0.41");
    expect(formatCost(0.41, true)).toBe("~$0.41");
    expect(formatCost(0, false)).toBe("$0.00");
    expect(formatCost(0.00046765, false)).toBe("$0.0005");
    expect(formatCost(12.3456, false)).toBe("$12.35");
  });

  it("shows a dash when no run has a price", () => {
    expect(costText(totals({ runs: 3, unpricedRuns: 3 }))).toBe("—");
    expect(costText(totals({ runs: 3, unpricedRuns: 1, costUsd: 0.2 }))).toBe("$0.20");
    expect(costText(totals({ runs: 3, costUsd: 0.41, estimatedRuns: 1, estimatedCostUsd: 0.03 }))).toBe("~$0.41");
    expect(costText(totals({}))).toBe("$0.00");
  });

  it("shows a dash in every column of a row with no runs", () => {
    expect(rowCells(totals({}))).toEqual({ tokens: "—", cost: "—", runs: "—" });
    expect(rowCells(totals({ tokens: 182_400, costUsd: 0.41, runs: 14 }))).toEqual({ tokens: "182k", cost: "$0.41", runs: "14" });
    expect(rowCells(totals({ tokens: 900, runs: 2, unpricedRuns: 2 }))).toEqual({ tokens: "900", cost: "—", runs: "2" });
  });

  it("writes the footer line", () => {
    const t = totals({ tokens: 182_400, costUsd: 0.41, estimatedCostUsd: 0.03, runs: 14, estimatedRuns: 3 });
    expect(footerText(t)).toBe("Today · 182k tokens · ~$0.41");
  });

  it("labels estimates and unpriced runs", () => {
    expect(usageNotes(totals({ runs: 20, estimatedRuns: 12, estimatedCostUsd: 0.03 }))).toEqual(["incl. ~$0.03 from 12 cancelled runs"]);
    expect(usageNotes(totals({ runs: 3, estimatedRuns: 1, estimatedCostUsd: 0.0004, unpricedRuns: 2 }))).toEqual([
      "incl. ~$0.0004 from 1 cancelled run",
      "price unknown for 2 runs",
    ]);
    expect(usageNotes(totals({ runs: 5, costUsd: 1 }))).toEqual([]);
  });

  it("names projects by their folder", () => {
    expect(projectName("/Users/me/code/shop-api")).toBe("shop-api");
    expect(projectName("/Users/me/code/shop-api/")).toBe("shop-api");
    expect(projectName("C:\\code\\billing")).toBe("billing");
    expect(projectName(null)).toBe("No project");
    expect(lineLabel({ ...totals({}), root: "/code/shop-api", kind: "brief" })).toBe("shop-api · brief");
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/usage.test.ts`
Expected: FAIL with `Failed to resolve import "./usage"`.

- [ ] **Step 3: Implement**

Create `src/lib/usage.ts`:

```ts
// The usage ledger keeps hourly UTC buckets. Days are the user's own, so
// grouping by day happens here, on the user's clock, not in Rust.
import type { UsageBucket, UsageKind } from "./ipc";

const HOUR = 3_600_000;
const DAY = 86_400_000;

/** How many local days the usage panel covers, today included. */
export const DAYS_SHOWN = 30;

/** Minutes east of UTC at instant `ms`: +330 in India, -420 in California in summer. */
export type Offset = (ms: number) => number;

/** The user's own offset, daylight saving included. */
export const localOffset: Offset = (ms) => -new Date(ms).getTimezoneOffset();

export interface UsageTotals {
  /** Input + cache write + cache read + output. */
  tokens: number;
  /** Estimates included; 0 for runs with no known price. */
  costUsd: number;
  estimatedCostUsd: number;
  runs: number;
  estimatedRuns: number;
  unpricedRuns: number;
}

/** One project and kind within a day ("shop-api · notes"), all models merged. */
export interface UsageLine extends UsageTotals {
  root: string | null;
  kind: UsageKind;
}

export interface DayUsage extends UsageTotals {
  /** The local calendar date, e.g. "2026-10-14". */
  day: string;
  lines: UsageLine[];
}

const ZERO: UsageTotals = { tokens: 0, costUsd: 0, estimatedCostUsd: 0, runs: 0, estimatedRuns: 0, unpricedRuns: 0 };

function plus(a: UsageTotals, b: UsageTotals): UsageTotals {
  return {
    tokens: a.tokens + b.tokens,
    costUsd: a.costUsd + b.costUsd,
    estimatedCostUsd: a.estimatedCostUsd + b.estimatedCostUsd,
    runs: a.runs + b.runs,
    estimatedRuns: a.estimatedRuns + b.estimatedRuns,
    unpricedRuns: a.unpricedRuns + b.unpricedRuns,
  };
}

export function bucketTotals(b: UsageBucket): UsageTotals {
  return {
    tokens: b.input + b.cacheWrite + b.cacheRead + b.output,
    costUsd: b.costUsd,
    estimatedCostUsd: b.estimatedCostUsd,
    runs: b.runs,
    estimatedRuns: b.estimatedRuns,
    unpricedRuns: b.unpricedRuns,
  };
}

export function sumTotals(items: UsageTotals[]): UsageTotals {
  return items.reduce(plus, ZERO);
}

/** Days since the epoch on the user's calendar. A bucket belongs to the day its hour starts in. */
function localDay(ms: number, offset: Offset): number {
  return Math.floor((ms + offset(ms) * 60_000) / DAY);
}

const isoDay = (day: number) => new Date(day * DAY).toISOString().slice(0, 10);

/** The first hour to ask `usage_report` for, covering `days` local days up to today. One hour early, so a daylight-saving change never cuts off the first local hour; the helpers below filter exactly. */
export function sinceHourFor(now: number, days: number, offset: Offset = localOffset): number {
  const midnight = (localDay(now, offset) - (days - 1)) * DAY - offset(now) * 60_000;
  return Math.floor(midnight / HOUR) - 1;
}

/** The usage panel's rows: every one of the last `DAYS_SHOWN` local days, today first, including days with no runs
 *  (all zeros, no lines). Each day is split by project and kind, largest first. */
export function groupByLocalDay(buckets: UsageBucket[], now: number, offset: Offset = localOffset): DayUsage[] {
  const today = localDay(now, offset);
  const days = new Map<number, Map<string, UsageLine>>();
  for (const b of buckets) {
    const day = localDay(b.hour * HOUR, offset);
    if (day > today || day <= today - DAYS_SHOWN) continue;
    const lines = days.get(day) ?? new Map<string, UsageLine>();
    days.set(day, lines);
    const key = JSON.stringify([b.root, b.kind]);
    const line = lines.get(key) ?? { root: b.root, kind: b.kind, ...ZERO };
    lines.set(key, { root: line.root, kind: line.kind, ...plus(line, bucketTotals(b)) });
  }
  return Array.from({ length: DAYS_SHOWN }, (_, back) => {
    const day = today - back;
    const list = [...(days.get(day)?.values() ?? [])].sort((x, y) => y.tokens - x.tokens);
    return { day: isoDay(day), ...sumTotals(list), lines: list };
  });
}

/** Today's totals on the user's clock, for the sidebar footer. */
export function todayTotals(buckets: UsageBucket[], now: number, offset: Offset = localOffset): UsageTotals {
  const today = localDay(now, offset);
  return sumTotals(buckets.filter((b) => localDay(b.hour * HOUR, offset) === today).map(bucketTotals));
}

/** "950", "1.2k", "182k", "1.2M". */
export function formatTokens(n: number): string {
  if (n < 1000) return String(Math.round(n));
  const [value, unit]: [number, string] = n < 999_500 ? [n / 1000, "k"] : [n / 1_000_000, "M"];
  const text = value < 9.95 ? value.toFixed(1).replace(/\.0$/, "") : String(Math.round(value));
  return text + unit;
}

/** "$0.41", "~$0.41" for an estimate, and four places below a cent ("$0.0005"), where one run usually costs. */
export function formatCost(usd: number, estimated: boolean): string {
  const text = usd === 0 ? "$0.00" : usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
  return estimated ? `~${text}` : text;
}

/** The cost to show: "—" when no run had a known price, "~" when estimates are part of it. */
export function costText(t: UsageTotals): string {
  if (t.runs > 0 && t.unpricedRuns === t.runs) return "—";
  return formatCost(t.costUsd, t.estimatedCostUsd > 0);
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** What's behind a cost: "incl. ~$0.03 from 12 cancelled runs", "price unknown for 2 runs". */
export function usageNotes(t: UsageTotals): string[] {
  const notes: string[] = [];
  if (t.estimatedRuns > 0) notes.push(`incl. ${formatCost(t.estimatedCostUsd, true)} from ${plural(t.estimatedRuns, "cancelled run")}`);
  if (t.unpricedRuns > 0) notes.push(`price unknown for ${plural(t.unpricedRuns, "run")}`);
  return notes;
}

/** A usage panel row's cells. A row with no runs (a quiet day) shows "—" in every column. */
export function rowCells(t: UsageTotals): { tokens: string; cost: string; runs: string } {
  if (t.runs === 0) return { tokens: "—", cost: "—", runs: "—" };
  return { tokens: formatTokens(t.tokens), cost: costText(t), runs: String(t.runs) };
}

/** "Today · 182k tokens · ~$0.41". */
export function footerText(t: UsageTotals): string {
  return `Today · ${formatTokens(t.tokens)} tokens · ${costText(t)}`;
}

/** The project's folder name; "No project" for tabs without one. */
export function projectName(root: string | null): string {
  if (root === null) return "No project";
  return root.split(/[\\/]/).filter(Boolean).pop() ?? root;
}

/** "shop-api · notes". */
export function lineLabel(l: UsageLine): string {
  return `${projectName(l.root)} · ${l.kind}`;
}

/** "Wed 14 Oct" in the user's locale. `day` is already a local date, so it's formatted as UTC. */
export function formatDay(day: string): string {
  return new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", timeZone: "UTC" });
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npx vitest run src/lib/usage.test.ts && npm run typecheck && npm test`
Expected: the 13 usage tests PASS, no type errors, and the whole vitest suite passes.

- [ ] **Step 5: Commit**

```bash
git add src/lib/usage.ts src/lib/usage.test.ts
git commit -m "feat(ui): usage helpers: local-day grouping, today's totals, token and cost formatting

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 9: The usage footer and panel, wired into the app; docs

**Files:**
- Create: `src/components/UsageFooter.tsx`, `src/components/UsageDialog.tsx`
- Modify: `src/App.tsx` (footer into `Sidebar`, the dialog, the Esc guard)
- Modify: `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes:
  - `api.usageReport(sinceHour: number): Promise<UsageBucket[]>`, `errorMessage(e: unknown): string` (`src/lib/ipc.ts`, Task 7 / today)
  - `onUsageChanged(fn: () => void): Promise<UnlistenFn>` (`src/lib/events.ts`, Task 7)
  - From `src/lib/usage.ts` (Task 8), with the signatures in Task 8's Interfaces: `DAYS_SHOWN`, `sinceHourFor(now, days)`, `groupByLocalDay(buckets, now): DayUsage[]` (exactly 30 days, today first), `todayTotals(buckets, now)`, `sumTotals(items)`, `rowCells(t): { tokens; cost; runs }`, `usageNotes(t): string[]`, `footerText(t)`, `lineLabel(l)`, `formatDay(day)`, `type UsageTotals`
  - From `src/components/ui.tsx`: `Button(props: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "primary" | "ghost" | "link" | "mini" })` (Phase 2 added `mini`) and `cx(...parts: (string | false | null | undefined)[]): string`
  - Phase 2's `Sidebar` props include `footer?: ReactNode`, rendered above the provider label. Phase 2's `App.tsx` renders `<Sidebar hidden={collapsed} opening={opening} providerLabel={…} onOpenProject={…} onShowBrief={showBrief} onBuildBrief={buildBrief} onOpenSettings={…} />` with no `footer`, has `const [showSettings, setShowSettings] = useState(false);`, renders `{showSettings && settings && <SettingsDialog … />}` last inside its root `<div>`, and closes the drawers with a window `keydown` effect whose condition is `e.key === "Escape" && !showSettings` and whose dependency array is `[showSettings]`.
- Produces:
  - `UsageFooter` props `{ onOpen: () => void }`
  - `UsageDialog` props `{ onClose: () => void }`
  - `export const SUBSCRIPTION_NOTE = "On a Claude subscription you aren't billed per token. This is what the same usage would cost on the API."` in `UsageDialog.tsx`

- [ ] **Step 1: Create `src/components/UsageFooter.tsx`**

```tsx
import { useEffect, useState } from "react";
import { api, type UsageBucket } from "../lib/ipc";
import { onUsageChanged } from "../lib/events";
import { footerText, sinceHourFor, todayTotals } from "../lib/usage";
import { Button } from "./ui";

/** "Today · 182k tokens · ~$0.41" in the sidebar footer. Refreshes after each
 *  recorded run (`usage-changed`, in every window) and once a minute, so the
 *  day turns over at local midnight. Opens the usage panel. */
export function UsageFooter({ onOpen }: { onOpen: () => void }) {
  const [view, setView] = useState<{ buckets: UsageBucket[]; at: number } | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () => {
      const at = Date.now();
      // A failed read leaves the last line in place; the ledger never blocks the app.
      api.usageReport(sinceHourFor(at, 1)).then((buckets) => alive && setView({ buckets, at }), () => {});
    };
    load();
    const unlisten = onUsageChanged(load);
    const timer = window.setInterval(load, 60_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
      void unlisten.then((stop) => stop());
    };
  }, []);

  if (!view) return null;
  return (
    <Button variant="ghost" className="w-full justify-start tabular-nums" onClick={onOpen} title="Usage for the last 30 days">
      {footerText(todayTotals(view.buckets, view.at))}
    </Button>
  );
}
```

- [ ] **Step 2: Create `src/components/UsageDialog.tsx`**

```tsx
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { api, errorMessage, type UsageBucket } from "../lib/ipc";
import { onUsageChanged } from "../lib/events";
import {
  DAYS_SHOWN,
  formatDay,
  groupByLocalDay,
  lineLabel,
  rowCells,
  sinceHourFor,
  sumTotals,
  usageNotes,
  type UsageTotals,
} from "../lib/usage";
import { Button, cx } from "./ui";

export const SUBSCRIPTION_NOTE =
  "On a Claude subscription you aren't billed per token. This is what the same usage would cost on the API.";

/** The last 30 local days: a total, then one row per day, today first. A day
 *  with runs expands into its projects and kinds; a quiet day shows "—". A
 *  native modal, like Settings. */
export function UsageDialog({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [view, setView] = useState<{ buckets: UsageBucket[]; at: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  // Native modal: focus trap, Esc (fires `close`) and ::backdrop for free.
  // StrictMode runs this twice, and older WebKit throws on a second showModal().
  useEffect(() => {
    if (dialog.current && !dialog.current.open) dialog.current.showModal();
  }, []);

  useEffect(() => {
    let alive = true;
    const load = () => {
      const at = Date.now();
      api.usageReport(sinceHourFor(at, DAYS_SHOWN)).then(
        (buckets) => {
          if (!alive) return;
          setView({ buckets, at });
          setError(null);
        },
        (e) => alive && setError(errorMessage(e)),
      );
    };
    load();
    const unlisten = onUsageChanged(load);
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);

  const days = view ? groupByLocalDay(view.buckets, view.at) : [];
  const total = sumTotals(days);

  return (
    <dialog
      ref={dialog}
      onClose={onClose}
      // A press on the backdrop targets the <dialog> itself; the content div covers the rest.
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      aria-labelledby="usage-title"
      className="m-auto max-h-[calc(100%-2.5rem)] w-[min(620px,calc(100%-2.5rem))] overflow-y-auto rounded-xl border border-line bg-page text-ink shadow-2xl backdrop:bg-black/40"
    >
      <div className="grid gap-5 p-6">
        <header className="flex items-center justify-between">
          <h2 id="usage-title" className="text-lg font-semibold tracking-tight">
            Usage
          </h2>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </header>

        {error && (
          <p className="text-[13px] text-fault" role="alert">
            {error}
          </p>
        )}

        {view && total.runs === 0 && <p className="text-[13px] text-muted">No usage in the last 30 days.</p>}

        {view && (
          <table className="w-full text-[13px] tabular-nums">
            <thead>
              <tr className="border-b border-line text-left text-xs text-muted">
                <th className="py-1.5 font-normal">Day</th>
                <th className="py-1.5 text-right font-normal">Tokens</th>
                <th className="py-1.5 text-right font-normal">Est. cost</th>
                <th className="py-1.5 text-right font-normal">Runs</th>
              </tr>
            </thead>
            <tbody>
              <Row label="Last 30 days" totals={total} strong />
              {days.map((d) => {
                const open = expanded === d.day;
                return (
                  <Fragment key={d.day}>
                    <Row
                      label={
                        d.lines.length === 0 ? (
                          <span className="flex items-center gap-1.5 text-muted">
                            <span aria-hidden className="w-3" />
                            {formatDay(d.day)}
                          </span>
                        ) : (
                          <button
                            type="button"
                            aria-expanded={open}
                            onClick={() => setExpanded(open ? null : d.day)}
                            className="flex cursor-pointer items-center gap-1.5 text-left hover:text-accent"
                          >
                            <span aria-hidden className="w-3 text-faint">
                              {open ? "▾" : "▸"}
                            </span>
                            {formatDay(d.day)}
                          </button>
                        )
                      }
                      totals={d}
                    />
                    {open &&
                      d.lines.map((l) => (
                        <Row key={`${l.root ?? ""}|${l.kind}`} label={<span className="pl-[18px] text-muted">{lineLabel(l)}</span>} totals={l} />
                      ))}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}

        <p className="text-xs leading-relaxed text-muted">{SUBSCRIPTION_NOTE}</p>
      </div>
    </dialog>
  );
}

/** One table row; estimates and unpriced runs are labelled under the label. */
function Row({ label, totals, strong }: { label: ReactNode; totals: UsageTotals; strong?: boolean }) {
  const notes = usageNotes(totals);
  const cells = rowCells(totals);
  return (
    <tr className={cx("border-b border-line align-top", strong && "font-semibold")}>
      <td className="py-1.5 pr-3">
        {label}
        {notes.length > 0 && <div className="pl-[18px] text-xs font-normal text-muted">{notes.join(" · ")}</div>}
      </td>
      <td className="py-1.5 text-right">{cells.tokens}</td>
      <td className="py-1.5 text-right">{cells.cost}</td>
      <td className="py-1.5 text-right">{cells.runs}</td>
    </tr>
  );
}
```

- [ ] **Step 3: Wire them into `App.tsx`**

In `src/App.tsx`, add:

```tsx
import { UsageFooter } from "./components/UsageFooter";
import { UsageDialog } from "./components/UsageDialog";
```

Next to `const [showSettings, setShowSettings] = useState(false);`, add:

```tsx
  const [showUsage, setShowUsage] = useState(false);
```

On the `<Sidebar … />` element Phase 2 renders, add the prop:

```tsx
            footer={<UsageFooter onOpen={() => setShowUsage(true)} />}
```

Next to the line that renders `SettingsDialog` (`{showSettings && settings && <SettingsDialog … />}`), add:

```tsx
      {showUsage && <UsageDialog onClose={() => setShowUsage(false)} />}
```

Where `App.tsx` closes the drawers on Escape (the `e.key === "Escape" && !showSettings` check), change the condition to `e.key === "Escape" && !showSettings && !showUsage`, and add `showUsage` to that effect's dependency array, so Esc in the usage panel closes only the panel.

- [ ] **Step 4: Typecheck and test**

Run: `npm run typecheck && npm test`
Expected: no type errors (every import is used; `ReactNode` comes from `react`), and all vitest tests pass.

- [ ] **Step 5: Update AGENTS.md**

Phases 1–3 have already edited AGENTS.md, so find each passage below by its text, not by line number, and add Phase 4's lines next to theirs.

In the "Architecture" table, in the row that starts with `| Core |` (Phase 1 rewrote it), replace `sessions (tabs, windows, saved analyses).` with `sessions (tabs, windows, saved analyses), the usage ledger (`usage`).`, so the row ends `…, the usage ledger (`usage`). **Must not depend on Tauri** |`.

In "Providers", add this bullet after the `**`AnthropicApi` / `OpenAiCompat`**` bullet:

```markdown
- **Usage:** every provider ends a call it has usage for with `Chunk::Usage` (one per model). Claude Code's comes from `result.modelUsage` (`costUSD`; falls back to `usage` + `total_cost_usd` under the requested model, and to `usage::PRICES` when the event reports no cost) and is emitted for failed runs too; the API's from `message_start` plus the last `message_delta`, priced with `usage::PRICES`; the local server's from the final chunk, requested with `stream_options.include_usage`, at $0. Sinks that `match` on `Chunk` need a `Usage` arm; `usage::metered` records it and never forwards it. `Provider::model()` is the model the provider was asked for (an alias such as "haiku" for Claude Code).
```

Then add one more sentence to that bullet, from Task 1's recorded outcome: for outcome A, "The pinned `llama-server` reports usage, so local runs are exact."; for B, "The pinned `llama-server` doesn't report usage, so local runs are estimates."; for C, "The pinned `llama-server` rejects `stream_options`, so it isn't sent and local runs are estimates." For outcome C, also delete ", requested with `stream_options.include_usage`," from the bullet.

In "Live analysis flow" step 4, after the sentence about `agents::run_limited`, add:

```markdown
Every lane, brief explorer and brief merge calls the provider through `usage::metered`, which creates a `usage::Meter` right before `provider.stream` (so a run cancelled during the pre-phase records nothing) and records the call's usage, or an estimate when it was cut short or reported none.
```

In "Persistence", add right after the line that starts with `- Sessions: `<app_data>/sessions/`` (Phase 1):

```markdown
- Usage ledger: `<app_data>/usage.json` (`usage::UsageLedger`): hourly UTC buckets keyed by (hour, root, kind, model), kept in memory, written atomically at most every 5 s when changed (a 1 s shell task that also emits `usage-changed`) and on exit, pruned to 90 days on load. A corrupt file is kept as `.bad`; a file that can't be read is never overwritten (that session's usage isn't saved). Estimates are `ceil(chars / 4)` tokens (system + user sent, text received) at list price, marked `estimatedRuns`: a call dropped mid-stream records one from `Meter::drop`, a call that succeeds without usage records one from `finish`, a call that fails without usage records nothing. Jev calls aren't counted. `Meter::drop` records through the ledger's `std` mutex, so the ledger's locks are never held across `.await`. The UI groups buckets by local day (`src/lib/usage.ts`), so "today" follows the user's clock without a time-zone crate.
```

In "Cross-language contracts", add this line right after the `- **Window events:**` line (Phase 3), which follows Phase 1's `- **Brief and workspace events:**` line; together the three cover every event the shell emits:

```markdown
- **Usage:** `UsageBucket` / `UsageKind` in `usage.rs` ↔ `src/lib/ipc.ts`; the `usage-changed` event (`events::USAGE_CHANGED`, no payload, at most once a second, to every window) ↔ `onUsageChanged` in `src/lib/events.ts`. The panel groups the hourly UTC buckets into local days in `src/lib/usage.ts` (`groupByLocalDay`), which always returns the last 30 days.
```

In "Deliberate decisions to preserve", add:

```markdown
- Prices are pinned in `usage::PRICES` (list $ per million tokens, from the Claude API model table cached 2026-10-06). Updating them is a deliberate one-line change, like `LLAMA_CPP_BUILD`. The CLI aliases map to the newest model of their family, local catalog models cost $0, and any other model is counted with no price ("—", "price unknown for N runs"), never as $0.
```

- [ ] **Step 6: Update the README**

In `README.md`, add this subsection after `### Session instructions` (and after any tabs or windows subsection an earlier phase added), before `### Requirements for the Claude subscription option`:

```markdown
### Usage

The sidebar footer shows today's tokens and estimated cost, for example "Today · 182k tokens · ~$0.41". Click it for the last 30 days: one row per day (a day without runs shows "—"), and each day by project and kind (notes or brief). Every model call counts: each notes agent, each brief explorer and the brief merge. Jev calls don't. A run you cut short by typing again never reports its usage, so LiveBrainstorm estimates it from the characters sent and received and marks the cost with "~". Costs use Anthropic's list prices. On a Claude subscription you aren't billed per token: the panel shows what the same usage would cost on the API. Local models cost nothing. The numbers stay on your computer, in `usage.json` in the app's data folder, for 90 days.
```

- [ ] **Step 7: Manual check in the running app (the user runs `npm run tauri dev`)**

1. Open a project, type a draft and pause. Within about a second of the notes finishing, the footer changes from "Today · 0 tokens · $0.00" to a non-zero line.
2. Type in bursts so a few runs are cut short. The footer's cost gains "~". Click it: the panel opens, and today's row says "incl. ~$… from N cancelled runs".
3. The panel lists 30 days under the "Last 30 days" total, today first. Days without runs show "—" in the Tokens, Est. cost and Runs columns and don't expand.
4. Expand today's row: it lists "shop-api · notes" (your project's name). Build a brief, and a "· brief" line appears without closing the panel.
5. Switch Settings to a custom Claude model id that isn't in the price table: after a run, that day's cost reads "—" if no other run had a price, and the row says "price unknown for 1 run".
6. Switch to a **Local model**: runs are counted and cost $0.00.
7. Quit (⌘Q) and relaunch. The footer shows the same totals, and leaving the app idle for a minute adds no runs (nothing starts a model call at launch).
8. With two windows open, a run in one window updates the footer in the other.
9. In the panel, Esc closes only the panel (an open brief drawer stays), and a click on the backdrop closes it. While it's open, ⌘W and ⌘T do nothing. The note at the bottom reads "On a Claude subscription you aren't billed per token. This is what the same usage would cost on the API."
10. Quit, then corrupt `~/Library/Application Support/app.livebrainstorm.desktop/usage.json` by hand and relaunch: the app opens normally with an empty ledger, and `usage.json.bad` holds the old content.

- [ ] **Step 8: Final checks and commit**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets && cargo check -p livebrainstorm && npm run typecheck && npm test`
Expected: everything passes, with no clippy warnings.

```bash
git add src/components/UsageFooter.tsx src/components/UsageDialog.tsx src/App.tsx AGENTS.md README.md
git commit -m "feat(ui): usage footer and 30-day usage panel; docs

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```
