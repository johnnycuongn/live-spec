# Phase 3: Parallel Notes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write the four note sections (Intent, Faults, Directions, Questions) with up to 4 agents at once, under the user's max-agents setting, so complete notes arrive sooner. "Checked in code" starts as soon as the Faults lane finishes.

**Architecture:**
- **Core:** `livebrainstorm-core` gains:
  - `agents::run_limited`, which runs a group of futures with at most N in flight inside the caller's task, so dropping it cancels all of them.
  - `prompt::section_groups(n)` and per-section prompts. With 1 lane, the prompt is byte-for-byte today's.
  - `analysis::merge_lanes`, which joins lane texts in fixed order and turns failed lanes into notices.
  - `AgentSettings` with `effective_note_agents()`.
- **Shell:** the Tauri `analyze` command:
  - tags every streamed chunk with its lane;
  - runs the lanes concurrently, with the fact-check alongside via a one-shot channel from the Faults lane, so `done` never waits for checks;
  - emits one notice per failed lane.
- **UI:** the UI keeps one buffer per lane and renders them joined in lane order.

**Tech Stack:** Rust 2021 (tokio `join!`, `sync::oneshot`, futures-util `buffer_unordered`), Tauri v2, React 19, vitest.

**Spec:** `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (phase 3: "Parallel notes"; sections "Live analysis pipeline" steps 3–5, "Section groups", "Settings", "IPC changes", "Frontend", "Testing").

## Global Constraints

- **Section groups** (sections in fixed order Intent, Faults, Directions, Questions):

  | n | Lanes |
  |---|---|
  | 1 | [Intent, Faults, Directions, Questions] |
  | 2 | [Intent, Faults], [Directions, Questions] |
  | 3 | [Intent, Faults], [Directions], [Questions] |
  | ≥4 | [Intent], [Faults], [Directions], [Questions] |

- **`effective_note_agents()`:** `min(max_agents, 4)` if `parallel_notes` is on and the provider is not Local; otherwise 1.
- **`AgentSettings`:**
  - `max_agents: u8` is clamped to 1..=8, default 4.
  - `parallel_brief: bool` defaults to true; it's used in phase 4.
  - `parallel_notes: bool` defaults to true.
  - Old settings files load with these defaults.
- **Prompt equivalence:** with all four sections, the assembled system prompt equals today's `ANALYSIS_SYSTEM` exactly. A lane's prompt contains exactly its own `## Heading`s.
- **Events:**
  - `text { text, lane }` and `activity { label, lane }`; the brief uses lane 0.
  - `done { text }` carries the lanes' final texts joined in lane order.
  - Each failed lane emits a `notice` before `done`. Only if every lane fails is there a single `error`.
- **Checks:** they start when the lane containing Faults succeeds, concurrently with the other lanes. If that lane fails, there are no check events. `done` never waits for checks, and check events may arrive before or after `done`.
- **Cancellation:** a new analysis aborts the task, which drops every lane (killing each `claude` child via `kill_on_drop`) and the check.
- **No Settings UI in this phase** (that's phase 5). The new settings work from `settings.json` and their defaults.
- **Process:**
  - Branch `feat/parallel-notes`.
  - Commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Never run `cargo fmt` across the repo; `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests.
  - Futures awaited in the shell's spawned task must avoid closures over borrowed arguments (they trip rustc's Send check). Build lane futures in a `for` loop.
  - Node 20.19+ for `npm run build` (prefix `PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH"`).

## Review Focus

1. **One lane fails** (rate limit, CLI error): the other sections still appear, with one notice naming the missing sections. Only if all lanes fail does the user see an error. Pinned in Task 3 (`a_failed_lane_becomes_a_notice`, `every_lane_failing_is_an_error`).
2. **Lanes stream interleaved:** sections must still render in fixed order and never mix text inside a section. Pinned in Task 5 (`interleaved lanes render in lane order`).
3. **Typing mid-fan-out:** every running agent is cancelled; there are no orphaned `claude` processes. Pinned in Task 2 (`dropping_the_group_cancels_every_job`), plus the manual `pgrep` in Task 6.
4. **The Local provider, `parallel_notes: false`, or `max_agents: 1`:** exactly today's single prompt. Pinned in Task 1 (`note_agents_follow_the_settings`) and Task 3 (`all_four_sections_is_todays_prompt`).
5. **A settings file from before this phase** loads with 4 agents and parallel notes on. Pinned in Task 1 (`files_from_before_agents_get_defaults`).

---

### Task 1: Agent settings

**Files:**
- Modify: `crates/livebrainstorm-core/src/settings.rs`

**Interfaces:**
- Produces:
  - `settings::AgentSettings { pub max_agents: u8, pub parallel_brief: bool, pub parallel_notes: bool }`, serialized camelCase with defaults 4 / true / true
  - the field `Settings.agents`
  - `Settings::max_agents(&self) -> usize`, clamped to 1..=8
  - `Settings::effective_note_agents(&self) -> usize`

- [ ] **Step 0: Branch**

```bash
git switch -c feat/parallel-notes
```

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `settings.rs`)

```rust
    #[test]
    fn note_agents_follow_the_settings() {
        let mut s = Settings::default();
        assert_eq!((s.max_agents(), s.effective_note_agents()), (4, 4));
        s.agents.max_agents = 2;
        assert_eq!(s.effective_note_agents(), 2);
        s.agents.max_agents = 8;
        assert_eq!((s.max_agents(), s.effective_note_agents()), (8, 4), "at most one agent per section");
        s.agents.max_agents = 0;
        assert_eq!((s.max_agents(), s.effective_note_agents()), (1, 1), "clamped to at least one");
        s.agents.max_agents = 200;
        assert_eq!(s.max_agents(), 8, "clamped to at most eight");
        s.agents = AgentSettings { parallel_notes: false, ..AgentSettings::default() };
        assert_eq!(s.effective_note_agents(), 1);
        s.agents = AgentSettings::default();
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert_eq!(s.effective_note_agents(), 1, "one local server would just queue the requests");
    }

    #[test]
    fn files_from_before_agents_get_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"provider":{"kind":"claudeCli","binaryPath":null,"fastModel":"haiku","deepModel":"sonnet"},"debounceMs":900}"#).unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.agents, AgentSettings { max_agents: 4, parallel_brief: true, parallel_notes: true });
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["agents"], serde_json::json!({"maxAgents": 4, "parallelBrief": true, "parallelNotes": true}));
    }
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core settings`
Expected: compile errors, because `AgentSettings`, `agents`, `max_agents` and `effective_note_agents` are not found.

- [ ] **Step 3: Implement**

In `settings.rs`, add the field to `Settings` after `pub jev: JevSettings,`:

```rust
    pub agents: AgentSettings,
```

Add `agents: AgentSettings::default(),` to `impl Default for Settings`. After `impl Default for JevSettings { … }`, add:

```rust
/// How many agents work at once. Notes use at most one per section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSettings {
    /// The cap on agents running at once; clamped to 1..=8.
    pub max_agents: u8,
    /// Map the project with parallel explorers (phase 4).
    pub parallel_brief: bool,
    /// Write the four note sections with parallel agents.
    pub parallel_notes: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self { max_agents: 4, parallel_brief: true, parallel_notes: true }
    }
}
```

Inside `impl Settings`, replace `jev_allowed` and add the agent methods:

```rust
    fn is_local(&self) -> bool {
        matches!(self.provider, ProviderSettings::Local { .. })
    }

    /// Jev sends drafts and code excerpts to TypeSafe, so the Local option
    /// ("private and offline") never uses it, whatever the switches say.
    pub fn jev_allowed(&self) -> bool {
        !self.is_local()
    }

    /// The user's cap on agents running at once.
    pub fn max_agents(&self) -> usize {
        usize::from(self.agents.max_agents.clamp(1, 8))
    }

    /// How many agents write the notes: up to one per section. The Local
    /// model gets one, because a single llama-server would queue the rest.
    pub fn effective_note_agents(&self) -> usize {
        if self.agents.parallel_notes && !self.is_local() {
            self.max_agents().min(4)
        } else {
            1
        }
    }
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 2 new ones; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/settings.rs
git commit -m "feat(core): agent settings and how many agents write the notes" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `agents::run_limited`

**Files:**
- Create: `crates/livebrainstorm-core/src/agents.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod agents;`)

**Interfaces:**
- Produces: `agents::run_limited<F: Future>(limit: usize, jobs: Vec<F>) -> Vec<F::Output>`. At most `max(limit, 1)` jobs are in flight; results come back in job order; there's no spawning, so dropping the future cancels every job.

- [ ] **Step 1: Write the failing tests**

Add `pub mod agents;` to `lib.rs` before `pub mod analysis;`. Create `crates/livebrainstorm-core/src/agents.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn results_come_back_in_job_order() {
        let jobs: Vec<_> = [30u64, 10, 20]
            .into_iter()
            .map(|ms| async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                ms
            })
            .collect();
        assert_eq!(run_limited(3, jobs).await, [30, 10, 20]);
    }

    #[tokio::test]
    async fn never_runs_more_than_the_limit() {
        let (running, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let jobs: Vec<_> = (0..6)
            .map(|_| {
                let (running, peak) = (&running, &peak);
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                }
            })
            .collect();
        run_limited(2, jobs).await;
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_zero_limit_still_runs_every_job() {
        let jobs: Vec<_> = (0..3).map(|i| async move { i }).collect();
        assert_eq!(run_limited(0, jobs).await, [0, 1, 2]);
    }

    #[tokio::test]
    async fn dropping_the_group_cancels_every_job() {
        let finished = AtomicUsize::new(0);
        let jobs: Vec<_> = (0..3)
            .map(|_| {
                let finished = &finished;
                async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    finished.fetch_add(1, Ordering::SeqCst);
                }
            })
            .collect();
        let cut_short = tokio::time::timeout(Duration::from_millis(50), run_limited(3, jobs)).await;
        assert!(cut_short.is_err());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(finished.load(Ordering::SeqCst), 0, "no job may keep running after the group is dropped");
    }
}
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core agents`
Expected: a compile error, because `run_limited` is not found.

- [ ] **Step 3: Implement** (insert at the top of `agents.rs`)

```rust
//! Runs a group of agents (any futures) with at most `limit` in flight and
//! returns their results in job order. Nothing is spawned: the jobs run
//! inside the caller's task, so dropping the group cancels every job, and
//! with it each `claude` child (`kill_on_drop`).

use std::future::Future;

use futures_util::stream::{self, StreamExt};

pub async fn run_limited<F: Future>(limit: usize, jobs: Vec<F>) -> Vec<F::Output> {
    let mut done: Vec<(usize, F::Output)> = stream::iter(jobs.into_iter().enumerate())
        .map(|(i, job)| async move { (i, job.await) })
        .buffer_unordered(limit.max(1))
        .collect()
        .await;
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, out)| out).collect()
}
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 4 new `agents::` tests; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core
git commit -m "feat(core): run a group of agents with a concurrency limit and shared cancellation" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Per-section prompts and lane merging

**Files:**
- Modify: `crates/livebrainstorm-core/src/prompt.rs` (`section_groups`, `analysis_system`, the `analysis_request` signature)
- Modify: `crates/livebrainstorm-core/src/analysis.rs` (`merge_lanes`)
- Modify: `src-tauri/src/commands.rs` (pass `&prompt::SECTIONS` so the shell keeps compiling until Task 4)

**Interfaces:**
- Consumes: `prompt::SECTIONS` (existing).
- Produces:
  - `prompt::section_groups(n: usize) -> Vec<Vec<&'static str>>`
  - `prompt::analysis_system(sections: &[&str]) -> String`
  - `prompt::analysis_request(draft, index, brief, snippets, provider, sections: &[&str]) -> ChatRequest`, which gains the final `sections` argument
  - `analysis::merge_lanes(groups: &[Vec<&str>], results: Vec<Result<String>>) -> Result<(String, Vec<String>)>`, returning the joined text and one notice per failed lane

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `prompt.rs`:

```rust
    /// Today's analysis prompt, verbatim. The four-section assembly must not drift from it.
    const TODAYS_PROMPT: &str = "\
You are LiveBrainstorm, a sharp brainstorming partner sitting beside a software engineer's draft. \
The draft is unfinished thinking typed in real time about the project described in <project>. \
Your job is to make their thinking better, not to finish their writing.

Respond in Markdown with exactly these four sections, in this order, and nothing else:
## Intent
What you think they are trying to do and why, in one or two bullets. If it's ambiguous, say what is ambiguous.
## Faults
Problems with the draft as it stands: conflicts with how the project actually works, things that already exist, \
wrong assumptions, missing edge cases, risky trade-offs. Cite file paths when the project context supports it. \
If nothing is worth raising, write \"- Nothing so far.\"
## Directions
Two or three concrete next moves or alternative approaches, each with its main trade-off.
## Questions
One to three questions whose answers would most change the design.

Rules: bullets only, at most 3 per section, each under 35 words. Ground claims in the project context; \
when the context doesn't cover something, say so instead of guessing. Never restate the draft. Do not use tools.";

    #[test]
    fn all_four_sections_is_todays_prompt() {
        assert_eq!(analysis_system(&SECTIONS), TODAYS_PROMPT);
    }

    #[test]
    fn a_lane_prompt_has_only_its_own_headings() {
        let faults = analysis_system(&["Faults"]);
        assert!(faults.contains("## Faults") && faults.contains("exactly this one section"));
        for other in ["## Intent", "## Directions", "## Questions"] {
            assert!(!faults.contains(other), "{other}");
        }
        assert!(faults.contains("written separately"), "tells the agent the rest is someone else's job");
        let pair = analysis_system(&["Directions", "Questions"]);
        assert!(pair.contains("exactly these two sections") && pair.contains("## Directions") && pair.contains("## Questions"));
        assert!(!pair.contains("## Faults"));
    }

    #[test]
    fn sections_are_grouped_per_the_spec() {
        let names = |n| section_groups(n).iter().map(|g| g.join("+")).collect::<Vec<_>>();
        assert_eq!(names(1), ["Intent+Faults+Directions+Questions"]);
        assert_eq!(names(2), ["Intent+Faults", "Directions+Questions"]);
        assert_eq!(names(3), ["Intent+Faults", "Directions", "Questions"]);
        assert_eq!(names(4), ["Intent", "Faults", "Directions", "Questions"]);
        assert_eq!(names(8), names(4));
        assert_eq!(names(0), names(1));
    }
```

In the same `mod tests`, change the existing `analysis_without_project_still_works` call to pass the sections:

```rust
        let req = analysis_request("add caching to search", None, None, &[], &local(), &SECTIONS);
```

Append inside `mod tests` in `analysis.rs`:

```rust
    #[test]
    fn lanes_join_in_lane_order() {
        let groups = vec![vec!["Intent", "Faults"], vec!["Directions", "Questions"]];
        let merged = merge_lanes(&groups, vec![Ok("## Intent\n- i\n## Faults\n- f\n".into()), Ok("## Directions\n- d\n## Questions\n- q".into())]);
        let (text, notices) = merged.unwrap();
        assert_eq!(text, "## Intent\n- i\n## Faults\n- f\n## Directions\n- d\n## Questions\n- q");
        assert!(notices.is_empty());
    }

    #[test]
    fn a_failed_lane_becomes_a_notice() {
        let groups = vec![vec!["Intent", "Faults"], vec!["Directions", "Questions"]];
        let (text, notices) = merge_lanes(&groups, vec![Ok("## Intent\n- i".into()), Err(Error::Provider("rate limited".into()))]).unwrap();
        assert_eq!(text, "## Intent\n- i");
        assert_eq!(notices, ["Couldn't write Directions and Questions: rate limited"]);
    }

    #[test]
    fn every_lane_failing_is_an_error() {
        let groups = vec![vec!["Intent"], vec!["Faults"]];
        let err = merge_lanes(&groups, vec![Err(Error::Provider("first".into())), Err(Error::Provider("second".into()))]).unwrap_err();
        assert_eq!(err.to_string(), "first");
    }
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core --lib`
Expected: compile errors, because `analysis_system`, `section_groups` and `merge_lanes` are not found, and `analysis_request` takes 5 arguments.

- [ ] **Step 3: Implement the prompts** (`prompt.rs`)

Replace the whole `const ANALYSIS_SYSTEM: &str = "…";` with:

```rust
const ANALYSIS_INTRO: &str = "\
You are LiveBrainstorm, a sharp brainstorming partner sitting beside a software engineer's draft. \
The draft is unfinished thinking typed in real time about the project described in <project>. \
Your job is to make their thinking better, not to finish their writing.

";

/// What each section asks for, in `SECTIONS` order.
const SECTION_GUIDE: [(&str, &str); 4] = [
    ("Intent", "What you think they are trying to do and why, in one or two bullets. If it's ambiguous, say what is ambiguous."),
    (
        "Faults",
        "Problems with the draft as it stands: conflicts with how the project actually works, things that already exist, \
wrong assumptions, missing edge cases, risky trade-offs. Cite file paths when the project context supports it. \
If nothing is worth raising, write \"- Nothing so far.\"",
    ),
    ("Directions", "Two or three concrete next moves or alternative approaches, each with its main trade-off."),
    ("Questions", "One to three questions whose answers would most change the design."),
];

const ANALYSIS_RULES: &str = "

Rules: bullets only, at most 3 per section, each under 35 words. Ground claims in the project context; \
when the context doesn't cover something, say so instead of guessing. Never restate the draft. Do not use tools.";

/// Which sections each agent writes, for `n` agents (see the spec's table).
pub fn section_groups(n: usize) -> Vec<Vec<&'static str>> {
    let [intent, faults, directions, questions] = SECTIONS;
    match n {
        0 | 1 => vec![vec![intent, faults, directions, questions]],
        2 => vec![vec![intent, faults], vec![directions, questions]],
        3 => vec![vec![intent, faults], vec![directions], vec![questions]],
        _ => vec![vec![intent], vec![faults], vec![directions], vec![questions]],
    }
}

/// The system prompt for an agent writing `sections`. With all four it is
/// exactly the single-agent prompt.
pub fn analysis_system(sections: &[&str]) -> String {
    let lead = match sections.len() {
        1 => "exactly this one section and nothing else".to_owned(),
        k => format!("exactly these {} sections, in this order, and nothing else", ["two", "three", "four"][k.clamp(2, 4) - 2]),
    };
    let mut prompt = format!("{ANALYSIS_INTRO}Respond in Markdown with {lead}:");
    for name in sections {
        let guide = SECTION_GUIDE.iter().find(|(n, _)| n == name).map_or("", |(_, g)| *g);
        prompt.push_str(&format!("\n## {name}\n{guide}"));
    }
    if sections.len() < SECTIONS.len() {
        prompt.push_str("\n\nThe other parts of the notes are written separately; write only the sections above.");
    }
    prompt.push_str(ANALYSIS_RULES);
    prompt
}
```

Change the `analysis_request` signature to:

```rust
pub fn analysis_request(
    draft: &str,
    index: Option<&ProjectIndex>,
    brief: Option<&str>,
    snippets: &[Snippet],
    provider: &Provider,
    sections: &[&str],
) -> ChatRequest {
```

and in its body replace `system: ANALYSIS_SYSTEM.to_owned(),` with `system: analysis_system(sections),`.

In `src-tauri/src/commands.rs`, change the one call `prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider)` to end with `, &provider, &prompt::SECTIONS)`.

- [ ] **Step 4: Implement the merge** (`analysis.rs`, after `pre_phase`)

```rust
/// Joins the lanes' texts in lane order. A failed lane becomes a notice
/// naming its sections; only if every lane failed is the first error returned.
pub fn merge_lanes(groups: &[Vec<&str>], results: Vec<crate::Result<String>>) -> crate::Result<(String, Vec<String>)> {
    let (mut texts, mut notices, mut first_error) = (Vec::new(), Vec::new(), None);
    for (sections, result) in groups.iter().zip(results) {
        match result {
            Ok(text) => texts.push(text.trim().to_owned()),
            Err(e) => {
                notices.push(format!("Couldn't write {}: {e}", sections.join(" and ")));
                first_error.get_or_insert(e);
            }
        }
    }
    if texts.is_empty() {
        return Err(first_error.unwrap_or_else(|| Error::Provider("No notes were written.".into())));
    }
    Ok((texts.join("\n"), notices))
}
```

- [ ] **Step 5: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 6 new ones and the updated `analysis_without_project_still_works`; no warnings. The shell compiles with `&prompt::SECTIONS`.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core src-tauri/src/commands.rs
git commit -m "feat(core): per-section prompts, section groups, and lane merging" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Shell — write the notes in lanes

**Files:**
- Modify: `src-tauri/src/commands.rs` (`UiEvent::Text`/`Activity` gain `lane`; `on_chunk`; the `analyze` body)
- Modify: `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes: `Settings::effective_note_agents` (Task 1); `agents::run_limited` (Task 2); `prompt::{section_groups, analysis_request(…, sections)}` and `analysis::merge_lanes` (Task 3); the phase 1–2 pre-phase and checks.
- Produces the IPC contract Task 5 mirrors:
  - `{type:"text", text, lane}` and `{type:"activity", label, lane}` (the brief uses lane 0)
  - `notice` per failed lane, before `done`
  - check events whenever the Faults lane is done

- [ ] **Step 1: Tag events with their lane**

In `UiEvent`, replace the first two variants with:

```rust
    Text { text: String, lane: usize },
    Activity { label: String, lane: usize },
```

Replace `on_chunk` (used by `build_brief`) with:

```rust
/// The brief streams as one lane.
fn on_chunk(ch: &Channel<UiEvent>) -> impl FnMut(Chunk) + Send + '_ {
    move |c| match c {
        Chunk::Text(text) => send(ch, UiEvent::Text { text, lane: 0 }),
        Chunk::Activity(label) => send(ch, UiEvent::Activity { label, lane: 0 }),
    }
}
```

Add to the imports:

```rust
use livebrainstorm_core::agents;
use tokio::sync::oneshot;
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
        let provider = match provider::resolve(&task_app, Tier::Fast).await {
            Ok(provider) => provider,
            Err(e) => return send(&on_event, UiEvent::Error { message: e.to_string() }),
        };
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
        let PreOutcome::Analyze { snippets, notice } = pre else { return send(&on_event, UiEvent::Skipped) };
        send(&on_event, UiEvent::Started);
        if let Some(message) = notice {
            send(&on_event, UiEvent::Notice { message });
        }

        // Each lane writes some of the sections; up to one agent per section.
        // A plain `for` loop, not `.map(…)`: closures over borrowed items trip
        // rustc's Send check inside this spawned task.
        let groups = prompt::section_groups(settings.effective_note_agents());
        let (faults_tx, faults_rx) = oneshot::channel::<String>();
        let mut faults_tx = Some(faults_tx);
        let mut lanes = Vec::new();
        for (lane, sections) in groups.iter().enumerate() {
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, sections);
            let tx = if sections.contains(&"Faults") { faults_tx.take() } else { None };
            let (provider, on_event) = (&provider, &on_event);
            lanes.push(async move {
                let mut sink = |chunk: Chunk| match chunk {
                    Chunk::Text(text) => send(on_event, UiEvent::Text { text, lane }),
                    Chunk::Activity(label) => send(on_event, UiEvent::Activity { label, lane }),
                };
                let result = provider.stream(&req, &mut sink).await;
                if let (Some(tx), Ok(text)) = (tx, &result) {
                    let _ = tx.send(text.clone());
                }
                result
            });
        }
        drop(faults_tx);

        let write = async {
            match analysis::merge_lanes(&groups, agents::run_limited(groups.len(), lanes).await) {
                Ok((text, notices)) => {
                    for message in notices {
                        send(&on_event, UiEvent::Notice { message });
                    }
                    send(&on_event, UiEvent::Done { text });
                }
                Err(e) => send(&on_event, UiEvent::Error { message: e.to_string() }),
            }
        };
        // "Checked in code" starts as soon as the Faults lane finishes, while
        // other lanes may still stream; `done` never waits for it. The next
        // analysis aborts this task, lanes and checks included.
        let check = async {
            let Ok(text) = faults_rx.await else { return }; // that lane failed
            if !settings.jev_check_enabled() {
                return;
            }
            let (Some(index), Some(jev)) = (index.as_deref(), jev.as_ref()) else { return };
            if let Some(faults) = prompt::section_body(&text, "Faults") {
                check::run(jev, index, &faults, |ev| send(&on_event, ev.into())).await;
            }
        };
        tokio::join!(write, check);
    });
    replace_task(&app.state::<AppState>().analysis, task);
    Ok(())
}
```

- [ ] **Step 3: Build and lint**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: no warnings and all tests pass. If rustc reports "implementation of FnOnce is not general enough" for the spawned task, the cause is a closure over a borrowed argument in this task. Replace it with a loop or an owned value (see AGENTS.md), ledger a ruling, and re-run.

- [ ] **Step 4: Update the docs**

1. In `AGENTS.md`, under "### Live analysis flow", replace step 4 with:

```markdown
4. After `started`, the sections are written by `Settings::effective_note_agents()` lanes at once (`prompt::section_groups`: 1 → all four, 2 → Intent+Faults | Directions+Questions, 3 → Intent+Faults | Directions | Questions, 4 → one each; always 1 for the Local provider). Lanes run through `agents::run_limited`, so dropping the task cancels every `claude` child. Every chunk is `text { text, lane }`; `done` carries the lanes joined in lane order (`analysis::merge_lanes`), and a failed lane becomes a `notice` naming its sections. The fact-check starts when the Faults lane finishes (a oneshot from that lane), concurrently with the other lanes; `done` never waits for it. Events: `text`, `activity`, `started`, `skipped`, `notice`, `checking`, `check`, `done`, `error`, `progress`.
```

2. In `AGENTS.md`, under "## Cross-language contracts", add:

```markdown
- **Lanes:** `prompt::section_groups` (Rust) decides which sections each `lane` carries; `src/lib/lanes.ts` joins lane buffers in lane order. `analysis_system(&SECTIONS)` must stay byte-identical to the single-agent prompt (`all_four_sections_is_todays_prompt`).
```

3. In `README.md`, under "**Live analysis:**", append:

```markdown
With the Claude or API providers, the four sections are written by up to four agents at once (`agents.maxAgents` in settings, default 4; `agents.parallelNotes: false` turns it off). Each agent is a separate model call, so this uses more of your subscription or API budget per pause.
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri AGENTS.md README.md
git commit -m "feat: write the notes in parallel lanes; checks start with the Faults lane" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Frontend — join lane buffers

**Files:**
- Modify: `src/lib/ipc.ts` (`lane` on `text`/`activity`; `AgentSettings`; `Settings.agents`)
- Create: `src/lib/lanes.ts`, `src/lib/lanes.test.ts`
- Modify: `src/lib/analysisState.ts` (`lanes` buffer)
- Modify: `src/lib/analysisState.test.ts` (text events carry `lane`)

**Interfaces:**
- Consumes: the IPC contract from Task 4.
- Produces:
  - `appendLane(lanes: string[], lane: number, text: string): string[]`
  - `joinLanes(lanes: string[]): string`
  - `AnalysisState.lanes: string[]`

- [ ] **Step 1: Mirror the IPC contract** (`src/lib/ipc.ts`)

In `UiEvent`, replace the `text` and `activity` members with:

```ts
  | { type: "text"; text: string; lane: number }
  | { type: "activity"; label: string; lane: number }
```

Replace the `Settings` interface with:

```ts
export interface AgentSettings {
  maxAgents: number;
  parallelBrief: boolean;
  parallelNotes: boolean;
}

export interface Settings {
  provider: ProviderSettings;
  debounceMs: number;
  jev: JevSettings;
  agents: AgentSettings;
}
```

- [ ] **Step 2: Write the failing tests**

`src/lib/lanes.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { appendLane, joinLanes } from "./lanes";
import { parseSections } from "./sections";

describe("lanes", () => {
  it("interleaved lanes render in lane order", () => {
    let lanes: string[] = [];
    lanes = appendLane(lanes, 1, "## Directions\n- go");
    lanes = appendLane(lanes, 0, "## Intent\n- why");
    lanes = appendLane(lanes, 1, " faster");
    lanes = appendLane(lanes, 0, "\n## Faults\n- bug");
    expect(joinLanes(lanes)).toBe("## Intent\n- why\n## Faults\n- bug\n## Directions\n- go faster");
    expect(parseSections(joinLanes(lanes))).toEqual({ intent: "- why", faults: "- bug", directions: "- go faster" });
  });

  it("a lane that hasn't started leaves no gap", () => {
    expect(joinLanes(appendLane([], 2, "## Questions\n- q"))).toBe("## Questions\n- q");
  });

  it("never mutates the previous buffers", () => {
    const before = ["## Intent\n- a"];
    appendLane(before, 0, " b");
    expect(before).toEqual(["## Intent\n- a"]);
  });
});
```

In `src/lib/analysisState.test.ts`, give every `{ type: "text", … }` event a lane by adding `lane: 0`. For example, change `{ type: "text", text: "## Intent\n" }` to `{ type: "text", text: "## Intent\n", lane: 0 }`, and do the same for `{ type: "text", text: "- a" }`. Then append:

```ts
describe("parallel lanes", () => {
  it("builds the notes from lanes in lane order", () => {
    const s = run([
      { type: "started" },
      { type: "text", text: "## Directions\n- d", lane: 1 },
      { type: "text", text: "## Intent\n- i", lane: 0 },
    ]);
    expect(s.markdown).toBe("## Intent\n- i\n## Directions\n- d");
  });

  it("started clears the lane buffers too", () => {
    const s = run([{ type: "started" }, { type: "text", text: "## Intent\n- old", lane: 0 }, { type: "done", text: "## Intent\n- old" }]);
    const next = applyEvent(startAnalysis(s, "v2"), { type: "started" });
    expect(applyEvent(next, { type: "text", text: "## Faults\n- new", lane: 1 }).markdown).toBe("## Faults\n- new");
  });
});
```

- [ ] **Step 3: Run and confirm the failure**

Run: `npm test`
Expected: FAIL. `./lanes` cannot be resolved, and the parallel-lanes tests produce the wrong markdown because text is still appended without regard to `lane`.

- [ ] **Step 4: Implement**

`src/lib/lanes.ts`:

```ts
// Parallel agents stream their sections on separate lanes. Each lane keeps
// its own buffer, and the notes are the lanes joined in lane order, so
// interleaved chunks never mix inside a section. Lanes mirror
// prompt::section_groups in Rust.

export function appendLane(lanes: string[], lane: number, text: string): string[] {
  const next = [...lanes];
  while (next.length <= lane) next.push("");
  next[lane] += text;
  return next;
}

export const joinLanes = (lanes: string[]) => lanes.filter((l) => l.length > 0).join("\n");
```

In `src/lib/analysisState.ts`:
- Add `import { appendLane, joinLanes } from "./lanes";`.
- Add `lanes: string[];` to `AnalysisState`, and `lanes: [],` to `initialAnalysis`.
- Replace the `text` and `started` cases with:

```ts
    case "text": {
      const lanes = appendLane(s.lanes, ev.lane, ev.text);
      return { ...s, lanes, markdown: joinLanes(lanes) };
    }
```

```ts
    case "started":
      return { ...s, lanes: [], markdown: "", checks: [], checking: null, notice: null, error: null };
```

- [ ] **Step 5: Verify**

Run: `npm test && npm run typecheck && PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH" npm run build`
Expected: 24 tests pass (sections 3, checks 5, analysisState 13, lanes 3); no type errors; the build prints `✓ built in`. If typecheck flags `App.tsx`'s brief handler, note that it only reads `ev.text` and `ev.label`, which still exist, so it needs no change.

- [ ] **Step 6: Commit**

```bash
git add src
git commit -m "feat(ui): join parallel lanes in section order" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Measure 1 vs 4 lanes and verify end to end

**Files:**
- Create: `crates/livebrainstorm-core/tests/notes_live.rs`
- Modify: `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (a "Measurements (phase 3)" section)
- Modify: `AGENTS.md` (the live-test command)

**Interfaces:**
- Consumes: `agents::run_limited`, `prompt::{section_groups, analysis_request, section_body, SECTIONS}`, `analysis::merge_lanes`, `llm::{ClaudeCli, Provider, Chunk}`, `shell_env::{login_shell_path, find_executable}`, `project::{scan, retrieve}`.

- [ ] **Step 1: Write the opt-in live test** (`crates/livebrainstorm-core/tests/notes_live.rs`)

```rust
//! Runs Claude Code for real (uses the subscription): times complete notes
//! for this repo with 1 lane and with 4 lanes, alternating, twice each.
//!   cargo test -p livebrainstorm-core --test notes_live -- --ignored --nocapture

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use livebrainstorm_core::llm::{Chunk, ClaudeCli, Provider};
use livebrainstorm_core::project::{retrieve, scan};
use livebrainstorm_core::{agents, analysis, prompt, shell_env};

#[tokio::test]
#[ignore = "runs Claude Code and uses the subscription"]
async fn four_lanes_vs_one() {
    let path_env = shell_env::login_shell_path();
    let Some(binary) = shell_env::find_executable("claude", path_env.as_deref()) else {
        eprintln!("skipped: claude not found");
        return;
    };
    let provider = Provider::ClaudeCli(ClaudeCli { binary, model: "haiku".into(), path_env });
    let index = scan(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))).unwrap();
    let draft = "Every pause spawns a fresh claude -p process from ClaudeCli::stream. I want to keep one process alive, reuse it across analyze calls, and kill it when replace_task aborts the old analysis.";
    let snippets = retrieve(&index, draft, 5, 30_000);

    for n in [1, 4, 1, 4] {
        let groups = prompt::section_groups(n);
        let first_text: Mutex<Option<Duration>> = Mutex::new(None);
        let started = Instant::now();
        let mut lanes = Vec::new();
        for sections in &groups {
            let req = prompt::analysis_request(draft, Some(&index), None, &snippets, &provider, sections);
            let (provider, first_text) = (&provider, &first_text);
            lanes.push(async move {
                let mut sink = |chunk: Chunk| {
                    if matches!(chunk, Chunk::Text(_)) {
                        first_text.lock().unwrap().get_or_insert(started.elapsed());
                    }
                };
                provider.stream(&req, &mut sink).await
            });
        }
        let (text, notices) = analysis::merge_lanes(&groups, agents::run_limited(n, lanes).await).expect("notes");
        let complete = started.elapsed();
        let missing: Vec<_> = prompt::SECTIONS.iter().filter(|s| prompt::section_body(&text, s).is_none()).collect();
        eprintln!(
            "lanes={n}  first text {:>5} ms  complete {:>5} ms  missing={missing:?}  notices={notices:?}",
            first_text.lock().unwrap().map_or(0, |d| d.as_millis()),
            complete.as_millis()
        );
    }
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p livebrainstorm-core --test notes_live -- --ignored --nocapture 2>&1 | grep -E "lanes=|skipped|test result|panicked"`
Expected: four `lanes=` lines (1, 4, 1, 4), each with `missing=[]`, then `test result: ok`. Record what actually happens. If a 4-lane run is missing a section or is slower than both 1-lane runs, report that in the measurements; don't change code here.

- [ ] **Step 3: Record the measurements** (append to the spec, using the real numbers)

```markdown
## Measurements (phase 3, 2026-10-14)

`cargo test -p livebrainstorm-core --test notes_live -- --ignored --nocapture`, Claude Code `haiku`, this repo, keyword snippets:

| Lanes | First text | Complete notes | Missing sections |
|---|---|---|---|
| 1 | <ms>, <ms> | <ms>, <ms> | <…> |
| 4 | <ms>, <ms> | <ms>, <ms> | <…> |

- <One sentence: did 4 lanes finish sooner than 1, by how much, and what it cost (4 `claude` processes per pause).>
```

Replace every `<…>` with the measured values before committing.

- [ ] **Step 4: Note the live test in AGENTS.md**

Under "### Jev (TypeSafe)", after the existing live-test bullet, add:

```markdown
- Parallel-notes live test (runs Claude Code, uses the subscription; prints 1-lane vs 4-lane timings): `cargo test -p livebrainstorm-core --test notes_live -- --ignored --nocapture`.
```

- [ ] **Step 5: Final verification and commit**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm test && npm run typecheck`
Expected: no warnings; all tests pass.

```bash
git add crates/livebrainstorm-core/tests/notes_live.rs docs/superpowers/specs AGENTS.md
git commit -m "test: live 1-lane vs 4-lane notes measurement" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: End-to-end check (the user, in the running app)**

Ask the user to restart `npm run tauri dev` with this repo open, then:
1. Pause on a draft: the four sections fill in, possibly out of order in time but always in the right place.
2. While they stream, run `pgrep -fl "claude -p" | wc -l`: up to 4.
3. Type again mid-stream: after the next pause, the count returns to the new run's agents only, with no orphans (`pgrep` once it finishes: 0).
4. "Checked in code" may appear before Directions and Questions finish.
