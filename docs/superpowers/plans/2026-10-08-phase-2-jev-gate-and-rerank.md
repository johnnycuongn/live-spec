# Phase 2: Jev Gate and Rerank — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Before each model call, Jev decides in one short round trip:
- whether the edit actually changed the idea. If not, the notes stay and no Claude call is made.
- which of the keyword-shortlisted code snippets the notes model should see.

Everything falls back to today's behaviour when Jev is off, missing, slow or failing, and never runs for the Local provider.

**Architecture:**
- **Core:** `livebrainstorm-core` gains `jev::gate`, `jev::rerank`, and `analysis::pre_phase`. The pre-phase runs gate and rerank concurrently under one deadline and returns either `Skip` or `Analyze { snippets, notice }`. A gate "unchanged" cancels the rerank.
- **Shell:** the Tauri `analyze` command takes the previous analysed draft, runs the pre-phase, and emits the new `skipped` or `started` events.
- **UI:** the UI keeps the old notes visible until `started`. It sends the last *completed* analysis's draft as `previousDraft`, so skipped pauses never move the comparison point and small edits add up.

**Tech Stack:** Rust 2021 (tokio with `select!`, futures-util, reqwest, serde_json), Tauri v2, React 19, vitest.

**Spec:** `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (phase 2: "Gate and rerank"; sections "Live analysis pipeline", "Jev questions", "IPC changes", "Frontend", "Measurements (phase 1)").

## Global Constraints

- **Gate (Noul):**
  - Question id: `changed`.
  - State: `{"previous_draft", "current_draft"}`, each shaped to ≤ 12,000 chars (head and tail kept).
  - Instructions, verbatim: "Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?"
  - Criteria `true`: "At least one idea, goal, constraint, or question was added, removed, or changed in meaning."
  - Criteria `false`: "Only wording, spelling, punctuation, or formatting changed; the ideas are the same."
  - Skip iff `noul < 0.3` (`GATE_SKIP_BELOW`).
- **Rerank (Noul per snippet):**
  - Question id: `relevant`.
  - Shortlist: `shortlist(index, draft, 20, 1_500)`.
  - State: `{"draft" (shaped ≤ 4,000 chars), "code": {"path", "start_line", "text"}}`.
  - Instructions, verbatim: "Is `code` relevant to what `draft` is discussing? Relevant means it implements, defines, configures, or directly affects something the draft talks about."
  - Criteria `true`: "The code implements, defines, configures, or directly affects a feature, component, or behaviour the draft discusses."
  - Criteria `false`: "The code is unrelated, or only shares generic words with the draft."
  - Keep `max_files` snippets by score, descending; ties keep shortlist order. Clip each to the per-file budget.
  - If the shortlist has ≤ `max_files` candidates, don't rerank.
- **Deadline:** gate and rerank share one deadline, `PRE_BUDGET = 1,500 ms`. Concurrency stays at `JEV_CONCURRENCY = 8`.
  - **Ruling against the spec's "about 1,000 ms":** 20 rerank requests at 8 in flight take 3 waves of about 0.3 s. Task 7 measures and tunes this.
- **Fallbacks:**
  - Any gate error or timeout means "changed", so the pause is analysed.
  - Any rerank error or timeout means today's `retrieve` keyword order is used.
  - The pre-phase reports at most one notice, worded "Jev unavailable, using plain matching: …".
- **Privacy:** Jev sends drafts and code excerpts to TypeSafe. It never runs for the Local provider, whatever the switches say (`Settings::jev_allowed()`).
- **Event order:**
  - When the pre-phase analyses, the backend emits `started` before any pre-phase `notice` and before the model's first `text`.
  - When it skips, it emits only `skipped`.
  - Check events from phase 1 still arrive after `done`.
- **Previous draft:** the draft of the last analysis that reached `done`.
  - `skipped` never updates it.
  - It resets to null on a context change (project or brief).
  - "Try again" sends null, so the gate is bypassed.
- **Process:**
  - Branch `feat/jev-phase-2`.
  - Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Never run `cargo fmt` across the repo; `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests; testable logic lives in core.
  - Node 20.19+ for `npm run build`. On this machine, prefix `PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH"`.

## Review Focus

1. **A slow or unreachable Jev never holds the notes back** much past `PRE_BUDGET`. The pause is analysed with keyword snippets and one quiet notice. Pinned in Task 4 (`a_slow_jev_never_holds_the_notes_past_the_budget`, `jev_errors_fall_back_to_keywords_with_one_notice`).
2. **Small edits add up.** Several minor rewordings, each skipped, must not drift the comparison point. Once they amount to a change, the gate sees the whole difference. Pinned in Task 6 (`moves only when an analysis completes`).
3. **Pausing again mid-pre-phase never blanks the notes.** The old notes stay until `started`. A skipped pause shows them as current, not dashed. Pinned in Task 6 (`keeps the old notes visible until the new run starts`, `a skipped pause keeps the notes and shows them as current`).
4. **"Try again" always re-analyses,** even if the gate would say unchanged. Pinned in Task 6 (`try again bypasses the gate`).
5. **The Local provider never calls Jev** for gate or rerank. Pinned in Task 1 (`local_models_never_send_code_to_jev`).

---

### Task 1: Core helpers — shaped drafts, a longer shortlist, Jev permission per feature

**Files:**
- Modify: `crates/livebrainstorm-core/src/prompt.rs` (`shape_draft_to`)
- Modify: `crates/livebrainstorm-core/src/project/retrieve.rs` (`shortlist`; `retrieve` delegates to it)
- Modify: `crates/livebrainstorm-core/src/project/mod.rs` (re-export `shortlist`)
- Modify: `crates/livebrainstorm-core/src/settings.rs` (`jev_allowed`, `jev_gate_enabled`, `jev_rerank_enabled`)

**Interfaces:**
- Produces:
  - `prompt::shape_draft_to(draft: &str, max_chars: usize) -> String`
  - `project::shortlist(index: &ProjectIndex, query: &str, n: usize, per_snippet_chars: usize) -> Vec<Snippet>`
  - `Settings::jev_allowed(&self) -> bool`, plus `jev_gate_enabled`, `jev_rerank_enabled`, and the existing `jev_check_enabled`. All four are false for `ProviderSettings::Local`.

- [ ] **Step 0: Branch**

```bash
git switch -c feat/jev-phase-2
```

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `prompt.rs`:

```rust
    #[test]
    fn drafts_shape_to_any_size() {
        let draft = format!("{}TAIL", "é".repeat(5_000));
        let shaped = shape_draft_to(&draft, 4_000);
        assert!(shaped.starts_with('é') && shaped.ends_with("TAIL"));
        assert!(shaped.len() < 4_100, "{}", shaped.len());
        assert_eq!(shape_draft_to("short", 4_000), "short");
    }
```

Append to `mod tests` in `retrieve.rs`:

```rust
    #[test]
    fn shortlist_keeps_keyword_order_and_clips_each_snippet() {
        let dir = fixture();
        let idx = scan(dir.path()).unwrap();
        let hits = shortlist(&idx, "I want invoices to include discounts per cart", 20, 40);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].path, "src/billing/invoice.ts");
        assert!(hits.iter().all(|s| s.text.len() <= 40));
        assert_eq!(retrieve(&idx, "I want invoices to include discounts per cart", 2, 80), shortlist(&idx, "I want invoices to include discounts per cart", 2, 40));
    }
```

In `settings.rs`, replace the test `local_models_never_send_code_to_jev` with:

```rust
    #[test]
    fn local_models_never_send_code_to_jev() {
        let mut s = Settings::default();
        assert!(s.jev_allowed() && s.jev_check_enabled() && s.jev_gate_enabled() && s.jev_rerank_enabled());
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert!(!s.jev_allowed(), "the Local option promises nothing leaves this computer");
        assert!(!s.jev_check_enabled() && !s.jev_gate_enabled() && !s.jev_rerank_enabled());
        s.provider = Settings::default().provider;
        s.jev = JevSettings { gate: false, rerank: false, check: false, ..JevSettings::default() };
        assert!(s.jev_allowed());
        assert!(!s.jev_check_enabled() && !s.jev_gate_enabled() && !s.jev_rerank_enabled());
    }
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core --lib`
Expected: compile errors, because `shape_draft_to`, `shortlist`, `jev_allowed`, `jev_gate_enabled` and `jev_rerank_enabled` are not found.

- [ ] **Step 3: Implement**

In `prompt.rs`, delete `const DRAFT_HEAD_CHARS: usize = 2_000;` and replace `fn shape_draft` with:

```rust
/// Very long drafts: keep the opening (the framing) and the tail (where the
/// user is typing now), drop the middle.
fn shape_draft(draft: &str) -> String {
    shape_draft_to(draft, MAX_DRAFT_CHARS)
}

/// `shape_draft` at any size: the first sixth is the opening, the rest the
/// tail. Jev uses it to keep its `state` small.
pub fn shape_draft_to(draft: &str, max_chars: usize) -> String {
    if draft.len() <= max_chars {
        return draft.to_owned();
    }
    let head_end = floor_boundary(draft, max_chars / 6);
    let tail_start = floor_boundary(draft, draft.len() - (max_chars - max_chars / 6));
    format!("{}\n[... middle of draft omitted ...]\n{}", &draft[..head_end], &draft[tail_start..])
}
```

In `retrieve.rs`, replace `pub fn retrieve` with:

```rust
pub fn retrieve(index: &ProjectIndex, query: &str, max_files: usize, budget_chars: usize) -> Vec<Snippet> {
    if max_files == 0 {
        return vec![];
    }
    shortlist(index, query, max_files, budget_chars / max_files)
}

/// Up to `n` snippets in TF-IDF order, each at most `per_snippet_chars`.
pub fn shortlist(index: &ProjectIndex, query: &str, n: usize, per_snippet_chars: usize) -> Vec<Snippet> {
    let weights = query_weights(query);
    if weights.is_empty() || index.files.is_empty() || n == 0 {
        return vec![];
    }
    let mut scored: Vec<(f64, usize)> = (0..index.files.len())
        .filter_map(|i| {
            let score = file_score(index, i, &weights);
            (score > 0.0).then_some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(n)
        .map(|(_, i)| snippet(&index.files[i].rel, &index.files[i].content, &weights, per_snippet_chars))
        .collect()
}
```

In `project/mod.rs`, change `pub use retrieve::{retrieve, Snippet};` to `pub use retrieve::{retrieve, shortlist, Snippet};`.

In `settings.rs`, replace `jev_check_enabled` and its doc comment with:

```rust
    /// Jev sends drafts and code excerpts to TypeSafe, so the Local option
    /// ("private and offline") never uses it, whatever the switches say.
    pub fn jev_allowed(&self) -> bool {
        !matches!(self.provider, ProviderSettings::Local { .. })
    }

    pub fn jev_check_enabled(&self) -> bool {
        self.jev.check && self.jev_allowed()
    }

    pub fn jev_gate_enabled(&self) -> bool {
        self.jev.gate && self.jev_allowed()
    }

    pub fn jev_rerank_enabled(&self) -> bool {
        self.jev.rerank && self.jev_allowed()
    }
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 3 new ones and the unchanged `retrieve` and `prompt` tests; no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core
git commit -m "feat(core): shaped drafts at any size, keyword shortlist, Jev permission per feature" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The gate

**Files:**
- Modify: `crates/livebrainstorm-core/src/jev/test_server.rs` (`serve_with`, `Canned::noul`, `Canned::delayed`)
- Create: `crates/livebrainstorm-core/src/jev/gate.rs`
- Modify: `crates/livebrainstorm-core/src/jev/mod.rs` (`pub mod gate;`)

**Interfaces:**
- Consumes: `Jev::ask`, `Response::noul`, `Question`, `NoulCriteria` (phase 1); `prompt::shape_draft_to` (Task 1).
- Produces:
  - `gate::GATE_SKIP_BELOW: f64 = 0.3`
  - `gate::question() -> Question` and `gate::state(previous: &str, current: &str) -> Value`
  - `gate::unchanged(noul: f64) -> bool`
  - `gate::is_unchanged(jev: &Jev, previous: &str, current: &str, timeout: Duration) -> Result<bool>`, where `Ok(true)` means skip
  - Test-only:
    - `test_server::serve_with(handler: impl Fn(&str) -> Option<Canned> + Send + Sync + 'static) -> (String, Arc<AtomicUsize>)`. The handler receives the request body; each connection is served on its own task.
    - `Canned::noul(id: &str, value: f64) -> Canned`
    - `Canned::delayed(self, ms: u64) -> Canned`
    - `serve(Vec<Canned>)` keeps its behaviour (the next canned response per connection).

- [ ] **Step 1: Extend the test server**

Replace the body of `crates/livebrainstorm-core/src/jev/test_server.rs` with:

```rust
//! A tiny HTTP/1.1 server for client tests. `serve` answers each connection
//! with the next canned response; `serve_with` lets a handler pick the reply
//! from the request body. Connections are served concurrently.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
pub struct Canned {
    status: u16,
    body: String,
    delay: Duration,
}

impl Canned {
    pub fn new(status: u16, body: &str) -> Self {
        Self { status, body: body.to_owned(), delay: Duration::ZERO }
    }

    /// A 200 with one Noul answer under question id `id`.
    pub fn noul(id: &str, value: f64) -> Self {
        Self::new(200, &format!(r#"{{"model":"jev-1.13.0","answers":{{"{id}":{{"type":"noul","noul":{value}}}}}}}"#))
    }

    /// Answer only after `ms` milliseconds, for timeout tests.
    pub fn delayed(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }
}

pub async fn serve(responses: Vec<Canned>) -> (String, Arc<AtomicUsize>) {
    let queue = Mutex::new(VecDeque::from(responses));
    serve_with(move |_: &str| queue.lock().unwrap().pop_front()).await
}

/// `handler` gets each request body; `None` closes the connection unanswered.
pub async fn serve_with(handler: impl Fn(&str) -> Option<Canned> + Send + Sync + 'static) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            let handler = handler.clone();
            tokio::spawn(async move {
                let body = read_request(&mut sock).await;
                let Some(r) = handler(&body) else { return };
                tokio::time::sleep(r.delay).await;
                let reply = format!(
                    "HTTP/1.1 {} Status\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    r.status,
                    r.body.len(),
                    r.body
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base_url, hits)
}

/// Reads the headers and a Content-Length body; returns the body.
async fn read_request(sock: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let Ok(n) = sock.read(&mut chunk).await else { return String::new() };
        if n == 0 {
            return String::new();
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
                return String::from_utf8_lossy(&buf[end + 4..end + 4 + len]).into_owned();
            }
        }
    }
}
```

Run: `cargo test -p livebrainstorm-core jev::`
Expected: every existing `jev::` test still passes (24 at the start of this phase). This is a pure refactor of a test helper.

- [ ] **Step 2: Write the failing gate tests**

Add `pub mod gate;` to `crates/livebrainstorm-core/src/jev/mod.rs`, after `pub mod check;`. Create `crates/livebrainstorm-core/src/jev/gate.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::jev::test_server::{serve, Canned};
    use crate::jev::DEFAULT_MODEL;
    use crate::Error;

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    #[test]
    fn question_is_worded_as_specified() {
        assert_eq!(
            serde_json::to_value(question()).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?",
                "criteria": {
                    "true": "At least one idea, goal, constraint, or question was added, removed, or changed in meaning.",
                    "false": "Only wording, spelling, punctuation, or formatting changed; the ideas are the same."
                }
            })
        );
    }

    #[test]
    fn long_drafts_are_shaped_before_sending() {
        let long = format!("START{}END", "x".repeat(30_000));
        let s = state(&long, "short");
        let previous = s["previous_draft"].as_str().unwrap();
        assert!(previous.starts_with("START") && previous.ends_with("END") && previous.len() < 12_100);
        assert_eq!(s["current_draft"], "short");
    }

    #[test]
    fn skips_only_when_fairly_sure_nothing_changed() {
        assert!(unchanged(0.1) && unchanged(0.29));
        assert!(!unchanged(0.3) && !unchanged(0.98));
    }

    #[tokio::test]
    async fn asks_jev_and_reads_the_answer() {
        let (url, _) = serve(vec![Canned::noul("changed", 0.1), Canned::noul("changed", 0.9)]).await;
        let jev = judge(url);
        assert!(is_unchanged(&jev, "Cache the index.", "Cache the index!", Duration::from_secs(5)).await.unwrap());
        assert!(!is_unchanged(&jev, "Cache the index.", "Cache the index per branch.", Duration::from_secs(5)).await.unwrap());
    }

    #[tokio::test]
    async fn errors_reach_the_caller() {
        let (url, _) = serve(vec![Canned::new(401, "{}")]).await;
        let err = is_unchanged(&judge(url), "a", "b", Duration::from_secs(5)).await.unwrap_err();
        assert!(matches!(err, Error::Precondition(_)), "{err}");
    }
}
```

- [ ] **Step 3: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core jev::gate`
Expected: compile errors, because `question`, `state`, `unchanged`, `is_unchanged` and `Jev` are not in scope.

- [ ] **Step 4: Implement** (insert at the top of `gate.rs`)

```rust
//! The gate: does this edit change the idea, or only the wording? A pause
//! whose draft says the same thing as the last analysed one skips the model
//! call and keeps the notes.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use super::{Jev, NoulCriteria, Question};
use crate::prompt::shape_draft_to;
use crate::{Error, Result};

/// Skip only when Jev is fairly sure nothing changed.
pub const GATE_SKIP_BELOW: f64 = 0.3;
const DRAFT_CHARS: usize = 12_000;

pub fn question() -> Question {
    Question::Noul {
        instructions: json!(
            "Compared with `previous_draft`, does `current_draft` add, remove, or change an idea, goal, constraint, or question?"
        ),
        criteria: Some(NoulCriteria {
            yes: json!("At least one idea, goal, constraint, or question was added, removed, or changed in meaning."),
            no: json!("Only wording, spelling, punctuation, or formatting changed; the ideas are the same."),
        }),
    }
}

pub fn state(previous: &str, current: &str) -> Value {
    json!({
        "previous_draft": shape_draft_to(previous, DRAFT_CHARS),
        "current_draft": shape_draft_to(current, DRAFT_CHARS),
    })
}

/// True when Jev's yes-probability ("it changed") is low enough to skip.
pub fn unchanged(noul: f64) -> bool {
    noul < GATE_SKIP_BELOW
}

/// Asks Jev whether `current` changes the idea of `previous`. `Ok(true)`
/// means skip; on an error the caller analyses anyway.
pub async fn is_unchanged(jev: &Jev, previous: &str, current: &str, timeout: Duration) -> Result<bool> {
    let questions = BTreeMap::from([("changed".to_owned(), question())]);
    let response = jev.ask(&state(previous, current), &questions, timeout).await?;
    let noul = response.noul("changed").ok_or_else(|| Error::Provider("Jev returned an unexpected answer.".into()))?;
    Ok(unchanged(noul))
}
```

- [ ] **Step 5: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core jev:: && cargo clippy --workspace --all-targets`
Expected: all `jev::` tests pass (the 24 existing and 5 new); no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/jev
git commit -m "feat(jev): gate that skips re-analysis when the idea is unchanged" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The rerank

**Files:**
- Create: `crates/livebrainstorm-core/src/jev/rerank.rs`
- Modify: `crates/livebrainstorm-core/src/jev/mod.rs` (`pub mod rerank;`)

**Interfaces:**
- Consumes: `Jev::ask`, `Response::noul`, `Question`, `NoulCriteria`, `check::JEV_CONCURRENCY` (phase 1); `shape_draft_to`, `Snippet` (Task 1); `test_server::{serve_with, Canned}` (Task 2).
- Produces:
  - `rerank::SHORTLIST: usize = 20` and `rerank::SHORTLIST_CHARS: usize = 1_500`
  - `rerank::question() -> Question` and `rerank::state(shaped_draft: &str, code: &Snippet) -> Value`
  - `rerank::pick(candidates: Vec<Snippet>, scores: &[f64], keep: usize, per_file_chars: usize) -> Vec<Snippet>`
  - `rerank::scores(jev: &Jev, draft: &str, candidates: &[Snippet], deadline: Instant) -> Result<Vec<f64>>`, which returns scores in candidate order and fails as a whole if any request fails

- [ ] **Step 1: Write the failing tests**

Add `pub mod rerank;` to `jev/mod.rs`, after `pub mod gate;`. Create `crates/livebrainstorm-core/src/jev/rerank.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::jev::test_server::{serve_with, Canned};
    use crate::jev::DEFAULT_MODEL;

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    fn snip(path: &str, text: &str) -> Snippet {
        Snippet { path: path.into(), start_line: 1, text: text.into() }
    }

    #[test]
    fn question_is_worded_as_specified() {
        assert_eq!(
            serde_json::to_value(question()).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Is `code` relevant to what `draft` is discussing? Relevant means it implements, defines, configures, or directly affects something the draft talks about.",
                "criteria": {
                    "true": "The code implements, defines, configures, or directly affects a feature, component, or behaviour the draft discusses.",
                    "false": "The code is unrelated, or only shares generic words with the draft."
                }
            })
        );
    }

    #[test]
    fn picks_the_best_scores_with_ties_in_shortlist_order() {
        let candidates = vec![snip("a.ts", "aaaa"), snip("b.ts", "bbbbbbbb"), snip("c.ts", "cccc"), snip("d.ts", "dddd")];
        let picked = pick(candidates, &[0.2, 0.9, 0.5, 0.5], 3, 4);
        assert_eq!(picked.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["b.ts", "c.ts", "d.ts"]);
        assert_eq!(picked[0].text, "bbbb", "clipped to the per-file budget");
    }

    #[tokio::test]
    async fn scores_come_back_in_candidate_order() {
        let (url, hits) = serve_with(|body: &str| Some(Canned::noul("relevant", if body.contains("createInvoice") { 0.9 } else { 0.1 }))).await;
        let candidates = vec![snip("src/cart/cart.ts", "export class CartStore {}"), snip("src/billing/invoice.ts", "export function createInvoice() {}")];
        let got = scores(&judge(url), "invoices need discounts", &candidates, Instant::now() + Duration::from_secs(5)).await.unwrap();
        assert_eq!(got, [0.1, 0.9]);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn one_failed_request_fails_the_ranking() {
        let (url, _) = serve_with(|body: &str| Some(if body.contains("CartStore") { Canned::new(500, "boom") } else { Canned::noul("relevant", 0.9) })).await;
        let candidates = vec![snip("src/cart/cart.ts", "export class CartStore {}"), snip("src/billing/invoice.ts", "createInvoice")];
        assert!(scores(&judge(url), "invoices", &candidates, Instant::now() + Duration::from_secs(5)).await.is_err());
    }
}
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core jev::rerank`
Expected: compile errors, because `question`, `pick`, `scores`, `Snippet`, `Jev` and `Instant` are not in scope.

- [ ] **Step 3: Implement** (insert at the top of `rerank.rs`)

```rust
//! Rerank: of the keyword shortlist, which snippets are actually about what
//! the draft discusses? One Noul per snippet, in parallel; the best few go to
//! the notes model.

use std::collections::BTreeMap;
use std::time::Instant;

use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};

use super::check::JEV_CONCURRENCY;
use super::{Jev, NoulCriteria, Question};
use crate::project::Snippet;
use crate::prompt::shape_draft_to;
use crate::{Error, Result};

pub const SHORTLIST: usize = 20;
pub const SHORTLIST_CHARS: usize = 1_500;
const DRAFT_CHARS: usize = 4_000;

pub fn question() -> Question {
    Question::Noul {
        instructions: json!(
            "Is `code` relevant to what `draft` is discussing? Relevant means it implements, defines, configures, or directly affects something the draft talks about."
        ),
        criteria: Some(NoulCriteria {
            yes: json!("The code implements, defines, configures, or directly affects a feature, component, or behaviour the draft discusses."),
            no: json!("The code is unrelated, or only shares generic words with the draft."),
        }),
    }
}

/// `shaped_draft` is already cut down with `shape_draft_to`.
pub fn state(shaped_draft: &str, code: &Snippet) -> Value {
    json!({ "draft": shaped_draft, "code": { "path": code.path, "start_line": code.start_line, "text": code.text } })
}

/// The `keep` highest-scoring snippets (ties keep shortlist order), each
/// clipped to `per_file_chars`.
pub fn pick(candidates: Vec<Snippet>, scores: &[f64], keep: usize, per_file_chars: usize) -> Vec<Snippet> {
    let mut ranked: Vec<(f64, Snippet)> = scores.iter().copied().zip(candidates).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked
        .into_iter()
        .take(keep)
        .map(|(_, mut s)| {
            if s.text.len() > per_file_chars {
                let cut = (0..=per_file_chars).rev().find(|i| s.text.is_char_boundary(*i)).unwrap_or(0);
                s.text.truncate(cut);
            }
            s
        })
        .collect()
}

/// One relevance score per candidate, in candidate order. Fails as a whole
/// if any request fails or the deadline passes: half a ranking is worse than
/// the keyword order.
pub async fn scores(jev: &Jev, draft: &str, candidates: &[Snippet], deadline: Instant) -> Result<Vec<f64>> {
    let shaped = shape_draft_to(draft, DRAFT_CHARS);
    let questions = BTreeMap::from([("relevant".to_owned(), question())]);
    let (shaped, questions) = (&shaped, &questions);
    let mut answers = std::pin::pin!(stream::iter(candidates.iter().enumerate())
        .map(|(i, code)| async move {
            let timeout = deadline.saturating_duration_since(Instant::now());
            (i, jev.ask(&state(shaped, code), questions, timeout).await)
        })
        .buffer_unordered(JEV_CONCURRENCY));
    let mut out = vec![0.0; candidates.len()];
    while let Some((i, answer)) = answers.next().await {
        out[i] = answer?.noul("relevant").ok_or_else(|| Error::Provider("Jev returned an unexpected answer.".into()))?;
    }
    Ok(out)
}
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core jev:: && cargo clippy --workspace --all-targets`
Expected: all `jev::` tests pass, including the 4 new rerank tests; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/jev
git commit -m "feat(jev): rerank the keyword shortlist by relevance to the draft" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The pre-phase

**Files:**
- Create: `crates/livebrainstorm-core/src/analysis.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod analysis;`)

**Interfaces:**
- Consumes: `gate::is_unchanged` (Task 2); `rerank::{scores, pick, SHORTLIST, SHORTLIST_CHARS}` (Task 3); `project::{retrieve, shortlist}` (Task 1); `test_server::{serve_with, Canned}` (Task 2).
- Produces:
  - `analysis::PRE_BUDGET: Duration` (1,500 ms)
  - `analysis::PreInput<'a> { draft: &'a str, previous_draft: Option<&'a str>, index: Option<&'a ProjectIndex>, max_files: usize, budget_chars: usize, jev: Option<&'a Jev>, gate: bool, rerank: bool }`
  - `analysis::PreOutcome { Skip, Analyze { snippets: Vec<Snippet>, notice: Option<String> } }` (`Debug`, `PartialEq`)
  - `analysis::pre_phase(input: PreInput<'_>, budget: Duration) -> PreOutcome`

- [ ] **Step 1: Write the failing tests**

Add `pub mod analysis;` to `lib.rs`, before `pub mod env_file;`. Create `crates/livebrainstorm-core/src/analysis.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::jev::test_server::{serve_with, Canned};
    use crate::jev::DEFAULT_MODEL;
    use crate::project::scan;
    use crate::project::scan::tests::fixture;

    const DRAFT: &str = "I want invoices to include discounts per cart";

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    fn input<'a>(index: &'a ProjectIndex, jev: Option<&'a Jev>, previous: Option<&'a str>, max_files: usize) -> PreInput<'a> {
        PreInput { draft: DRAFT, previous_draft: previous, index: Some(index), max_files, budget_chars: 4_000, jev, gate: true, rerank: true }
    }

    fn keyword(index: &ProjectIndex, max_files: usize) -> Vec<Snippet> {
        retrieve(index, DRAFT, max_files, 4_000)
    }

    /// Gate requests carry `previous_draft`; every other request is a rerank.
    fn answers(gate: f64, relevant: fn(&str) -> f64) -> impl Fn(&str) -> Option<Canned> + Send + Sync + 'static {
        move |body: &str| {
            Some(if body.contains("previous_draft") { Canned::noul("changed", gate) } else { Canned::noul("relevant", relevant(body)) })
        }
    }

    #[tokio::test]
    async fn without_jev_it_is_todays_keyword_retrieval() {
        let idx = scan(fixture().path()).unwrap();
        let out = pre_phase(input(&idx, None, Some("old"), 1), PRE_BUDGET).await;
        assert_eq!(out, PreOutcome::Analyze { snippets: keyword(&idx, 1), notice: None });
    }

    #[tokio::test]
    async fn an_unchanged_idea_skips_the_analysis() {
        let (url, _) = serve_with(answers(0.1, |_| 0.5)).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        assert_eq!(pre_phase(input(&idx, Some(&jev), Some("old wording"), 1), PRE_BUDGET).await, PreOutcome::Skip);
    }

    #[tokio::test]
    async fn a_changed_idea_gets_the_reranked_code() {
        let (url, _) = serve_with(answers(0.9, |body| if body.contains("CartStore") { 0.95 } else { 0.05 })).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let PreOutcome::Analyze { snippets, notice } = pre_phase(input(&idx, Some(&jev), Some("old"), 1), PRE_BUDGET).await else {
            panic!("expected Analyze");
        };
        assert_eq!(notice, None);
        assert_eq!(snippets.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["src/cart/cart.ts"]);
        assert_eq!(keyword(&idx, 1)[0].path, "src/billing/invoice.ts", "keyword order alone would pick invoice.ts");
    }

    #[tokio::test]
    async fn jev_errors_fall_back_to_keywords_with_one_notice() {
        let (url, _) = serve_with(|_: &str| Some(Canned::new(500, "boom"))).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let PreOutcome::Analyze { snippets, notice } = pre_phase(input(&idx, Some(&jev), Some("old"), 1), PRE_BUDGET).await else {
            panic!("an error must never skip");
        };
        assert_eq!(snippets, keyword(&idx, 1));
        assert!(notice.as_deref().is_some_and(|n| n.starts_with("Jev unavailable, using plain matching")), "{notice:?}");
    }

    #[tokio::test]
    async fn a_slow_jev_never_holds_the_notes_past_the_budget() {
        // Says "unchanged", but too late: the pause must be analysed anyway.
        let (url, _) = serve_with(|body: &str| {
            let reply = if body.contains("previous_draft") { Canned::noul("changed", 0.1) } else { Canned::noul("relevant", 0.9) };
            Some(reply.delayed(2_000))
        })
        .await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let started = Instant::now();
        let out = pre_phase(input(&idx, Some(&jev), Some("old"), 1), Duration::from_millis(300)).await;
        assert!(started.elapsed() < Duration::from_millis(1_000), "{:?}", started.elapsed());
        assert!(matches!(out, PreOutcome::Analyze { ref snippets, notice: Some(_) } if *snippets == keyword(&idx, 1)), "{out:?}");
    }

    #[tokio::test]
    async fn no_previous_draft_or_few_candidates_cost_no_jev_calls() {
        let (url, hits) = serve_with(answers(0.1, |_| 0.9)).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        // No previous draft: no gate. 3 candidates for 5 slots: no rerank.
        let out = pre_phase(input(&idx, Some(&jev), None, 5), PRE_BUDGET).await;
        assert_eq!(out, PreOutcome::Analyze { snippets: keyword(&idx, 5), notice: None });
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn switched_off_features_never_call_jev() {
        let (url, hits) = serve_with(answers(0.1, |_| 0.9)).await;
        let idx = scan(fixture().path()).unwrap();
        let jev = judge(url);
        let off = PreInput { gate: false, rerank: false, ..input(&idx, Some(&jev), Some("old"), 1) };
        assert_eq!(pre_phase(off, PRE_BUDGET).await, PreOutcome::Analyze { snippets: keyword(&idx, 1), notice: None });
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
}
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core analysis`
Expected: compile errors, because `PreInput`, `PreOutcome`, `pre_phase`, `PRE_BUDGET`, `retrieve`, `Jev`, `ProjectIndex`, `Snippet`, `Duration` and `Instant` are not in scope.

- [ ] **Step 3: Implement** (insert at the top of `analysis.rs`)

```rust
//! The per-pause pipeline before the model call ("pre-phase"): the Jev gate
//! and the Jev rerank run at the same time under one deadline. Without Jev,
//! or when it's slow or failing, this is exactly today's keyword retrieval.

use std::time::{Duration, Instant};

use crate::jev::{gate, rerank, Jev};
use crate::project::{retrieve, shortlist, ProjectIndex, Snippet};
use crate::Error;

/// Gate and rerank share this deadline. Jev answers in about 0.3 s, and the
/// rerank's 20 requests run 8 at a time (3 waves).
pub const PRE_BUDGET: Duration = Duration::from_millis(1_500);

pub struct PreInput<'a> {
    pub draft: &'a str,
    /// The last draft that was actually analysed; None means no gate.
    pub previous_draft: Option<&'a str>,
    pub index: Option<&'a ProjectIndex>,
    /// How many snippets the notes model gets, and their total size.
    pub max_files: usize,
    pub budget_chars: usize,
    /// None without a key, or for the Local provider.
    pub jev: Option<&'a Jev>,
    pub gate: bool,
    pub rerank: bool,
}

#[derive(Debug, PartialEq)]
pub enum PreOutcome {
    /// The gate found no real change: keep the notes, make no model call.
    Skip,
    Analyze { snippets: Vec<Snippet>, notice: Option<String> },
}

pub async fn pre_phase(input: PreInput<'_>, budget: Duration) -> PreOutcome {
    let deadline = Instant::now() + budget;
    let gate_jev = input.jev.filter(|_| input.gate);
    let rerank_jev = input.jev.filter(|_| input.rerank);

    let gate_check = async {
        let (jev, previous) = (gate_jev?, input.previous_draft?);
        Some(gate::is_unchanged(jev, previous, input.draft, deadline.saturating_duration_since(Instant::now())).await)
    };
    let picking = async {
        let Some(index) = input.index else { return (vec![], None) };
        let keyword = || retrieve(index, input.draft, input.max_files, input.budget_chars);
        let Some(jev) = rerank_jev else { return (keyword(), None) };
        let candidates = shortlist(index, input.draft, rerank::SHORTLIST, rerank::SHORTLIST_CHARS);
        if candidates.len() <= input.max_files {
            return (keyword(), None);
        }
        match rerank::scores(jev, input.draft, &candidates, deadline).await {
            Ok(scores) => {
                let per_file = input.budget_chars / input.max_files.max(1);
                (rerank::pick(candidates, &scores, input.max_files, per_file), None)
            }
            Err(e) => (keyword(), Some(unavailable(&e))),
        }
    };

    tokio::pin!(gate_check, picking);
    let (mut gate_answer, mut picked) = (None, None);
    while gate_answer.is_none() || picked.is_none() {
        tokio::select! {
            answer = &mut gate_check, if gate_answer.is_none() => {
                if matches!(answer, Some(Ok(true))) {
                    return PreOutcome::Skip; // dropping `picking` cancels the rerank
                }
                gate_answer = Some(answer);
            }
            result = &mut picking, if picked.is_none() => picked = Some(result),
        }
    }
    let (snippets, rerank_notice) = picked.expect("the loop ends once both are done");
    let gate_notice = match gate_answer {
        Some(Some(Err(e))) => Some(unavailable(&e)),
        _ => None,
    };
    PreOutcome::Analyze { snippets, notice: gate_notice.or(rerank_notice) }
}

fn unavailable(e: &Error) -> String {
    format!("Jev unavailable, using plain matching: {e}")
}
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 7 new `analysis::` tests; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core
git commit -m "feat(core): pre-phase running the Jev gate and rerank under one deadline" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Shell — run the pre-phase in `analyze`

**Files:**
- Modify: `src-tauri/src/commands.rs` (`UiEvent::Started`, `UiEvent::Skipped`, the `analyze` signature and body)
- Modify: `AGENTS.md`, `README.md`, `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md`

**Interfaces:**
- Consumes: `analysis::{pre_phase, PreInput, PreOutcome, PRE_BUDGET}` (Task 4); `Settings::{jev_allowed, jev_gate_enabled, jev_rerank_enabled, jev_check_enabled}` (Task 1); `provider::jev` (phase 1).
- Produces the IPC contract that Task 6 mirrors:
  - The command signature becomes `analyze(draft: String, previous_draft: Option<String>, on_event)`. The JS side sends `{ draft, previousDraft }`.
  - New events: `{type:"started"}`, emitted before any pre-phase `notice` and before the first `text`; and `{type:"skipped"}`, which is final, so no `done` follows.

- [ ] **Step 1: Add the events**

In `src-tauri/src/commands.rs`, add these to `UiEvent` after `Notice { message: String },`:

```rust
    /// The pre-phase passed: the model call is starting (clear old notes now).
    Started,
    /// The gate found no real change: keep the current notes. Final.
    Skipped,
```

Add to the imports:

```rust
use livebrainstorm_core::analysis::{self, PreInput, PreOutcome, PRE_BUDGET};
```

- [ ] **Step 2: Replace `analyze`**

```rust
#[tauri::command]
pub async fn analyze(app: AppHandle, draft: String, previous_draft: Option<String>, on_event: Channel<UiEvent>) -> Result<()> {
    let task_app = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let settings = state.settings();
        let index = state.project();
        let result = async {
            let provider = provider::resolve(&task_app, Tier::Fast).await?;
            let brief = state.brief.read().expect("brief lock").as_ref().map(|b| b.text.clone());
            let budget = provider.context_budget_chars();
            let max_files = if budget < 40_000 { 3 } else { 5 };

            // Jev sends text to TypeSafe, so never for the Local provider.
            let jev = if settings.jev_allowed() { provider::jev(&task_app) } else { None };
            let pre = analysis::pre_phase(
                PreInput {
                    draft: &draft,
                    previous_draft: previous_draft.as_deref(),
                    index: index.as_deref(),
                    max_files,
                    budget_chars: budget / 4,
                    jev: jev.as_ref(),
                    gate: settings.jev_gate_enabled(),
                    rerank: settings.jev_rerank_enabled(),
                },
                PRE_BUDGET,
            )
            .await;
            let PreOutcome::Analyze { snippets, notice } = pre else { return Ok(None) };
            send(&on_event, UiEvent::Started);
            if let Some(message) = notice {
                send(&on_event, UiEvent::Notice { message });
            }

            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider);
            provider.stream(&req, &mut on_chunk(&on_event)).await.map(Some)
        }
        .await;
        let text = match result {
            Ok(Some(text)) => text,
            Ok(None) => return send(&on_event, UiEvent::Skipped),
            Err(e) => return send(&on_event, UiEvent::Error { message: e.to_string() }),
        };
        send(&on_event, UiEvent::Done { text: text.clone() });

        // "Checked in code" fills in after the notes. The next analysis
        // aborts this task, checks included.
        if !settings.jev_check_enabled() {
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

- [ ] **Step 3: Build and lint**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: no warnings; all tests pass.

- [ ] **Step 4: Update the docs**

1. In `AGENTS.md`, under "### Live analysis flow", replace step 3 with:

```markdown
3. The task resolves a provider at `Tier::Fast` (`src-tauri/src/provider.rs`), then runs `analysis::pre_phase`. The Jev gate ("did the idea change since `previousDraft`?") and the Jev rerank (scores the 20-snippet keyword shortlist and keeps the best 3 or 5) run concurrently under `PRE_BUDGET` (1.5 s). A confident "unchanged" emits `skipped` and stops. Otherwise it emits `started`, then at most one pre-phase `notice`, builds the prompt with `prompt::analysis_request` and streams. Without Jev, or for the Local provider, or when Jev is slow or failing, this is plain keyword retrieval (`project::retrieve`).
```

2. In `AGENTS.md`, under "### Jev (TypeSafe)", add:

```markdown
- **Gate and rerank** (`jev::gate`, `jev::rerank`, `analysis::pre_phase`): the UI sends the draft of the last *completed* analysis as `previousDraft` (skips never move it; "Try again" sends null). Gate errors mean "analyse"; rerank errors mean keyword order. Neither ever adds more than `PRE_BUDGET` before the model call.
- `Settings::jev_allowed()` is false for the Local provider; `jev_gate_enabled`, `jev_rerank_enabled` and `jev_check_enabled` all respect it.
```

3. In `README.md`, under "### Optional: fact-check notes with Jev", replace the sentence beginning "Checking sends each Fault…" with:

```markdown
With a key, Jev also skips re-analysis when an edit doesn't change the idea, and picks which code snippets the notes model sees. These calls send your draft, each Fault, and short code excerpts to TypeSafe. They never run with the Local model option, which promises nothing leaves your computer.
```

4. In the spec, under "### Jev client", change "Timeouts: each request is bounded by the remaining pre-phase budget, or 5 s during the check phase." to:

```markdown
- Timeouts: each request is bounded by the remaining pre-phase budget (`PRE_BUDGET`, 1,500 ms in phase 2: 20 rerank requests at 8 in flight take 3 waves of about 0.3 s; tuned from the phase 2 measurement), or 5 s during the check phase.
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri AGENTS.md README.md docs/superpowers/specs
git commit -m "feat: run the Jev pre-phase before each analysis; started and skipped events" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Frontend — keep notes until `started`, send `previousDraft`

**Files:**
- Modify: `src/lib/ipc.ts` (`analyze` signature; the `started` and `skipped` events)
- Modify: `src/lib/analysisState.ts` (`startAnalysis(prev, draft)`, the `started`/`skipped` handling, `lastAnalyzedAfter`, `previousDraftFor`)
- Modify: `src/lib/analysisState.test.ts`
- Modify: `src/lib/useLiveAnalysis.ts`

**Interfaces:**
- Consumes: the IPC contract from Task 5.
- Produces:
  - `startAnalysis(prev: AnalysisState, draft: string): AnalysisState`
  - `lastAnalyzedAfter(last: string | null, ev: UiEvent, sent: string): string | null`
  - `previousDraftFor(last: string | null, rerun: boolean): string | null`
  - `api.analyze(draft: string, previousDraft: string | null, onEvent)`

- [ ] **Step 1: Mirror the IPC contract** (`src/lib/ipc.ts`)

Add two members to the `UiEvent` union:

```ts
  | { type: "started" }
  | { type: "skipped" }
```

Replace the `analyze` entry in `api` with:

```ts
  analyze: (draft: string, previousDraft: string | null, onEvent: (e: UiEvent) => void) =>
    stream("analyze", { draft, previousDraft }, onEvent),
```

- [ ] **Step 2: Write the failing tests** (`src/lib/analysisState.test.ts`, full replacement)

```ts
import { describe, expect, it } from "vitest";
import { applyEvent, initialAnalysis, lastAnalyzedAfter, previousDraftFor, startAnalysis } from "./analysisState";
import type { UiEvent } from "./ipc";

const run = (events: UiEvent[]) => events.reduce(applyEvent, startAnalysis(initialAnalysis, "draft"));
const check: UiEvent = { type: "check", claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("applyEvent", () => {
  it("streams text, then completes", () => {
    const s = run([{ type: "started" }, { type: "text", text: "## Intent\n" }, { type: "text", text: "- a" }, { type: "done", text: "" }]);
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

  it("keeps the old notes visible until the new run starts", () => {
    const before = run([{ type: "done", text: "## Intent\n- old" }, { type: "checking", count: 1, total: 1 }, check, { type: "notice", message: "x" }]);
    const pending = startAnalysis(before, "new draft");
    expect(pending.status).toBe("thinking");
    expect(pending.markdown).toBe("## Intent\n- old");
    const started = applyEvent(pending, { type: "started" });
    expect(started.markdown).toBe("");
    expect(started.checks).toEqual([]);
    expect(started.checking).toBeNull();
    expect(started.notice).toBeNull();
  });

  it("a skipped pause keeps the notes and shows them as current", () => {
    const before = run([{ type: "done", text: "## Intent\n- a" }]);
    const s = applyEvent(startAnalysis(before, "draft, reworded"), { type: "skipped" });
    expect(s.status).toBe("done");
    expect(s.markdown).toBe("## Intent\n- a");
    expect(s.analyzedDraft).toBe("draft, reworded");
  });
});

describe("previous draft for the gate", () => {
  it("moves only when an analysis completes", () => {
    let last: string | null = null;
    last = lastAnalyzedAfter(last, { type: "started" }, "v1");
    expect(last).toBeNull();
    last = lastAnalyzedAfter(last, { type: "done", text: "" }, "v1");
    expect(last).toBe("v1");
    last = lastAnalyzedAfter(last, { type: "skipped" }, "v1, reworded");
    last = lastAnalyzedAfter(last, { type: "skipped" }, "v1, reworded again");
    expect(last).toBe("v1");
  });

  it("try again bypasses the gate", () => {
    expect(previousDraftFor("v1", true)).toBeNull();
    expect(previousDraftFor("v1", false)).toBe("v1");
    expect(previousDraftFor(null, false)).toBeNull();
  });
});
```

- [ ] **Step 3: Run and confirm the failure**

Run: `npm test`
Expected: FAIL. `lastAnalyzedAfter` and `previousDraftFor` are not exported, and `startAnalysis` takes one argument.

- [ ] **Step 4: Implement** (`src/lib/analysisState.ts`)

Replace `startAnalysis` with:

```ts
/** A new pause: the old notes stay visible until the backend says `started`. */
export const startAnalysis = (prev: AnalysisState, draft: string): AnalysisState => ({
  ...prev,
  status: "thinking",
  error: null,
  analyzedDraft: draft,
});
```

Add these cases to the `switch` in `applyEvent`, before `default`:

```ts
    case "started":
      return { ...s, markdown: "", checks: [], checking: null, notice: null, error: null };
    case "skipped":
      return { ...s, status: "done" };
```

Append to the file:

```ts
/**
 * The draft the gate compares against: the last one actually analysed.
 * Skips never move it, so small edits add up to a real change.
 */
export const lastAnalyzedAfter = (last: string | null, ev: UiEvent, sent: string): string | null =>
  ev.type === "done" ? sent : last;

/** "Try again" always re-analyses, so it sends no previous draft. */
export const previousDraftFor = (last: string | null, rerun: boolean): string | null => (rerun ? null : last);
```

Replace `src/lib/useLiveAnalysis.ts` with:

```ts
import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./ipc";
import {
  applyEvent,
  initialAnalysis,
  lastAnalyzedAfter,
  previousDraftFor,
  startAnalysis,
  type AnalysisState,
} from "./analysisState";

const MIN_CHARS = 24;
const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/**
 * Re-analyzes the draft after the user pauses typing.
 *
 * Like a colleague reading over your shoulder: they wait until you stop
 * typing, then comment; if you keep going, their last comment stays visible
 * but marked as out of date. If the edit didn't change the idea, the backend
 * skips (Jev gate) and the notes stay. Old responses are dropped by request
 * id; the backend also aborts the superseded run, checks included.
 */
export function useLiveAnalysis(draft: string, debounceMs: number, contextKey: string) {
  const [state, setState] = useState<AnalysisState>(initialAnalysis);
  const requestId = useRef(0);
  const lastSent = useRef("");
  const lastAnalyzed = useRef<string | null>(null);

  const run = useCallback((text: string, rerun = false) => {
    const id = ++requestId.current;
    lastSent.current = normalize(text);
    setState((s) => startAnalysis(s, text));
    const mine = () => id === requestId.current;

    api
      .analyze(text, previousDraftFor(lastAnalyzed.current, rerun), (ev) => {
        if (!mine()) return;
        lastAnalyzed.current = lastAnalyzedAfter(lastAnalyzed.current, ev, text);
        setState((s) => applyEvent(s, ev));
      })
      .catch((e) => mine() && setState((s) => ({ ...s, status: "error", error: errorMessage(e) })));
  }, []);

  // New project or brief: the same draft deserves a fresh read, with no gate.
  useEffect(() => {
    lastSent.current = "";
    lastAnalyzed.current = null;
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
    rerun: () => run(draft, true),
  };
}
```

- [ ] **Step 5: Verify**

Run: `npm test && npm run typecheck && PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH" npm run build`
Expected: 15 tests pass (sections 3, analysisState 7, checks 5); no type errors; the build prints `✓ built in`.

- [ ] **Step 6: Commit**

```bash
git add src
git commit -m "feat(ui): keep notes until started, send the last analysed draft for the gate" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Measure the pre-phase on this repo and verify end to end

**Files:**
- Modify: `crates/livebrainstorm-core/tests/jev_live.rs` (add `pre_phase_on_this_repo`; use `gate::question()` in the existing test)
- Modify: `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (a "Measurements (phase 2)" section)
- Possibly modify: `crates/livebrainstorm-core/src/analysis.rs` (`PRE_BUDGET`, only per Step 3's rule)

**Interfaces:**
- Consumes: `analysis::{pre_phase, PreInput, PreOutcome, PRE_BUDGET}`, `project::{scan, retrieve}`, `jev::gate`.

- [ ] **Step 1: Add the live test**

In `crates/livebrainstorm-core/tests/jev_live.rs`, add to the imports:

```rust
use livebrainstorm_core::analysis::{pre_phase, PreInput, PreOutcome, PRE_BUDGET};
use livebrainstorm_core::jev::gate;
use livebrainstorm_core::project::{retrieve, scan};
```

In `noul_and_choice_round_trip`, replace the whole `Question::Noul { … }` value in the `gate` map with `gate::question()`, and remove `NoulCriteria` and `Question` from the imports if they become unused. Then append:

```rust
#[tokio::test]
#[ignore = "calls the TypeSafe API"]
async fn pre_phase_on_this_repo() {
    let Some(key) = key() else {
        eprintln!("skipped: no TYPESAFE_API_KEY in the environment or .env.local");
        return;
    };
    let jev = Jev::new(key, DEFAULT_MODEL.into(), reqwest::Client::new());
    let index = scan(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))).unwrap();
    let before = "Every pause spawns a fresh claude -p process from ClaudeCli::stream.";
    let changed = "Every pause spawns a fresh claude -p process from ClaudeCli::stream. I want to keep one process alive, reuse it across analyze calls, and kill it when replace_task aborts the old analysis.";
    let reworded = "Each pause starts a new claude -p process from ClaudeCli::stream.";

    for (label, current) in [("changed", changed), ("reworded", reworded)] {
        for round in 1..=3 {
            let t = Instant::now();
            let input = PreInput {
                draft: current,
                previous_draft: Some(before),
                index: Some(&index),
                max_files: 5,
                budget_chars: 30_000,
                jev: Some(&jev),
                gate: true,
                rerank: true,
            };
            match pre_phase(input, PRE_BUDGET).await {
                PreOutcome::Skip => eprintln!("{label:<9} round {round}: {:>5} ms  SKIP", t.elapsed().as_millis()),
                PreOutcome::Analyze { snippets, notice } => eprintln!(
                    "{label:<9} round {round}: {:>5} ms  notice={notice:?}  picked={:?}",
                    t.elapsed().as_millis(),
                    snippets.iter().map(|s| s.path.as_str()).collect::<Vec<_>>()
                ),
            }
        }
    }
    let keyword: Vec<String> = retrieve(&index, changed, 5, 30_000).into_iter().map(|s| s.path).collect();
    eprintln!("keyword order (no Jev): {keyword:?}");
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p livebrainstorm-core --test jev_live -- --ignored --nocapture 2>&1 | grep -E "round|keyword order|skipped|test result"`
Expected: both tests pass, with six `changed`/`reworded` lines and a keyword-order line.
- The `reworded` rounds should usually `SKIP` and the `changed` rounds should usually not; record what actually happens.
- If you see `skipped: no TYPESAFE_API_KEY`, stop and ask the user. Never print the key.

- [ ] **Step 3: Tune `PRE_BUDGET` only by this rule**

If 2 or more of the 3 `changed` rounds show `notice=Some(… timed out …)`:
1. Set `PRE_BUDGET` in `analysis.rs` to the slowest of those rounds' elapsed times, rounded up to the next 250 ms.
2. Re-run Step 2 once.

Otherwise leave it at 1,500 ms. Either way, record what you did in Step 4.

- [ ] **Step 4: Record the measurements** (append to the spec, using the real numbers)

```markdown
## Measurements (phase 2, 2026-10-08)

`cargo test -p livebrainstorm-core --test jev_live pre_phase_on_this_repo -- --ignored --nocapture` on this repo, `PRE_BUDGET` = <value> ms:

| Draft vs previous | Rounds (elapsed, outcome) |
|---|---|
| Adds an idea | <ms, outcome>; <ms, outcome>; <ms, outcome> |
| Rewords only | <ms, outcome>; <ms, outcome>; <ms, outcome> |

- Reranked picks vs keyword order: <the two lists, and whether the reranked picks look more on-topic>.
- `PRE_BUDGET`: <kept at 1,500 ms | raised to N ms, because …>.
```

Replace every `<…>` with the measured values before committing.

- [ ] **Step 5: Final verification and commit**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm test && npm run typecheck`
Expected: no warnings; all tests pass.

```bash
git add crates/livebrainstorm-core docs/superpowers/specs
git commit -m "test(jev): live pre-phase measurement on this repo" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: End-to-end check (the user, in the running app)**

Ask the user to restart `npm run tauri dev` with this repo open as the project.
1. Type a draft about real code and pause: notes appear as before.
2. Reword one sentence without changing its meaning and pause: the notes stay put (no flicker, rule solid) and no new `claude -p` process appears (`pgrep -fl "claude -p"`).
3. Add a new idea and pause: the old notes stay visible until the new ones start streaming.
4. Click **Try again** after a skipped pause: it re-analyses.
