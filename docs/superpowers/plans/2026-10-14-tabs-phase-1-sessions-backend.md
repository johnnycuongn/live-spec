# Tabs, windows and usage, Phase 1: Sessions backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each window works on tabs that the backend keeps on disk: a tab's draft, title and last 5 analyses survive restarts, tabs analyse in parallel (one run per tab, one brief build per project), and the existing single-pane UI runs on the window's active tab. The only visible change is that notes survive restarts.

**Architecture:**
- **Core (`livebrainstorm-core`):**
  - New `store.rs`: atomic writes, keeping unparseable files as `.bad`, and `load_json`. `InstructionStore` moves onto it.
  - New `sessions` module (`mod.rs`, `ids.rs`, `store.rs`): the types, id validation, `SessionStore` (workspace + one file per tab, under one `std` mutex) and `Recorder`, which saves one run's notes and checks to a tab's history.
- **Shell (`src-tauri`):**
  - `AppState` gets a project registry (one index and brief per root), per-tab analysis slots and per-root brief slots.
  - `analyze` takes a tab id and the context key, and saves its snapshot before it sends `done`.
  - `build_brief` takes a root, refuses a second build of that root, and broadcasts `brief-status`. The session commands are thin wrappers that emit `workspace-changed`.
- **Frontend:**
  - Building blocks: IPC types and wrappers, `events.ts`, `windowId.ts`, `tabTitle.ts`, `legacyDrafts.ts`, `tabContext.ts`.
  - `TabSession`, a per-tab controller outside React, replaces `useLiveAnalysis`. `useTabSession` reads it.
  - `App.tsx` drives one `TabSession` for the window's active tab. *Open a project* switches to this window's tab for that project, creating one if there is none. A fresh workspace imports the old `localStorage` drafts.

**Tech Stack:** Rust 2021 (serde, serde_json, tempfile for tests), Tauri v2 (`Emitter`, `Channel`), React 19 + Tailwind v4, vitest 3 (node environment, fake timers, `vi.mock`), `@tauri-apps/api` 2.12 (`event`, `window`).

**Spec:** `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md`. This plan covers:
- Section 1 ("Data model and persistence") in full, including `move_tab` and `add_window` at the store level.
- Section 2 ("Backend runtime"), except `open_window` and the `move_tab` command (Phase 3), the `settings-changed` and `instructions-changed` events (Phase 3), and the usage ledger (Phase 4).
- From Section 4, the `TabSession` controller, `tabTitle`, and the context-per-tab rule. From Section 6, the error-handling rows that live in this code.
- Phase 1's share of "Documentation updates".

**Prerequisites:**
- `main` at `4c42d58`.
- Work happens on branch `feat/tabs-windows-usage`. The executor creates it in a worktree (superpowers:using-git-worktrees) before Task 1.
- Phases 2, 3 and 4 follow on the same branch and build on the names this plan produces (listed next).

### What this plan leaves behind for Phases 2–4

- **Core:**
  - `livebrainstorm_core::store::{write_atomic, set_aside, Loaded, load_json}`.
  - `livebrainstorm_core::sessions::{HISTORY_LEN, TITLE_CHARS, Snapshot, Tab, TabSummary, WindowEntry, Workspace, WindowView, LegacyDraft, valid_id, tab_id, window_id, first_line, SessionStore, Recorder}`.
  - `SessionStore` has every method in the contract, plus `window_of(&self, id) -> Option<String>`.
  - `save_draft` updates closed tabs too.
  - `move_tab` moves the source window's `active` off the moved tab, the same way `close_tab` does.
  - `window_ids()` creates `w-1` only when `workspace.json` is missing. A workspace that lists no windows gives an empty list.
- **Shell:**
  - `state.rs`: `ProjectEntry`; the `AppState` fields `projects`, `analyses`, `brief_tasks` and `sessions`; `AppState::new(data_dir, config_dir)`.
  - `state.rs` helpers, all `pub`: `AppState::project_entry`, `AppState::evict_unused_projects`, `replace_task(slots, key, task)` and `cancel_task(slots, key)`.
  - `events.rs`: `BRIEF_STATUS`, `WORKSPACE_CHANGED`, `BriefState`, `BriefStatus` and `WorkspaceChanged`, plus the emit helpers `brief_status` and `workspace_changed`. It declares none of the Phase 3 or Phase 4 events.
  - `analyze` reads the tab's root in the command body (`let root = app.state::<AppState>().sessions.open_tab_root(&tab_id)?;`, so a closed tab returns an `Err`) and moves it into the spawned task, which binds `state` (the `AppState`). The task keeps today's lane loop: `let (provider, on_event) = (&provider, &on_event);`, a `sink` that `match`es `Chunk`, and `provider.stream(&req, &mut sink).await`.
  - `build_brief`'s spawned task binds `task_app` and `index: Arc<ProjectIndex>`, and keeps today's three `provider.stream` calls and the `on_chunk` helper.
- **Frontend:**
  - `ipc.ts`: types and wrappers.
  - `events.ts`: `onBriefStatus` and `onWorkspaceChanged`.
  - `windowId.ts`: `currentWindowId`.
  - `tabTitle.ts`: `firstLine` and `tabTitle`.
  - `tabSession.ts`: `TabSession`, `TabSessionDeps`, `TabView`, `MIN_CHARS` and `normalize`. There is no `view()` yet, and `contextDirty` and `viewing` are always `false` and `0`.
  - `useTabSession.ts`: `useTabSession`.
  - `useLiveAnalysis.ts` is deleted.
  - `App.tsx` is the single-pane wiring, and `NotesPane` takes `analysis: AnalysisState & { stale; rerun }`. Its `RuleState` and `RULE` definitions are untouched, because Phase 2's Task 1 moves them.
  - `legacyDrafts.ts` and `tabContext.ts` are used only by Phase 1's `App.tsx`. Phase 2's Task 9 deletes them once nothing imports them.

## Global Constraints

- **Files:** everything lives under `<app_data>/sessions/`: `workspace.json` (windows, their tab order, the active tab) and `tabs/<tabId>.json` (one tab).
- **History:** `HISTORY_LEN = 5` analyses per tab, newest first. Skips and errors never add one. A snapshot is recorded *before* `done` is sent.
- **Ids:**
  - Tab ids are `t-<unix millis, hex>-<per-process counter, hex>`. Window ids are `w-<nextWindow>`, in decimal.
  - Every id that arrives over IPC must match `^[tw]-[0-9a-f-]{1,40}$` (`valid_id`) before it is used. Anything else is `Error::Precondition("Unknown tab.")`.
- **Error messages, verbatim:**
  - `Error::Precondition`: "Unknown tab.", "This tab was closed.", "That tab is already open in another window.", "This window was closed.", "Close this tab before deleting it.", "That folder no longer exists. Open it again from its new place."
  - Notices and banners: "Couldn't save these notes: {error}", "Couldn't save your draft: {error}", "Couldn't bring over your earlier drafts: {error}", "Couldn't read a tab; kept it as {file}", "Couldn't read the saved windows; kept them as {file}".
- **A closed tab is any tab file that no window lists.** *Recently closed* sorts by `closedAt`, else `updatedAt`, newest first.
- **Store behaviour:**
  - Writes are atomic (`<file>.tmp`, then rename).
  - A file that exists but doesn't parse is renamed to the first free `.bad`, `.bad.1`, … before anything is written. A read error that isn't a parse error is reported and never renamed or overwritten.
  - One `std::sync::Mutex` guards the workspace and the tab cache. It is held only for synchronous file I/O, never across `.await`.
- **Tab titles:** `first_line` is the draft's first non-empty line, trimmed, at most `TITLE_CHARS = 48` Unicode scalars, with trailing whitespace trimmed after the cut. "Whitespace" is Unicode White_Space on both sides: Rust's `str::trim`, and `\p{White_Space}` in TS, because JS's `trim()` strips U+FEFF and keeps U+0085. Rust and TS share test strings.
- **Draft saves:** `createSaver` keyed by tab id, 500 ms after the last edit. Flushed before switching tabs and on `pagehide`. A failed save shows as a banner and is retried with the next edit.
- **Concurrency:**
  - No global cap: one `analyze` task per tab and one `build_brief` task per root.
  - `std` locks are never held across `.await`.
  - Inside the shell's spawned tasks, avoid closures over borrowed arguments in futures that are awaited (`Option::filter(|_| …)`, `.map(|(i, &x)| …)`). They trip rustc's Send check.
- **Unchanged pipeline:** provider at `Tier::Fast`, pre-phase, lanes, merge and fact-check are today's. `prompt.rs` is not touched, so the existing prompt tests (`all_four_sections_is_todays_prompt` and the rest) are the byte-identical guard.
- **Context per tab:** `analysisContextKey(root, brief?.createdAt, instructions.current)`. The window calls `setContext` for a tab only after it has loaded that root's summary *and* its instructions.
- **Process:**
  - One commit per task on `feat/tabs-windows-usage`.
  - Never run `cargo fmt` across the repo: match the hand-wrapped ~120-column style.
  - `cargo clippy --workspace --all-targets` stays warning-free. `src-tauri` has no tests: shell tasks are verified with `cargo check -p livebrainstorm` and clippy.
  - UI uses semantic colour tokens and `ui.tsx` primitives.
  - The agent shell has Node 20.11, so `npm run tauri dev` can't run there. Manual checks are a checklist for the user.
  - Every commit message ends with:
    ```
    Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
    Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47
    ```

## Review Focus

1. **A tab or window id from the webview that isn't one of ours** (`t-../../x`, `t-1/2`, `T-1`, 41 hex characters). Expected: "Unknown tab." before any file is touched, in the command and again in the store. Pinned in Task 2 (`valid_id_accepts_only_our_ids`) and Task 3 (`ids_that_fail_validation_never_reach_the_disk`).
2. **Relaunching with a draft that matches its latest analysis, or differs only in whitespace.** Expected: the notes show at once and no model call starts. That holds even when the project summary or instructions arrive late. Pinned in Task 8 (`shows the latest analysis and starts no run when the draft matches it`, `an edit since the latest analysis shows as stale, and the next pause compares against it`) and Task 9 (`tabContext` `waits for both the project summary and the instructions`).
3. **Fact-checks that straddle `done`, or that an edit cuts off.** Expected: the saved snapshot holds what the notes pane showed. Checks that arrived before `done` are in it, later ones update it, and a cut-off run restores as "Checks stopped when you edited." Pinned in Task 4 (`a_run_is_saved_at_done_with_the_checks_that_came_first`, `checks_cut_off_by_an_edit_stay_short_of_their_total`) and Task 8 (`checks an edit cut off read as stopped, as they did live`).
4. **A damaged or unreadable session file.** Expected:
   - A corrupt `workspace.json` or tab file is kept as `.bad`, never replacing an earlier `.bad`, and the app still starts.
   - A lost workspace leaves every tab under *Recently closed*.
   - A file that can't be read (a directory in the way) is never renamed or overwritten.

   Pinned in Task 1 (`a_corrupt_file_is_kept_and_earlier_copies_are_never_replaced`) and Task 3 (`a_corrupt_workspace_is_kept_and_its_tabs_show_as_closed`, `a_corrupt_tab_is_kept_and_skipped`, `a_tab_file_that_cannot_be_read_is_reported_and_left_alone`, `a_workspace_that_cannot_be_read_is_never_overwritten`).
5. **Upgrading from today's build.** Expected:
   - Every non-empty `localStorage` draft becomes a tab exactly once, the root `none` becomes no project, and the last project's tab is active.
   - The keys are removed only after the import succeeds.
   - A relaunch never imports again.

   Pinned in Task 4 (`legacy_drafts_are_imported_once`, `without_a_last_project_the_no_project_draft_is_active`) and Task 7 (`importLegacy` tests).

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/livebrainstorm-core/src/store.rs` (new) | `write_atomic`, `set_aside`, `Loaded`, `load_json` | 1 |
| `crates/livebrainstorm-core/src/instructions.rs` | `InstructionStore` on the `store` helpers | 1 |
| `crates/livebrainstorm-core/src/jev/check.rs` | `CheckResult` and `Verdict` gain `Deserialize` | 1 |
| `crates/livebrainstorm-core/src/lib.rs` | `pub mod store;`, `pub mod sessions;` | 1, 2 |
| `crates/livebrainstorm-core/src/sessions/mod.rs` (new) | Types, `HISTORY_LEN`, `TITLE_CHARS`, `first_line`, `Snapshot::apply_check`, `Tab::summary`, workspace helpers, re-exports | 2, 3, 4 |
| `crates/livebrainstorm-core/src/sessions/ids.rs` (new) | `valid_id`, `tab_id`, `window_id` | 2 |
| `crates/livebrainstorm-core/src/sessions/store.rs` (new) | `SessionStore`; `Recorder` | 3, 4 |
| `src-tauri/src/state.rs` | `ProjectEntry`, registry, keyed task slots, `sessions`, eviction | 5, 6 |
| `src-tauri/src/events.rs` (new) | `brief-status`, `workspace-changed` | 5, 6 |
| `src-tauri/src/commands.rs` | Project registry commands, per-root briefs, per-tab `analyze` with recording; session commands | 5, 6 |
| `src-tauri/src/lib.rs` | `mod events;`, new handlers | 5, 6 |
| `src/lib/ipc.ts` | Session types and wrappers; new `analyze` / `buildBrief` / `cancelAnalysis` signatures | 7, 9 |
| `src/lib/events.ts`, `windowId.ts`, `tabTitle.ts`, `legacyDrafts.ts` (+ tests) | Event wrappers, window id, titles, the one-time import | 7 |
| `src/lib/tabSession.ts`, `useTabSession.ts` (+ tests) | Per-tab controller and its hook | 8 |
| `src/lib/tabContext.ts` (+ test) | When a tab's context is ready, and its key | 9 |
| `src/App.tsx`, `src/components/NotesPane.tsx` | Single-pane wiring on the active tab | 9 |
| `src/lib/useLiveAnalysis.ts` (delete) | Replaced by `TabSession` | 9 |
| `AGENTS.md` | Phase 1 docs | 10 |

---

### Task 1: Core: shared file helpers (`store.rs`), and checks that read back from disk

**Files:**
- Create: `crates/livebrainstorm-core/src/store.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod store;`)
- Modify: `crates/livebrainstorm-core/src/instructions.rs:164-195` (`read`, `set_aside` and `write` move onto the helpers; `load` and `update` are unchanged)
- Modify: `crates/livebrainstorm-core/src/jev/check.rs:11,35-54` (`Deserialize`)

**Interfaces:**
- Consumes: nothing new.
- Produces (`livebrainstorm_core::store`):
  ```rust
  /// Atomic write: `<file>.tmp` then rename; creates the parent dir.
  pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()>;
  /// Renames a corrupt file to the first free `<name>.bad`, `<name>.bad.1`, …; never replaces an earlier copy.
  pub fn set_aside(path: &Path) -> Option<PathBuf>;
  pub enum Loaded<T> { Missing, Parsed(T), Corrupt { kept: Option<PathBuf> } }
  /// Err only for read errors other than NotFound (those files are left in place).
  pub fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Loaded<T>>;
  ```
- Produces (`livebrainstorm_core::jev::check`): `Verdict` derives `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize` (it keeps `Copy`), and `CheckResult` derives `Debug, Clone, PartialEq, Serialize, Deserialize`. Both stay `rename_all = "camelCase"`.

- [ ] **Step 1: Write the failing tests**

Create `crates/livebrainstorm-core/src/store.rs` with stub bodies and its tests:

```rust
//! File helpers shared by the stores that keep JSON under the app's folders
//! (instructions, sessions, usage): atomic writes, and keeping a file that
//! doesn't parse instead of overwriting it, because it may hold the user's work.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::Result;

/// What `load_json` found.
#[derive(Debug)]
pub enum Loaded<T> {
    /// No file: a first run.
    Missing,
    Parsed(T),
    /// The file didn't parse. It was renamed to `kept`; `None` if even that failed.
    Corrupt { kept: Option<PathBuf> },
}

/// Atomic write: `<file>.tmp` then rename; creates the parent dir.
pub fn write_atomic(_path: &Path, _bytes: &[u8]) -> Result<()> {
    unimplemented!()
}

/// Renames a corrupt file to the first free `<name>.bad`, `<name>.bad.1`, …;
/// never replaces an earlier copy.
pub fn set_aside(_path: &Path) -> Option<PathBuf> {
    unimplemented!()
}

/// Err only for read errors other than NotFound (those files are left in place).
pub fn load_json<T: DeserializeOwned>(_path: &Path) -> Result<Loaded<T>> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Deserialize)]
    struct Doc {
        n: u32,
    }

    #[test]
    fn writes_atomically_into_a_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/doc.json");
        write_atomic(&path, br#"{"n":1}"#).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"n":1}"#);
        assert!(!dir.path().join("a/b/doc.json.tmp").exists(), "the temporary file is renamed, not left behind");
        write_atomic(&path, br#"{"n":2}"#).unwrap();
        assert!(matches!(load_json::<Doc>(&path).unwrap(), Loaded::Parsed(Doc { n: 2 })));
    }

    #[test]
    fn a_missing_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load_json::<Doc>(&dir.path().join("nope.json")).unwrap(), Loaded::Missing));
    }

    #[test]
    fn a_corrupt_file_is_kept_and_earlier_copies_are_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        for (i, body) in ["first", "second", "third"].iter().enumerate() {
            std::fs::write(&path, body).unwrap();
            let Loaded::Corrupt { kept: Some(kept) } = load_json::<Doc>(&path).unwrap() else { panic!("not kept") };
            let name = if i == 0 { "doc.json.bad".to_owned() } else { format!("doc.json.bad.{i}") };
            assert_eq!(kept, dir.path().join(name));
            assert!(!path.exists(), "the damaged file moved aside");
        }
        assert_eq!(std::fs::read_to_string(dir.path().join("doc.json.bad")).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(dir.path().join("doc.json.bad.1")).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(dir.path().join("doc.json.bad.2")).unwrap(), "third");
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        // A directory where the file should be: reading fails, but not with NotFound.
        std::fs::create_dir(&path).unwrap();
        assert!(load_json::<Doc>(&path).is_err());
        assert!(path.is_dir(), "nothing was renamed");
        assert!(!dir.path().join("doc.json.bad").exists(), "a read failure is not corruption");
    }
}
```

In `crates/livebrainstorm-core/src/lib.rs`, add `pub mod store;` after `pub mod shell_env;`.

In `crates/livebrainstorm-core/src/jev/check.rs`, inside `mod tests`, add after `extracts_bullets_and_skips_nothing_so_far`:

```rust
    #[test]
    fn check_results_round_trip_through_json() {
        let r = CheckResult {
            claim: "`x` is wrong".into(),
            verdict: Verdict::NotShown,
            confidence: Some(0.9),
            path: Some("src/x.rs".into()),
            line: Some(3),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"claim":"`x` is wrong","verdict":"notShown","confidence":0.9,"path":"src/x.rs","line":3}"#);
        assert_eq!(serde_json::from_str::<CheckResult>(&json).unwrap(), r, "saved checks read back unchanged");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core -- store::tests check_results_round_trip_through_json`
Expected: compile error `the trait bound `CheckResult: Deserialize<'_>` is not satisfied` (and, once that compiles, `not implemented` panics in the four `store::tests`).

- [ ] **Step 3: Implement the helpers**

In `crates/livebrainstorm-core/src/store.rs`, replace the three stub functions with:

```rust
/// Atomic write: `<file>.tmp` then rename; creates the parent dir.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Renames a corrupt file to the first free `<name>.bad`, `<name>.bad.1`, …;
/// never replaces an earlier copy.
pub fn set_aside(path: &Path) -> Option<PathBuf> {
    let kept = (0u32..)
        .map(|n| with_suffix(path, &if n == 0 { ".bad".to_owned() } else { format!(".bad.{n}") }))
        .find(|p| !p.exists())?;
    std::fs::rename(path, &kept).ok()?;
    Some(kept)
}

/// Err only for read errors other than NotFound (those files are left in place).
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<Loaded<T>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(match serde_json::from_slice(&bytes) {
            Ok(value) => Loaded::Parsed(value),
            Err(_) => Loaded::Corrupt { kept: set_aside(path) },
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::Missing),
        Err(e) => Err(e.into()),
    }
}

/// `doc.json` + `.tmp` = `doc.json.tmp`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}
```

- [ ] **Step 4: `Deserialize` for checks**

In `crates/livebrainstorm-core/src/jev/check.rs`, change `use serde::Serialize;` to `use serde::{Deserialize, Serialize};`. Change the derive above `pub enum Verdict` to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
```

and the derive above `pub struct CheckResult` to:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
```

- [ ] **Step 5: Move `InstructionStore` onto the helpers**

In `crates/livebrainstorm-core/src/instructions.rs`, add `use crate::store::{self, Loaded};` after `use crate::Result;`. Replace the methods `read`, `set_aside` and `write` (lines 164-195, from `fn read(&self)` to the end of `fn write`) with:

```rust
    fn read(&self) -> Result<InstructionsFile> {
        // A file that doesn't parse is kept as `.bad`, `.bad.1`, … instead of
        // being overwritten by the next save (settings.json just resets; this
        // holds the user's writing). A file that can't be read right now is an
        // error, so `update` never writes over it.
        Ok(match store::load_json(&self.path)? {
            Loaded::Parsed(file) => file,
            Loaded::Missing | Loaded::Corrupt { .. } => InstructionsFile::default(),
        })
    }

    /// Atomic write: never leaves a half-written file behind.
    fn write(&self, file: &InstructionsFile) -> Result<()> {
        store::write_atomic(&self.path, &serde_json::to_vec_pretty(file)?)
    }
```

`load` and `update` stay as they are: `load` still calls `self.read().unwrap_or_default()`, and `update` still calls `self.read()?` then `self.write(&file)`.

- [ ] **Step 6: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core -- store::tests instructions::tests jev::check::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected:
- The four `store::tests` and `check_results_round_trip_through_json` PASS.
- Every `instructions::tests` test still passes unchanged, including `a_corrupt_file_is_kept_aside_not_overwritten`, `a_second_corruption_never_destroys_the_first_kept_copy` and `a_file_that_cannot_be_read_is_left_alone_and_never_overwritten`.
- The whole core suite passes, and clippy prints no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/store.rs crates/livebrainstorm-core/src/lib.rs crates/livebrainstorm-core/src/instructions.rs crates/livebrainstorm-core/src/jev/check.rs
git commit -m "feat(core): shared atomic-write and .bad helpers; instruction store uses them; checks deserialize

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 2: Core: session types, ids and titles

**Files:**
- Create: `crates/livebrainstorm-core/src/sessions/mod.rs`, `crates/livebrainstorm-core/src/sessions/ids.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod sessions;`)

**Interfaces:**
- Consumes (Task 1): `CheckResult` and `Verdict` with `Deserialize`, from `livebrainstorm_core::jev::check`, plus `CheckEvent`:
  ```rust
  pub enum CheckEvent { Checking { count: usize, total: usize }, Result(CheckResult), Notice(String) }
  ```
- Produces (`livebrainstorm_core::sessions`). Every struct is `#[serde(rename_all = "camelCase")]`:
  ```rust
  pub const HISTORY_LEN: usize = 5;
  pub const TITLE_CHARS: usize = 48;

  pub struct Snapshot {            // Debug, Clone, Default, PartialEq, Serialize, Deserialize; serde(default)
      pub id: u64,                 // assigned by the store
      pub at: u64,                 // unix seconds, set by the caller
      pub draft: String,
      pub context: String,
      pub markdown: String,
      pub partial: bool,
      pub notices: Vec<String>,
      pub checks: Vec<CheckResult>,
      pub checks_total: Option<usize>,
      pub checks_faults: Option<usize>,
      pub checks_stopped: bool,
  }
  pub struct Tab {                 // Debug, Clone, Default, PartialEq, Serialize, Deserialize; serde(default)
      pub id: String,
      pub root: Option<String>,
      pub title: Option<String>,
      pub created_at: u64,
      pub updated_at: u64,
      pub closed_at: Option<u64>,
      pub draft: String,
      pub next_snapshot: u64,
      pub history: Vec<Snapshot>,  // newest first, len <= HISTORY_LEN
  }
  pub struct WindowEntry { pub id: String, pub tabs: Vec<String>, pub active: Option<String> }  // + Default; serde(default)
  pub struct Workspace { pub version: u32, pub next_window: u64, pub windows: Vec<WindowEntry> } // Default: version 1, next_window 1, no windows
  pub struct TabSummary {          // Debug, Clone, PartialEq, Serialize
      pub id: String, pub root: Option<String>, pub title: Option<String>,
      pub first_line: String,      // first non-empty draft line, trimmed, at most 48 chars (Unicode scalars)
      pub updated_at: u64, pub closed_at: Option<u64>,
  }
  pub struct WindowView {          // Debug, Clone, Serialize
      pub window: String,
      pub tabs: Vec<TabSummary>,   // this window's tabs, in stored order
      pub active: Option<Tab>,     // full active tab
      pub closed: Vec<TabSummary>, // shared, newest first (closed_at, else updated_at)
      pub fresh: bool,             // workspace.json did not exist when this process first loaded it, and no import has run
      pub unreadable: Vec<String>, // paths kept as .bad during this process
  }
  pub struct LegacyDraft { pub root: Option<String>, pub draft: String }   // Debug, Clone, Deserialize

  pub fn valid_id(id: &str) -> bool;                    // ^[tw]-[0-9a-f-]{1,40}$
  pub fn tab_id(millis: u64, counter: u64) -> String;   // t-<millis hex>-<counter hex>
  pub fn window_id(n: u64) -> String;                   // w-<n>
  pub fn first_line(draft: &str) -> String;             // mirrors firstLine in src/lib/tabTitle.ts
  impl Snapshot { pub fn apply_check(&mut self, ev: &CheckEvent) }
  impl Tab { pub fn summary(&self) -> TabSummary }
  impl Workspace {
      pub fn first() -> Self;                                         // w-1, empty; next_window 2
      pub fn sanitized(self) -> Self;
      pub fn push_window(&mut self) -> String;                        // allocates w-<next_window>
      pub fn window_mut(&mut self, id: &str) -> Option<&mut WindowEntry>;
      pub fn window_of(&self, tab: &str) -> Option<&str>;             // the window that lists `tab`
  }
  impl WindowEntry { pub fn remove_tab(&mut self, tab: &str) }       // active moves to the next tab, else the previous
  ```

- [ ] **Step 1: Write `ids.rs` with its tests, and stub bodies**

Create `crates/livebrainstorm-core/src/sessions/ids.rs`:

```rust
//! Tab and window ids. A tab id becomes a file name (`tabs/<id>.json`), so
//! every id that arrives over IPC passes `valid_id` before it is used: that
//! check is what stops path traversal.

/// `^[tw]-[0-9a-f-]{1,40}$`.
pub fn valid_id(_id: &str) -> bool {
    unimplemented!()
}

/// `t-<unix millis, hex>-<per-process counter, hex>`.
pub fn tab_id(_millis: u64, _counter: u64) -> String {
    unimplemented!()
}

/// `w-<n>`, `n` in decimal (the workspace's `nextWindow`).
pub fn window_id(_n: u64) -> String {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_id_accepts_only_our_ids() {
        let longest = format!("t-{}", "a".repeat(40));
        for ok in ["t-18f2a3b4c5d-1", "w-1", "w-12", "t-0", longest.as_str()] {
            assert!(valid_id(ok), "{ok}");
        }
        let too_long = format!("t-{}", "a".repeat(41));
        let bad = [
            "", "t-", "w-", "x-1", "T-1", "t-A1", "t-1/2", "t-../x", "t-1.json", "t- 1", "tt-1", "t_1", "t-1\n", "w-1;rm",
            too_long.as_str(),
        ];
        for id in bad {
            assert!(!valid_id(id), "{id:?}");
        }
    }

    #[test]
    fn generated_ids_pass_validation() {
        assert_eq!(tab_id(0x18f2a3b4c5d, 1), "t-18f2a3b4c5d-1");
        assert!(valid_id(&tab_id(u64::MAX, u64::MAX)), "even the longest");
        assert_eq!(window_id(3), "w-3");
        assert!(valid_id(&window_id(123)));
    }
}
```

- [ ] **Step 2: Write `mod.rs` with its tests, and stub bodies**

Create `crates/livebrainstorm-core/src/sessions/mod.rs`:

```rust
//! Tabs, windows and their saved analyses, kept under `<app_data>/sessions/`:
//! `workspace.json` lists each window's tabs and its active tab, and
//! `tabs/<id>.json` holds one tab's draft, title and last `HISTORY_LEN`
//! analyses. A closed tab is any tab file that no window lists, so there is
//! no separate closed list that could drift. Live analysis stays stateless:
//! the history is for the user to browse and is never sent to the model.

mod ids;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::jev::check::{CheckEvent, CheckResult};

pub use ids::{tab_id, valid_id, window_id};

/// Analyses kept per tab, newest first.
pub const HISTORY_LEN: usize = 5;
/// The most characters (Unicode scalars) of a draft's first line a tab title shows.
pub const TITLE_CHARS: usize = 48;

/// One analysis, as the notes pane showed it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    /// Assigned by the store from the tab's `next_snapshot`.
    pub id: u64,
    /// Unix seconds.
    pub at: u64,
    /// The draft this analysis read.
    pub draft: String,
    /// The frontend's `analysisContextKey` for the run, opaque here. It lets
    /// the gate keep working after a restart.
    pub context: String,
    pub markdown: String,
    pub partial: bool,
    pub notices: Vec<String>,
    pub checks: Vec<CheckResult>,
    /// The claim count from the last `checking` event. Fewer checks than this,
    /// with `checks_stopped` false, means an edit cut the run off.
    pub checks_total: Option<usize>,
    /// The Faults count from the same event, so restored notes read "2 of 5
    /// faults checked" as they did live.
    pub checks_faults: Option<usize>,
    pub checks_stopped: bool,
}

/// One tab: `tabs/<id>.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tab {
    pub id: String,
    /// `None`: no project; it analyses without code context.
    pub root: Option<String>,
    /// `None`: the sidebar shows the draft's first line.
    pub title: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub closed_at: Option<u64>,
    pub draft: String,
    pub next_snapshot: u64,
    /// Newest first, at most `HISTORY_LEN`.
    pub history: Vec<Snapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowEntry {
    pub id: String,
    pub tabs: Vec<String>,
    pub active: Option<String>,
}

/// `workspace.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub version: u32,
    pub next_window: u64,
    pub windows: Vec<WindowEntry>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self { version: 1, next_window: 1, windows: Vec::new() }
    }
}

/// What lists show; no draft body or history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabSummary {
    pub id: String,
    pub root: Option<String>,
    pub title: Option<String>,
    /// `first_line(draft)`.
    pub first_line: String,
    pub updated_at: u64,
    pub closed_at: Option<u64>,
}

/// One window's tabs, the full active tab, and the shared closed list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    pub window: String,
    /// This window's tabs, in stored order.
    pub tabs: Vec<TabSummary>,
    pub active: Option<Tab>,
    /// Every window's closed tabs, newest first.
    pub closed: Vec<TabSummary>,
    /// `workspace.json` didn't exist when this process first read it, and no
    /// import has run: the frontend imports the old build's drafts.
    pub fresh: bool,
    /// Files kept as `.bad` during this process.
    pub unreadable: Vec<String>,
}

/// A draft the old build kept in `localStorage`, keyed by project root.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDraft {
    pub root: Option<String>,
    pub draft: String,
}

/// The draft's first non-empty line, trimmed, at most `TITLE_CHARS`
/// characters. Mirrors `firstLine` in `src/lib/tabTitle.ts`.
pub fn first_line(_draft: &str) -> String {
    unimplemented!()
}

impl Snapshot {
    /// Applies one fact-check event the way the notes pane does live.
    pub fn apply_check(&mut self, _ev: &CheckEvent) {
        unimplemented!()
    }
}

impl Tab {
    pub fn summary(&self) -> TabSummary {
        unimplemented!()
    }
}

impl Workspace {
    /// A first run: one empty window, `w-1`.
    pub fn first() -> Self {
        unimplemented!()
    }

    /// Drops what a hand-edited or damaged file could hold that would break
    /// the rules: bad ids, a tab listed twice, an active tab the window
    /// doesn't list, and a `next_window` that would reuse an id.
    pub fn sanitized(self) -> Self {
        unimplemented!()
    }

    /// Adds an empty window, `w-<next_window>`, and returns its id.
    pub fn push_window(&mut self) -> String {
        unimplemented!()
    }

    pub fn window_mut(&mut self, _id: &str) -> Option<&mut WindowEntry> {
        unimplemented!()
    }

    /// The window that lists `tab`, if the tab is open.
    pub fn window_of(&self, _tab: &str) -> Option<&str> {
        unimplemented!()
    }
}

impl WindowEntry {
    /// Removes `tab`. If it was active, the tab that took its place (else the
    /// one before it) becomes active.
    pub fn remove_tab(&mut self, _tab: &str) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jev::check::Verdict;

    fn check() -> CheckResult {
        CheckResult {
            claim: "`x` is wrong".into(),
            verdict: Verdict::NotShown,
            confidence: Some(0.9),
            path: Some("src/x.rs".into()),
            line: Some(3),
        }
    }

    #[test]
    fn tabs_serialize_with_the_frontends_field_names() {
        let tab = Tab {
            id: "t-1".into(),
            root: Some("/p".into()),
            title: None,
            created_at: 1,
            updated_at: 2,
            closed_at: None,
            draft: "d".into(),
            next_snapshot: 7,
            history: vec![Snapshot {
                id: 6,
                at: 3,
                draft: "d".into(),
                context: "/p|1|".into(),
                markdown: "## Intent".into(),
                partial: false,
                notices: vec![],
                checks: vec![check()],
                checks_total: Some(4),
                checks_faults: Some(5),
                checks_stopped: false,
            }],
        };
        let json = serde_json::to_value(&tab).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "t-1", "root": "/p", "title": null, "createdAt": 1, "updatedAt": 2, "closedAt": null,
                "draft": "d", "nextSnapshot": 7,
                "history": [{
                    "id": 6, "at": 3, "draft": "d", "context": "/p|1|", "markdown": "## Intent", "partial": false,
                    "notices": [],
                    "checks": [{ "claim": "`x` is wrong", "verdict": "notShown", "confidence": 0.9, "path": "src/x.rs", "line": 3 }],
                    "checksTotal": 4, "checksFaults": 5, "checksStopped": false
                }]
            })
        );
        assert_eq!(serde_json::from_value::<Tab>(json).unwrap(), tab, "and reads back unchanged");
        let summary = serde_json::to_value(tab.summary()).unwrap();
        assert_eq!(summary["firstLine"], "d");
        assert_eq!(summary["closedAt"], serde_json::Value::Null);
    }

    #[test]
    fn a_tab_file_with_missing_fields_still_reads() {
        let tab: Tab = serde_json::from_str(r#"{"id":"t-1","draft":"x"}"#).unwrap();
        assert_eq!((tab.draft.as_str(), tab.next_snapshot, tab.history.len()), ("x", 0, 0));
    }

    #[test]
    fn first_line_matches_the_frontend() {
        // The same strings as `firstLine` in src/lib/tabTitle.test.ts.
        assert_eq!(first_line(""), "");
        assert_eq!(first_line("\n  \n\t Retry failed checkout payments  \nsecond line"), "Retry failed checkout payments");
        assert_eq!(first_line("line one\r\nline two"), "line one");
        assert_eq!(first_line(&"a".repeat(60)), "a".repeat(48));
        assert_eq!(first_line(&format!("{} yz", "x".repeat(47))), "x".repeat(47));
        assert_eq!(first_line(&"😀".repeat(50)), "😀".repeat(48));
        // Unicode White_Space is trimmed (NEL, no-break and ideographic spaces); a BOM is not whitespace.
        assert_eq!(first_line("\u{85}\u{a0}Retry\u{2003}payments\u{3000}\nmore"), "Retry\u{2003}payments");
        assert_eq!(first_line("\u{feff}Retry"), "\u{feff}Retry");
    }

    #[test]
    fn checks_apply_as_the_notes_pane_shows_them() {
        let mut s = Snapshot::default();
        s.apply_check(&CheckEvent::Checking { count: 2, total: 3 });
        s.apply_check(&CheckEvent::Result(check()));
        assert_eq!((s.checks_total, s.checks_faults, s.checks.len(), s.checks_stopped), (Some(2), Some(3), 1, false));
        s.apply_check(&CheckEvent::Notice("Jev is unavailable.".into()));
        assert_eq!(s.notices, ["Jev is unavailable."]);
        assert!(s.checks_stopped, "a failed fact-check is checksStopped, as it is live");
    }

    #[test]
    fn a_damaged_workspace_is_sanitized() {
        let ws: Workspace = serde_json::from_str(
            r#"{"version":1,"nextWindow":1,"windows":[
                {"id":"w-1","tabs":["t-1","t-1","../x","t-2"],"active":"t-9"},
                {"id":"bad","tabs":["t-3"],"active":null},
                {"id":"w-4","tabs":["t-2"],"active":"t-2"},
                {"id":"w-1","tabs":[],"active":null}]}"#,
        )
        .unwrap();
        let ws = ws.sanitized();
        let windows: Vec<(&str, Vec<&str>, Option<&str>)> = ws
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.tabs.iter().map(String::as_str).collect(), w.active.as_deref()))
            .collect();
        assert_eq!(windows, [("w-1", vec!["t-1", "t-2"], Some("t-1")), ("w-4", vec![], None)]);
        assert_eq!(ws.next_window, 5, "a window id is never reused");
    }

    #[test]
    fn new_windows_count_up_from_the_first() {
        let mut ws = Workspace::first();
        assert_eq!(ws.windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["w-1"]);
        assert_eq!(ws.push_window(), "w-2");
        assert_eq!(ws.next_window, 3);
        ws.window_mut("w-2").unwrap().tabs.push("t-a".into());
        assert_eq!(ws.window_of("t-a"), Some("w-2"));
        assert_eq!(ws.window_of("t-b"), None);
        assert!(ws.window_mut("w-9").is_none());
    }

    #[test]
    fn removing_the_active_tab_activates_its_neighbour() {
        let mut w = WindowEntry { id: "w-1".into(), tabs: vec!["t-a".into(), "t-b".into(), "t-c".into()], active: Some("t-b".into()) };
        w.remove_tab("t-b");
        assert_eq!(w.active.as_deref(), Some("t-c"), "the tab that took its place");
        w.remove_tab("t-c");
        assert_eq!(w.active.as_deref(), Some("t-a"), "at the end: the one before it");
        w.tabs.push("t-d".into());
        w.remove_tab("t-d");
        assert_eq!(w.active.as_deref(), Some("t-a"), "closing another tab leaves the active one");
        w.remove_tab("t-e");
        assert_eq!(w.tabs, ["t-a"], "a tab the window doesn't list changes nothing");
        w.remove_tab("t-a");
        assert!(w.tabs.is_empty() && w.active.is_none());
    }
}
```

In `crates/livebrainstorm-core/src/lib.rs`, add `pub mod sessions;` after `pub mod prompt;`. The list stays alphabetical: `prompt`, `sessions`, `settings`, `shell_env`, `store`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core sessions::`
Expected: eight of the nine tests FAIL with `not implemented` panics. `a_tab_file_with_missing_fields_still_reads` already passes, because it only exercises serde.

- [ ] **Step 4: Implement `ids.rs`**

Replace the three stub functions in `crates/livebrainstorm-core/src/sessions/ids.rs` with:

```rust
/// `^[tw]-[0-9a-f-]{1,40}$`.
pub fn valid_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("t-").or_else(|| id.strip_prefix("w-")) else { return false };
    (1..=40).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

/// `t-<unix millis, hex>-<per-process counter, hex>`.
pub fn tab_id(millis: u64, counter: u64) -> String {
    format!("t-{millis:x}-{counter:x}")
}

/// `w-<n>`, `n` in decimal (the workspace's `nextWindow`).
pub fn window_id(n: u64) -> String {
    format!("w-{n}")
}
```

- [ ] **Step 5: Implement the helpers in `mod.rs`**

In `crates/livebrainstorm-core/src/sessions/mod.rs`, replace each stub body. `first_line`:

```rust
pub fn first_line(draft: &str) -> String {
    let line = draft.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().take(TITLE_CHARS).collect::<String>().trim_end().to_owned()
}
```

`Snapshot::apply_check`:

```rust
    pub fn apply_check(&mut self, ev: &CheckEvent) {
        match ev {
            CheckEvent::Checking { count, total } => {
                self.checks_total = Some(*count);
                self.checks_faults = Some(*total);
            }
            CheckEvent::Result(result) => self.checks.push(result.clone()),
            // The fact-check failed: shown as a notice, and the rest were dropped.
            CheckEvent::Notice(message) => {
                self.notices.push(message.clone());
                self.checks_stopped = true;
            }
        }
    }
```

`Tab::summary`:

```rust
    pub fn summary(&self) -> TabSummary {
        TabSummary {
            id: self.id.clone(),
            root: self.root.clone(),
            title: self.title.clone(),
            first_line: first_line(&self.draft),
            updated_at: self.updated_at,
            closed_at: self.closed_at,
        }
    }
```

The `Workspace` methods:

```rust
    pub fn first() -> Self {
        let mut ws = Self::default();
        ws.push_window();
        ws
    }

    pub fn sanitized(mut self) -> Self {
        let mut seen = HashSet::new();
        let mut listed = HashSet::new();
        self.windows.retain(|w| w.id.starts_with("w-") && valid_id(&w.id) && seen.insert(w.id.clone()));
        for w in &mut self.windows {
            w.tabs.retain(|t| t.starts_with("t-") && valid_id(t) && listed.insert(t.clone()));
            if !w.active.as_ref().is_some_and(|a| w.tabs.contains(a)) {
                w.active = w.tabs.first().cloned();
            }
        }
        let highest = self.windows.iter().filter_map(|w| w.id.strip_prefix("w-")?.parse::<u64>().ok()).max().unwrap_or(0);
        self.next_window = self.next_window.max(highest + 1);
        self
    }

    pub fn push_window(&mut self) -> String {
        let id = window_id(self.next_window);
        self.next_window += 1;
        self.windows.push(WindowEntry { id: id.clone(), tabs: Vec::new(), active: None });
        id
    }

    pub fn window_mut(&mut self, id: &str) -> Option<&mut WindowEntry> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn window_of(&self, tab: &str) -> Option<&str> {
        self.windows.iter().find(|w| w.tabs.iter().any(|t| t == tab)).map(|w| w.id.as_str())
    }
```

`WindowEntry::remove_tab`:

```rust
    pub fn remove_tab(&mut self, tab: &str) {
        let Some(pos) = self.tabs.iter().position(|t| t == tab) else { return };
        self.tabs.remove(pos);
        if self.active.as_deref() == Some(tab) {
            self.active = self.tabs.get(pos).or_else(|| self.tabs.last()).cloned();
        }
    }
```

- [ ] **Step 6: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core sessions:: && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: the nine `sessions` tests PASS, the whole core suite passes, and clippy prints no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/sessions crates/livebrainstorm-core/src/lib.rs
git commit -m "feat(core): session types, id validation and tab titles

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 3: Core: `SessionStore`: windows, tabs, drafts and history

**Files:**
- Create: `crates/livebrainstorm-core/src/sessions/store.rs`
- Modify: `crates/livebrainstorm-core/src/sessions/mod.rs` (`mod store;`, `pub use store::SessionStore;`)

**Interfaces:**
- Consumes:
  - Task 1 (`livebrainstorm_core::store`):
    ```rust
    pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()>;
    pub enum Loaded<T> { Missing, Parsed(T), Corrupt { kept: Option<PathBuf> } }
    pub fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Loaded<T>>;
    ```
  - Task 2 (`super::`, in `sessions/mod.rs` and `sessions/ids.rs`; every struct is `#[serde(rename_all = "camelCase")]`):
    ```rust
    pub const HISTORY_LEN: usize = 5;
    pub struct Snapshot { pub id: u64, pub at: u64, pub draft: String, pub context: String, pub markdown: String,
                          pub partial: bool, pub notices: Vec<String>, pub checks: Vec<CheckResult>,
                          pub checks_total: Option<usize>, pub checks_faults: Option<usize>,
                          pub checks_stopped: bool }                                          // + Default; serde(default)
    pub struct Tab { pub id: String, pub root: Option<String>, pub title: Option<String>, pub created_at: u64,
                     pub updated_at: u64, pub closed_at: Option<u64>, pub draft: String, pub next_snapshot: u64,
                     pub history: Vec<Snapshot> }                                       // + Default; serde(default)
    pub struct TabSummary { pub id: String, pub root: Option<String>, pub title: Option<String>, pub first_line: String,
                            pub updated_at: u64, pub closed_at: Option<u64> }
    pub struct WindowEntry { pub id: String, pub tabs: Vec<String>, pub active: Option<String> }
    pub struct Workspace { pub version: u32, pub next_window: u64, pub windows: Vec<WindowEntry> }  // Default: version 1, next_window 1, no windows
    pub struct WindowView { pub window: String, pub tabs: Vec<TabSummary>, pub active: Option<Tab>,
                            pub closed: Vec<TabSummary>, pub fresh: bool, pub unreadable: Vec<String> }
    pub fn valid_id(id: &str) -> bool;                  // ^[tw]-[0-9a-f-]{1,40}$
    pub fn tab_id(millis: u64, counter: u64) -> String; // t-<millis hex>-<counter hex>
    impl Tab { pub fn summary(&self) -> TabSummary }
    impl Workspace {
        pub fn first() -> Self;                                     // w-1, empty; next_window 2
        pub fn sanitized(self) -> Self;                             // drops bad or repeated ids; fixes `active` and `next_window`
        pub fn push_window(&mut self) -> String;                    // allocates w-<next_window>
        pub fn window_mut(&mut self, id: &str) -> Option<&mut WindowEntry>;
        pub fn window_of(&self, tab: &str) -> Option<&str>;         // the window that lists `tab`
    }
    ```
  - `crate::Error::Precondition(String)` and `crate::Result<T>` (`error.rs`, unchanged).
- Produces (`livebrainstorm_core::sessions::SessionStore`). Every id argument is checked with `valid_id` first; a failure is `Error::Precondition("Unknown tab.")`.
  ```rust
  impl SessionStore {
      pub fn new(dir: PathBuf) -> Self;   // dir = <app_data>/sessions
      pub(crate) fn with_clock(dir: PathBuf, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self;  // unix millis; for tests
      pub fn window_ids(&self) -> Vec<String>;                       // creates w-1 if no workspace
      pub fn window_view(&self, window: &str) -> Result<WindowView>; // Err(Precondition("This window was closed.")) for an unknown window
      pub fn add_window(&self) -> Result<String>;                     // allocates w-<next_window>
      pub fn window_of(&self, id: &str) -> Option<String>;            // extra: the window that lists an open tab
      pub fn create_tab(&self, window: &str, root: Option<String>) -> Result<Tab>;  // appended, made active
      pub fn tab(&self, id: &str) -> Result<Tab>;                     // any tab file, open or closed; missing: "This tab was closed."
      pub fn open_tab_root(&self, id: &str) -> Result<Option<String>>;  // Err(Precondition("This tab was closed.")) if closed/unknown
      pub fn save_draft(&self, id: &str, draft: String) -> Result<bool>;  // open or closed tabs; true when the first line changed
      pub fn rename_tab(&self, id: &str, title: Option<String>) -> Result<()>;  // None or blank: automatic title
      pub fn record_snapshot(&self, id: &str, snap: Snapshot) -> Result<u64>;
      pub fn update_snapshot(&self, id: &str, snapshot: u64, f: impl FnOnce(&mut Snapshot)) -> Result<()>;
      pub fn activate(&self, window: &str, id: &str) -> Result<()>;
      pub fn closed(&self) -> Vec<TabSummary>;
  }
  ```

- [ ] **Step 1: Write the file with its tests, and stub bodies**

Create `crates/livebrainstorm-core/src/sessions/store.rs`:

```rust
//! `SessionStore`: `workspace.json` and `tabs/<id>.json` under
//! `<app_data>/sessions/`. One `std` mutex guards the workspace and a cache of
//! the tabs read so far. It is held only for synchronous file I/O, never
//! across `.await`. The store is the only writer, so the cache stays current.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::{first_line, tab_id, valid_id, Snapshot, Tab, TabSummary, WindowView, Workspace, HISTORY_LEN};
use crate::store::{load_json, write_atomic, Loaded};
use crate::{Error, Result};

pub struct SessionStore {
    dir: PathBuf,
    inner: Mutex<Inner>,
    /// Unix milliseconds. Injectable, so tests can order timestamps.
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

#[derive(Default)]
struct Inner {
    /// Read on first use; `None` until then, or after a read error.
    workspace: Option<Workspace>,
    /// Every tab read so far, by id.
    tabs: HashMap<String, Tab>,
    /// `workspace.json` didn't exist when this process first read it, and no import has run.
    fresh: bool,
    /// Files kept as `.bad` during this process.
    unreadable: Vec<String>,
    /// The per-process part of new tab ids.
    counter: u64,
}

impl SessionStore {
    /// `dir` is `<app_data>/sessions`. Nothing is read until first use.
    pub fn new(dir: PathBuf) -> Self {
        Self::with_clock(dir, || SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))
    }

    /// `clock` returns unix milliseconds.
    pub(crate) fn with_clock(dir: PathBuf, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self { dir, inner: Mutex::new(Inner::default()), clock: Box::new(clock) }
    }

    pub fn window_ids(&self) -> Vec<String> {
        unimplemented!()
    }

    pub fn window_view(&self, _window: &str) -> Result<WindowView> {
        unimplemented!()
    }

    pub fn add_window(&self) -> Result<String> {
        unimplemented!()
    }

    pub fn window_of(&self, _id: &str) -> Option<String> {
        unimplemented!()
    }

    pub fn create_tab(&self, _window: &str, _root: Option<String>) -> Result<Tab> {
        unimplemented!()
    }

    pub fn tab(&self, _id: &str) -> Result<Tab> {
        unimplemented!()
    }

    pub fn open_tab_root(&self, _id: &str) -> Result<Option<String>> {
        unimplemented!()
    }

    pub fn save_draft(&self, _id: &str, _draft: String) -> Result<bool> {
        unimplemented!()
    }

    pub fn rename_tab(&self, _id: &str, _title: Option<String>) -> Result<()> {
        unimplemented!()
    }

    pub fn activate(&self, _window: &str, _id: &str) -> Result<()> {
        unimplemented!()
    }

    pub fn closed(&self) -> Vec<TabSummary> {
        unimplemented!()
    }

    pub fn record_snapshot(&self, _id: &str, _snap: Snapshot) -> Result<u64> {
        unimplemented!()
    }

    pub fn update_snapshot(&self, _id: &str, _snapshot: u64, _f: impl FnOnce(&mut Snapshot)) -> Result<()> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A store whose clock moves one second per reading, so timestamps order.
    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = at(&dir);
        (dir, store)
    }

    /// A new store on the same folder: what the next launch reads.
    fn at(dir: &tempfile::TempDir) -> SessionStore {
        let ms = AtomicU64::new(1_760_400_000_000);
        SessionStore::with_clock(dir.path().join("sessions"), move || ms.fetch_add(1_000, Ordering::Relaxed))
    }

    fn sessions(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("sessions")
    }

    fn snap(draft: &str) -> Snapshot {
        Snapshot { draft: draft.into(), context: "/p|1|".into(), markdown: "## Intent\n- x".into(), ..Snapshot::default() }
    }

    fn ids(list: &[TabSummary]) -> Vec<&str> {
        list.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn a_first_run_has_one_empty_window_and_is_fresh() {
        let (dir, s) = store();
        assert_eq!(s.window_ids(), ["w-1"]);
        let view = s.window_view("w-1").unwrap();
        assert!(view.fresh);
        assert!(view.tabs.is_empty() && view.active.is_none() && view.closed.is_empty() && view.unreadable.is_empty());
        assert!(!sessions(&dir).join("workspace.json").exists(), "nothing is written until something changes");
    }

    #[test]
    fn tabs_are_created_saved_and_renamed() {
        let (dir, s) = store();
        let tab = s.create_tab("w-1", Some("/p".into())).unwrap();
        assert!(tab.id.starts_with("t-") && valid_id(&tab.id), "{}", tab.id);
        assert_eq!((tab.root.as_deref(), tab.draft.as_str(), tab.next_snapshot), (Some("/p"), "", 1));
        assert!(s.save_draft(&tab.id, "Retry failed payments".into()).unwrap(), "a new first line changes the lists");
        assert!(!s.save_draft(&tab.id, "Retry failed payments\nwith backoff".into()).unwrap(), "a later line doesn't");
        s.rename_tab(&tab.id, Some("  Checkout retry ".into())).unwrap();
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [tab.id.as_str()]);
        assert_eq!(view.tabs[0].title.as_deref(), Some("Checkout retry"));
        assert_eq!(view.tabs[0].first_line, "Retry failed payments");
        assert!(view.tabs[0].updated_at > tab.updated_at, "saving a draft moves updatedAt");
        assert_eq!(view.active.map(|t| t.draft), Some("Retry failed payments\nwith backoff".to_owned()));
        s.rename_tab(&tab.id, Some("   ".into())).unwrap();
        assert_eq!(s.tab(&tab.id).unwrap().title, None, "a blank name returns to the automatic title");
        s.rename_tab(&tab.id, Some("Checkout retry".into())).unwrap();
        s.rename_tab(&tab.id, None).unwrap();
        // The next launch reads the same files back.
        let again = at(&dir);
        let read = again.tab(&tab.id).unwrap();
        assert_eq!((read.title, read.draft.as_str()), (None, "Retry failed payments\nwith backoff"));
        assert_eq!(again.open_tab_root(&tab.id).unwrap().as_deref(), Some("/p"));
        assert!(!again.window_view("w-1").unwrap().fresh, "the workspace file exists now");
    }

    #[test]
    fn a_new_tab_goes_last_and_becomes_active() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        let b = s.create_tab("w-1", None).unwrap();
        assert_ne!(a.id, b.id);
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str(), b.id.as_str()]);
        assert_eq!(view.active.map(|t| t.id), Some(b.id.clone()));
        s.activate("w-1", &a.id).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().active.map(|t| t.id), Some(a.id.clone()));
        assert_eq!(s.window_of(&a.id).as_deref(), Some("w-1"));
        assert_eq!(s.open_tab_root(&a.id).unwrap(), None, "a tab with no project");
        let w2 = s.add_window().unwrap();
        assert_eq!(s.activate(&w2, &a.id).unwrap_err().to_string(), "That tab is already open in another window.");
        assert_eq!(s.open_tab_root("t-dead").unwrap_err().to_string(), "This tab was closed.");
        assert_eq!(s.activate("w-1", "t-dead").unwrap_err().to_string(), "This tab was closed.");
    }

    #[test]
    fn windows_get_new_ids_and_unknown_ones_are_refused() {
        let (_dir, s) = store();
        assert_eq!(s.add_window().unwrap(), "w-2");
        assert_eq!(s.window_ids(), ["w-1", "w-2"]);
        assert_eq!(s.window_view("w-9").unwrap_err().to_string(), "This window was closed.");
        assert_eq!(s.create_tab("w-9", None).unwrap_err().to_string(), "This window was closed.");
        assert!(s.closed().is_empty(), "no tab file was written for it");
    }

    #[test]
    fn ids_that_fail_validation_never_reach_the_disk() {
        let (dir, s) = store();
        std::fs::write(dir.path().join("secret.json"), r#"{"draft":"private"}"#).unwrap();
        let too_long = format!("t-{}", "a".repeat(41));
        for bad in ["t-../secret", "t-../../secret", "t-1/2", "T-1", "t-", "x-1", "w-1;rm", too_long.as_str()] {
            assert_eq!(s.tab(bad).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.save_draft(bad, "x".into()).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.record_snapshot(bad, snap("x")).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.window_view(bad).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.create_tab(bad, None).unwrap_err().to_string(), "Unknown tab.", "{bad}");
        }
        assert_eq!(std::fs::read_to_string(dir.path().join("secret.json")).unwrap(), r#"{"draft":"private"}"#);
        assert!(!sessions(&dir).exists(), "nothing was written");
    }

    #[test]
    fn history_keeps_the_newest_five() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        let saved: Vec<u64> = (0..7).map(|i| s.record_snapshot(&tab.id, snap(&format!("draft {i}"))).unwrap()).collect();
        assert_eq!(saved, [1, 2, 3, 4, 5, 6, 7]);
        let t = s.tab(&tab.id).unwrap();
        assert_eq!(t.history.iter().map(|h| h.id).collect::<Vec<_>>(), [7, 6, 5, 4, 3]);
        assert_eq!(t.history[0].draft, "draft 6");
        assert_eq!(t.next_snapshot, 8);
    }

    #[test]
    fn update_snapshot_finds_its_id_and_ignores_trimmed_ones() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        let first = s.record_snapshot(&tab.id, snap("one")).unwrap();
        s.record_snapshot(&tab.id, snap("two")).unwrap();
        s.update_snapshot(&tab.id, first, |h| h.checks_stopped = true).unwrap();
        let t = s.tab(&tab.id).unwrap();
        assert_eq!((t.history[0].checks_stopped, t.history[1].checks_stopped), (false, true));
        for i in 0..HISTORY_LEN {
            s.record_snapshot(&tab.id, snap(&format!("more {i}"))).unwrap();
        }
        s.update_snapshot(&tab.id, first, |h| h.markdown = "changed".into()).unwrap(); // trimmed: nothing to update
        assert!(s.tab(&tab.id).unwrap().history.iter().all(|h| h.markdown != "changed"));
    }

    #[test]
    fn a_corrupt_workspace_is_kept_and_its_tabs_show_as_closed() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", Some("/p".into())).unwrap();
        drop(s);
        std::fs::write(sessions(&dir).join("workspace.json"), "{ not json").unwrap();
        std::fs::write(sessions(&dir).join("workspace.json.bad"), "an older copy").unwrap();
        let s = at(&dir);
        let view = s.window_view("w-1").unwrap();
        assert!(view.tabs.is_empty() && !view.fresh, "a damaged file is not a first run");
        assert_eq!(ids(&view.closed), [a.id.as_str()]);
        assert_eq!(view.unreadable.len(), 1);
        assert!(view.unreadable[0].ends_with("workspace.json.bad.1"), "{:?}", view.unreadable);
        assert_eq!(std::fs::read_to_string(sessions(&dir).join("workspace.json.bad")).unwrap(), "an older copy");
        assert_eq!(std::fs::read_to_string(sessions(&dir).join("workspace.json.bad.1")).unwrap(), "{ not json");
        assert!(!at(&dir).window_view("w-1").unwrap().fresh, "an empty workspace was saved in its place right away");
    }

    #[test]
    fn a_lost_workspace_shows_every_tab_as_closed() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        let b = s.create_tab("w-1", None).unwrap();
        drop(s);
        std::fs::remove_file(sessions(&dir).join("workspace.json")).unwrap();
        let view = at(&dir).window_view("w-1").unwrap();
        assert!(view.tabs.is_empty());
        let mut closed = ids(&view.closed);
        closed.sort_unstable();
        let mut both = vec![a.id.as_str(), b.id.as_str()];
        both.sort_unstable();
        assert_eq!(closed, both);
    }

    #[test]
    fn a_corrupt_tab_is_kept_and_skipped() {
        let (dir, s) = store();
        let good = s.create_tab("w-1", None).unwrap();
        let bad = s.create_tab("w-1", None).unwrap(); // active
        drop(s);
        let tabs = sessions(&dir).join("tabs");
        std::fs::write(tabs.join(format!("{}.json", bad.id)), "garbage").unwrap();
        let s = at(&dir);
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [good.id.as_str()]);
        assert!(view.active.is_none(), "the active tab was the damaged one");
        assert_eq!(view.unreadable.len(), 1);
        assert!(view.unreadable[0].ends_with(&format!("{}.json.bad", bad.id)), "{:?}", view.unreadable);
        assert_eq!(std::fs::read_to_string(tabs.join(format!("{}.json.bad", bad.id))).unwrap(), "garbage");
        assert_eq!(s.tab(&bad.id).unwrap_err().to_string(), "This tab was closed.");
        assert_eq!(s.tab(&good.id).unwrap().id, good.id, "the other tabs are fine");
    }

    #[test]
    fn a_tab_file_that_cannot_be_read_is_reported_and_left_alone() {
        let (dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        drop(s);
        // A directory where the file should be: reading fails, but not with NotFound.
        let path = sessions(&dir).join(format!("tabs/{}.json", tab.id));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let s = at(&dir);
        assert!(s.tab(&tab.id).is_err());
        assert!(s.save_draft(&tab.id, "x".into()).is_err());
        let view = s.window_view("w-1").unwrap();
        assert!(view.tabs.is_empty() && view.unreadable.is_empty(), "left out of the list, and not counted as corrupt");
        assert!(path.is_dir(), "nothing was renamed or replaced");
    }

    #[test]
    fn a_workspace_that_cannot_be_read_is_never_overwritten() {
        let (dir, s) = store();
        let ws = sessions(&dir).join("workspace.json");
        std::fs::create_dir_all(&ws).unwrap();
        assert!(s.window_view("w-1").is_err());
        assert!(s.create_tab("w-1", None).is_err());
        assert!(s.window_ids().is_empty());
        assert!(ws.is_dir(), "nothing was renamed or replaced");
    }

    #[test]
    fn concurrent_changes_never_lose_an_update() {
        let (_dir, s) = store();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    s.create_tab("w-1", None).unwrap();
                });
            }
        });
        assert_eq!(s.window_view("w-1").unwrap().tabs.len(), 8);
    }
}
```

In `crates/livebrainstorm-core/src/sessions/mod.rs`, add `mod store;` after `mod ids;`, and `pub use store::SessionStore;` after the `pub use ids::…;` line.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core sessions::store::`
Expected: the 13 tests FAIL with `not implemented` panics. Warnings about unused imports are expected until Step 3.

- [ ] **Step 3: Implement the store**

In `crates/livebrainstorm-core/src/sessions/store.rs`, replace the whole `impl SessionStore { … }` block (from `impl SessionStore {` down to its closing `}`, just above `#[cfg(test)]`) with:

```rust
impl SessionStore {
    /// `dir` is `<app_data>/sessions`. Nothing is read until first use.
    pub fn new(dir: PathBuf) -> Self {
        Self::with_clock(dir, || SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))
    }

    /// `clock` returns unix milliseconds.
    pub(crate) fn with_clock(dir: PathBuf, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self { dir, inner: Mutex::new(Inner::default()), clock: Box::new(clock) }
    }

    // ---------- windows ----------

    /// Every window, in order. A first run has one empty window, `w-1`, saved
    /// with the first change. A workspace that can't be read gives none.
    pub fn window_ids(&self) -> Vec<String> {
        let mut inner = self.lock();
        match self.read_ws(&mut inner) {
            Ok(ws) => ws.windows.into_iter().map(|w| w.id).collect(),
            Err(e) => {
                eprintln!("LiveBrainstorm: couldn't read the saved windows: {e}");
                Vec::new()
            }
        }
    }

    /// One window's tabs, its full active tab, and every window's closed tabs.
    /// A tab file that can't be read is left out of the lists.
    pub fn window_view(&self, window: &str) -> Result<WindowView> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let ws = self.read_ws(inner)?;
        let entry = ws.windows.iter().find(|w| w.id == window).ok_or_else(window_closed)?;
        let mut tabs = Vec::new();
        for id in &entry.tabs {
            if let Some(tab) = self.read_tab_logged(inner, id) {
                tabs.push(tab.summary());
            }
        }
        let active = match &entry.active {
            Some(id) => self.read_tab_logged(inner, id),
            None => None,
        };
        let closed = self.closed_in(inner, &ws);
        Ok(WindowView {
            window: window.to_owned(),
            tabs,
            active,
            closed,
            fresh: inner.fresh,
            unreadable: inner.unreadable.clone(),
        })
    }

    /// A new, empty window, `w-<next_window>`.
    pub fn add_window(&self) -> Result<String> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let id = ws.push_window();
        self.put_ws(inner, ws)?;
        Ok(id)
    }

    /// The window that lists `id`, if the tab is open.
    pub fn window_of(&self, id: &str) -> Option<String> {
        if !valid_id(id) {
            return None;
        }
        let mut guard = self.lock();
        let ws = self.read_ws(&mut guard).ok()?;
        ws.window_of(id).map(str::to_owned)
    }

    // ---------- tabs ----------

    /// A new, empty tab at the end of `window`, made active.
    pub fn create_tab(&self, window: &str, root: Option<String>) -> Result<Tab> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let tab = self.new_tab(inner, root, String::new());
        // The file first: if the app stops before the workspace is saved, the
        // tab shows as closed rather than as a listed id with no file.
        self.put_tab(inner, tab.clone())?;
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        entry.tabs.push(tab.id.clone());
        entry.active = Some(tab.id.clone());
        self.put_ws(inner, ws)?;
        Ok(tab)
    }

    /// Any tab file, open or closed.
    pub fn tab(&self, id: &str) -> Result<Tab> {
        validate(id)?;
        let mut guard = self.lock();
        self.existing_tab(&mut guard, id)
    }

    /// The tab's project, if a window lists the tab; "This tab was closed." otherwise.
    pub fn open_tab_root(&self, id: &str) -> Result<Option<String>> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        if self.read_ws(inner)?.window_of(id).is_none() {
            return Err(tab_closed());
        }
        Ok(self.existing_tab(inner, id)?.root)
    }

    /// Open or closed: the webview's pending save can land just after its tab closed.
    /// Returns whether the first line changed: it's the only part of a draft any list shows.
    pub fn save_draft(&self, id: &str, draft: String) -> Result<bool> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        if tab.draft == draft {
            return Ok(false);
        }
        let retitled = first_line(&tab.draft) != first_line(&draft);
        tab.draft = draft;
        tab.updated_at = self.now_secs();
        self.put_tab(inner, tab)?;
        Ok(retitled)
    }

    /// `None`, or a blank title, returns the tab to its automatic title.
    pub fn rename_tab(&self, id: &str, title: Option<String>) -> Result<()> {
        validate(id)?;
        let title = title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        if tab.title == title {
            return Ok(());
        }
        tab.title = title;
        self.put_tab(inner, tab)
    }

    /// Shows `id` in `window`. A window shows only the tabs it lists.
    pub fn activate(&self, window: &str, id: &str) -> Result<()> {
        validate(window)?;
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let elsewhere = ws.window_of(id).is_some_and(|w| w != window);
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        if !entry.tabs.iter().any(|t| t == id) {
            return Err(if elsewhere { open_elsewhere() } else { tab_closed() });
        }
        if entry.active.as_deref() == Some(id) {
            return Ok(());
        }
        entry.active = Some(id.to_owned());
        self.put_ws(inner, ws)
    }

    /// Every tab file no window lists, newest first.
    pub fn closed(&self) -> Vec<TabSummary> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        match self.read_ws(inner) {
            Ok(ws) => self.closed_in(inner, &ws),
            Err(_) => Vec::new(), // which tabs are open is unknown
        }
    }

    // ---------- history ----------

    /// Saves `snap` as the tab's newest analysis and keeps `HISTORY_LEN` of
    /// them. Returns the id it was given.
    pub fn record_snapshot(&self, id: &str, mut snap: Snapshot) -> Result<u64> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        // Never reuse an id still in the history, even if the file lost its counter.
        let next = tab.history.iter().map(|h| h.id + 1).max().unwrap_or(1).max(tab.next_snapshot);
        snap.id = next;
        tab.next_snapshot = next + 1;
        tab.history.insert(0, snap);
        tab.history.truncate(HISTORY_LEN);
        self.put_tab(inner, tab)?;
        Ok(next)
    }

    /// Changes one saved analysis; a no-op once it has been trimmed away.
    pub fn update_snapshot(&self, id: &str, snapshot: u64, f: impl FnOnce(&mut Snapshot)) -> Result<()> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        let Some(saved) = tab.history.iter_mut().find(|h| h.id == snapshot) else { return Ok(()) };
        f(saved);
        self.put_tab(inner, tab)
    }

    // ---------- files ----------

    fn now_secs(&self) -> u64 {
        (self.clock)() / 1000
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("sessions lock")
    }

    fn workspace_path(&self) -> PathBuf {
        self.dir.join("workspace.json")
    }

    fn tab_path(&self, id: &str) -> PathBuf {
        self.dir.join("tabs").join(format!("{id}.json"))
    }

    /// The workspace, read on first use. A missing file is a first run: one
    /// empty window, saved with the first change. A file that doesn't parse is
    /// kept as `.bad` and replaced right away by one empty window, so its tabs
    /// show under Recently closed. A read error is returned and nothing is written.
    fn read_ws(&self, inner: &mut Inner) -> Result<Workspace> {
        if let Some(ws) = &inner.workspace {
            return Ok(ws.clone());
        }
        let path = self.workspace_path();
        let ws = match load_json::<Workspace>(&path)? {
            Loaded::Parsed(ws) => ws.sanitized(),
            Loaded::Missing => {
                inner.fresh = true;
                Workspace::first()
            }
            Loaded::Corrupt { kept: Some(kept) } => {
                inner.unreadable.push(kept.display().to_string());
                let ws = Workspace::first();
                if let Err(e) = write_json(&path, &ws) {
                    eprintln!("LiveBrainstorm: couldn't save a new workspace: {e}");
                }
                ws
            }
            Loaded::Corrupt { kept: None } => return Err(not_kept(&path)),
        };
        inner.workspace = Some(ws.clone());
        Ok(ws)
    }

    fn put_ws(&self, inner: &mut Inner, ws: Workspace) -> Result<()> {
        write_json(&self.workspace_path(), &ws)?;
        inner.workspace = Some(ws);
        Ok(())
    }

    /// A tab from the cache or its file. `None` when there's no such file, or
    /// it didn't parse (it's kept as `.bad` and skipped).
    fn read_tab(&self, inner: &mut Inner, id: &str) -> Result<Option<Tab>> {
        if let Some(tab) = inner.tabs.get(id) {
            return Ok(Some(tab.clone()));
        }
        let path = self.tab_path(id);
        match load_json::<Tab>(&path)? {
            Loaded::Parsed(mut tab) => {
                tab.id = id.to_owned(); // the file name is the id
                inner.tabs.insert(tab.id.clone(), tab.clone());
                Ok(Some(tab))
            }
            Loaded::Missing => Ok(None),
            Loaded::Corrupt { kept: Some(kept) } => {
                inner.unreadable.push(kept.display().to_string());
                Ok(None)
            }
            Loaded::Corrupt { kept: None } => Err(not_kept(&path)),
        }
    }

    /// For lists: a tab that can't be read is left out, never fatal to the list.
    fn read_tab_logged(&self, inner: &mut Inner, id: &str) -> Option<Tab> {
        self.read_tab(inner, id).unwrap_or_else(|e| {
            eprintln!("LiveBrainstorm: couldn't read tab {id}: {e}");
            None
        })
    }

    fn existing_tab(&self, inner: &mut Inner, id: &str) -> Result<Tab> {
        self.read_tab(inner, id)?.ok_or_else(tab_closed)
    }

    fn put_tab(&self, inner: &mut Inner, tab: Tab) -> Result<()> {
        write_json(&self.tab_path(&tab.id), &tab)?;
        inner.tabs.insert(tab.id.clone(), tab);
        Ok(())
    }

    /// A tab with a fresh id: `t-<millis hex>-<counter hex>`, never one in use.
    fn new_tab(&self, inner: &mut Inner, root: Option<String>, draft: String) -> Tab {
        let now = (self.clock)();
        let id = loop {
            inner.counter += 1;
            let id = tab_id(now, inner.counter);
            if !inner.tabs.contains_key(&id) && !self.tab_path(&id).exists() {
                break id;
            }
        };
        Tab { id, root, created_at: now / 1000, updated_at: now / 1000, draft, next_snapshot: 1, ..Tab::default() }
    }

    /// Every tab file no window lists, newest first: by `closed_at`, or, for a
    /// tab whose close never finished writing, by `updated_at`.
    fn closed_in(&self, inner: &mut Inner, ws: &Workspace) -> Vec<TabSummary> {
        let listed: HashSet<&str> = ws.windows.iter().flat_map(|w| w.tabs.iter().map(String::as_str)).collect();
        let mut closed = Vec::new();
        for id in self.tab_ids_on_disk() {
            if listed.contains(id.as_str()) {
                continue;
            }
            if let Some(tab) = self.read_tab_logged(inner, &id) {
                closed.push(tab.summary());
            }
        }
        closed.sort_by(|a, b| newest_first(b).cmp(&newest_first(a)).then_with(|| b.id.cmp(&a.id)));
        closed
    }

    /// Tab ids from `tabs/*.json`. `.tmp`, `.bad` and other names are skipped.
    fn tab_ids_on_disk(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.dir.join("tabs")) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                (id.starts_with("t-") && valid_id(id)).then(|| id.to_owned())
            })
            .collect()
    }
}

fn validate(id: &str) -> Result<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(Error::Precondition("Unknown tab.".into()))
    }
}

fn tab_closed() -> Error {
    Error::Precondition("This tab was closed.".into())
}

fn window_closed() -> Error {
    Error::Precondition("This window was closed.".into())
}

fn open_elsewhere() -> Error {
    Error::Precondition("That tab is already open in another window.".into())
}

/// A file that didn't parse and couldn't be moved aside: never written over.
fn not_kept(path: &Path) -> Error {
    std::io::Error::other(format!("couldn't keep a copy of the damaged {}", path.display())).into()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

/// Recently closed order: when it was closed, else when it was last edited.
fn newest_first(t: &TabSummary) -> u64 {
    t.closed_at.unwrap_or(t.updated_at)
}
```

- [ ] **Step 4: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core sessions:: && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: the 13 `sessions::store` tests and the nine from Task 2 PASS, the whole core suite passes, and clippy prints no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/sessions
git commit -m "feat(core): SessionStore: windows, tabs, drafts and history on disk; damaged files kept as .bad

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 4: Core: closing, reopening, deleting and moving tabs; the legacy import; `Recorder`

**Files:**
- Modify: `crates/livebrainstorm-core/src/sessions/store.rs` (new methods in `impl SessionStore`, `Recorder`, tests)
- Modify: `crates/livebrainstorm-core/src/sessions/mod.rs` (re-export `Recorder`)

**Interfaces:**
- Consumes:
  - Task 3: `SessionStore` and its private items in the same file (`sessions/store.rs`):
    ```rust
    struct Inner { workspace: Option<Workspace>, tabs: HashMap<String, Tab>, fresh: bool, unreadable: Vec<String>, counter: u64 }
    impl SessionStore {
        fn lock(&self) -> MutexGuard<'_, Inner>;
        fn read_ws(&self, inner: &mut Inner) -> Result<Workspace>;                 // cached clone; first use reads the file
        fn put_ws(&self, inner: &mut Inner, ws: Workspace) -> Result<()>;          // atomic write, then cache
        fn existing_tab(&self, inner: &mut Inner, id: &str) -> Result<Tab>;        // no such file: Err(tab_closed())
        fn read_tab_logged(&self, inner: &mut Inner, id: &str) -> Option<Tab>;     // a read error is logged and gives None
        fn put_tab(&self, inner: &mut Inner, tab: Tab) -> Result<()>;              // atomic write, then cache
        fn new_tab(&self, inner: &mut Inner, root: Option<String>, draft: String) -> Tab;  // fresh id; not saved yet
        fn now_secs(&self) -> u64;
        fn tab_path(&self, id: &str) -> PathBuf;                                   // <dir>/tabs/<id>.json
    }
    fn validate(id: &str) -> Result<()>;   // Err(Precondition("Unknown tab.")) unless valid_id
    fn tab_closed() -> Error;              // Precondition("This tab was closed.")
    fn window_closed() -> Error;           // Precondition("This window was closed.")
    fn open_elsewhere() -> Error;          // Precondition("That tab is already open in another window.")
    ```
  - Task 3's test helpers in `mod tests`, which the new tests use: `fn store() -> (tempfile::TempDir, SessionStore)` (a clock that moves one second per reading), `fn at(dir: &tempfile::TempDir) -> SessionStore` (the next launch on the same folder), `fn sessions(dir: &tempfile::TempDir) -> PathBuf`, `fn snap(draft: &str) -> Snapshot` and `fn ids(list: &[TabSummary]) -> Vec<&str>`.
  - Task 2 (`super::`):
    ```rust
    pub struct LegacyDraft { pub root: Option<String>, pub draft: String }   // Debug, Clone, Deserialize
    impl WindowEntry { pub fn remove_tab(&mut self, tab: &str) }             // active moves to the next tab, else the previous
    impl Workspace {
        pub fn push_window(&mut self) -> String;                              // allocates w-<next_window>
        pub fn window_mut(&mut self, id: &str) -> Option<&mut WindowEntry>;
        pub fn window_of(&self, tab: &str) -> Option<&str>;
    }
    impl Snapshot { pub fn apply_check(&mut self, ev: &CheckEvent) }         // Checking sets checks_total = count, checks_faults = total; Notice also sets checks_stopped
    ```
  - From `crate::jev::check` (unchanged apart from Task 1's `Deserialize`):
    ```rust
    pub enum CheckEvent { Checking { count: usize, total: usize }, Result(CheckResult), Notice(String) }
    pub struct CheckResult { pub claim: String, pub verdict: Verdict, pub confidence: Option<f64>, pub path: Option<String>, pub line: Option<usize> }
    pub enum Verdict { Confirmed, Contradicted, Unclear, NotShown, NotFound }
    ```
- Produces:
  ```rust
  impl SessionStore {
      pub fn close_tab(&self, id: &str) -> Result<Option<String>>;    // the window it was in; None if already closed
      pub fn close_window(&self, window: &str) -> Result<Vec<String>>; // ids of the tabs it closed
      pub fn reopen_tab(&self, id: &str, window: &str) -> Result<()>; // Err(Precondition("That tab is already open in another window."))
      pub fn delete_tab(&self, id: &str) -> Result<()>;               // closed tabs only: Err(Precondition("Close this tab before deleting it."))
      pub fn move_tab(&self, id: &str, to: Option<&str>) -> Result<(String, String)>;  // (from, to); None creates a window
      pub fn import_legacy(&self, window: &str, drafts: Vec<LegacyDraft>, active_root: Option<String>) -> Result<()>;
      pub fn open_roots(&self) -> std::collections::HashSet<String>;
  }
  /// Saves one run to its tab's history (re-exported as livebrainstorm_core::sessions::Recorder).
  pub struct Recorder<'a> { /* store, tab, pending snapshot, saved id */ }
  impl<'a> Recorder<'a> {
      pub fn new(store: &'a SessionStore, tab: &str, draft: &str, context: &str) -> Self;
      pub fn notice(&mut self, message: &str) -> Result<()>;      // a notice about these notes
      pub fn check(&mut self, ev: &CheckEvent) -> Result<()>;     // before done: kept; after: update_snapshot
      pub fn done(&mut self, markdown: &str, partial: bool) -> Result<u64>;  // record_snapshot; call before sending `done`
  }
  ```
  Behaviour that later phases rely on:
  - `close_tab` and `move_tab` move the source window's `active` to the tab that took the removed one's place, else the one before it.
  - `move_tab` to a new window makes the tab active there. Into an existing window it appends the tab and leaves that window's `active` alone, unless the window has none.
  - `import_legacy` runs only while the workspace is `fresh`, then clears `fresh`. Later calls do nothing.

- [ ] **Step 1: Write the failing tests**

In `crates/livebrainstorm-core/src/sessions/store.rs`, at the top of `mod tests` add `use crate::jev::check::{CheckResult, Verdict};` after `use super::*;`, then add these tests and helper after `concurrent_changes_never_lose_an_update`:

```rust
    #[test]
    fn closing_a_tab_keeps_it_for_recently_closed() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", Some("/a".into())).unwrap();
        let b = s.create_tab("w-1", None).unwrap();
        let c = s.create_tab("w-1", None).unwrap();
        s.activate("w-1", &b.id).unwrap();
        assert_eq!(s.close_tab(&b.id).unwrap().as_deref(), Some("w-1"));
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str(), c.id.as_str()]);
        assert_eq!(view.active.map(|t| t.id), Some(c.id.clone()), "the tab that took its place");
        assert_eq!(ids(&view.closed), [b.id.as_str()]);
        assert!(view.closed[0].closed_at.is_some());
        assert_eq!(s.open_tab_root(&b.id).unwrap_err().to_string(), "This tab was closed.");
        assert_eq!(s.tab(&b.id).unwrap().id, b.id, "its file stays");
        assert_eq!(s.close_tab(&b.id).unwrap(), None, "closing it again changes nothing");
        s.close_tab(&c.id).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().active.map(|t| t.id), Some(a.id.clone()), "at the end: the one before it");
        s.close_tab(&a.id).unwrap();
        let empty = s.window_view("w-1").unwrap();
        assert!(empty.tabs.is_empty() && empty.active.is_none(), "the window stays, empty");
        assert_eq!(s.window_ids(), ["w-1"]);
    }

    #[test]
    fn recently_closed_is_newest_first_and_a_half_finished_close_sorts_by_its_last_edit() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        let b = s.create_tab("w-1", None).unwrap();
        s.close_tab(&a.id).unwrap();
        s.close_tab(&b.id).unwrap();
        let closed = s.closed();
        assert_eq!(ids(&closed), [b.id.as_str(), a.id.as_str()]);
        let (a_at, b_at) = (closed[1].closed_at.unwrap(), closed[0].closed_at.unwrap());
        // Tab files no window lists that never got `closedAt` (the app stopped mid-close),
        // and files that aren't tabs.
        let tabs = sessions(&dir).join("tabs");
        std::fs::write(tabs.join("t-c0ffee.json"), format!(r#"{{"id":"t-c0ffee","updatedAt":{}}}"#, b_at + 100)).unwrap();
        std::fs::write(tabs.join("t-01d.json"), format!(r#"{{"id":"t-01d","updatedAt":{}}}"#, a_at - 100)).unwrap();
        std::fs::write(tabs.join("t-abc.json.bad"), "kept").unwrap();
        std::fs::write(tabs.join("t-abc.json.tmp"), "half written").unwrap();
        std::fs::write(tabs.join("notes.txt"), "not a tab").unwrap();
        assert_eq!(ids(&s.closed()), ["t-c0ffee", b.id.as_str(), a.id.as_str(), "t-01d"]);
    }

    #[test]
    fn reopening_puts_a_tab_back_and_refuses_an_open_one() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        s.close_tab(&a.id).unwrap();
        let w2 = s.add_window().unwrap();
        s.reopen_tab(&a.id, &w2).unwrap();
        let view = s.window_view(&w2).unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str()]);
        assert_eq!(view.active.as_ref().map(|t| (t.id.as_str(), t.closed_at)), Some((a.id.as_str(), None)));
        assert!(view.closed.is_empty());
        let err = s.reopen_tab(&a.id, "w-1").unwrap_err();
        assert!(matches!(err, Error::Precondition(_)));
        assert_eq!(err.to_string(), "That tab is already open in another window.");
        assert_eq!(s.reopen_tab("t-dead", "w-1").unwrap_err().to_string(), "This tab was closed.");
        s.close_tab(&a.id).unwrap();
        assert_eq!(s.reopen_tab(&a.id, "w-9").unwrap_err().to_string(), "This window was closed.");
    }

    #[test]
    fn only_closed_tabs_can_be_deleted() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        assert_eq!(s.delete_tab(&a.id).unwrap_err().to_string(), "Close this tab before deleting it.");
        s.close_tab(&a.id).unwrap();
        s.delete_tab(&a.id).unwrap();
        assert!(!sessions(&dir).join(format!("tabs/{}.json", a.id)).exists());
        assert!(s.closed().is_empty());
        assert_eq!(s.tab(&a.id).unwrap_err().to_string(), "This tab was closed.");
        s.delete_tab(&a.id).unwrap(); // already gone: nothing to do
    }

    #[test]
    fn a_closed_tab_still_takes_a_late_draft_save() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        s.close_tab(&a.id).unwrap();
        s.save_draft(&a.id, "typed just before closing".into()).unwrap();
        assert_eq!(s.tab(&a.id).unwrap().draft, "typed just before closing");
        assert_eq!(ids(&s.closed()), [a.id.as_str()], "and it stays closed");
    }

    #[test]
    fn closing_a_window_closes_its_tabs() {
        let (_dir, s) = store();
        let w2 = s.add_window().unwrap();
        let x = s.create_tab(&w2, Some("/x".into())).unwrap();
        let y = s.create_tab(&w2, None).unwrap();
        assert_eq!(s.close_window(&w2).unwrap(), [x.id.clone(), y.id.clone()]);
        assert_eq!(s.window_ids(), ["w-1"]);
        let closed = s.closed();
        assert_eq!(closed.len(), 2);
        assert!(closed.iter().all(|t| t.closed_at.is_some()));
        assert_eq!(s.close_window(&w2).unwrap_err().to_string(), "This window was closed.");
        assert_eq!(s.add_window().unwrap(), "w-3", "window ids are never reused");
    }

    #[test]
    fn moving_a_tab() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        let b = s.create_tab("w-1", None).unwrap(); // active
        assert_eq!(s.move_tab(&b.id, None).unwrap(), ("w-1".to_owned(), "w-2".to_owned()), "None makes a new window");
        let here = s.window_view("w-1").unwrap();
        assert_eq!(ids(&here.tabs), [a.id.as_str()]);
        assert_eq!(here.active.map(|t| t.id), Some(a.id.clone()), "the old window no longer shows it");
        let there = s.window_view("w-2").unwrap();
        assert_eq!(ids(&there.tabs), [b.id.as_str()]);
        assert_eq!(there.active.map(|t| t.id), Some(b.id.clone()));
        assert_eq!(s.move_tab(&a.id, Some("w-2")).unwrap(), ("w-1".to_owned(), "w-2".to_owned()));
        let there = s.window_view("w-2").unwrap();
        assert_eq!(ids(&there.tabs), [b.id.as_str(), a.id.as_str()], "it joins the end");
        assert_eq!(there.active.map(|t| t.id), Some(b.id.clone()), "that window keeps showing its own tab");
        assert!(s.window_view("w-1").unwrap().active.is_none());
        assert_eq!(s.move_tab(&a.id, Some("w-2")).unwrap(), ("w-2".to_owned(), "w-2".to_owned()), "already there");
        assert_eq!(s.move_tab(&a.id, Some("w-9")).unwrap_err().to_string(), "This window was closed.");
        assert_eq!(ids(&s.window_view("w-2").unwrap().tabs), [b.id.as_str(), a.id.as_str()], "a failed move changes nothing");
        s.close_tab(&a.id).unwrap();
        assert_eq!(s.move_tab(&a.id, None).unwrap_err().to_string(), "This tab was closed.");
    }

    #[test]
    fn legacy_drafts_are_imported_once() {
        let (dir, s) = store();
        assert!(s.window_view("w-1").unwrap().fresh);
        let drafts = vec![
            LegacyDraft { root: Some("/b".into()), draft: "b idea".into() },
            LegacyDraft { root: None, draft: "a loose idea".into() },
            LegacyDraft { root: Some("/a".into()), draft: "  \n".into() },
        ];
        s.import_legacy("w-1", drafts, Some("/b".into())).unwrap();
        let view = s.window_view("w-1").unwrap();
        assert!(!view.fresh);
        let tabs: Vec<(Option<&str>, &str)> = view.tabs.iter().map(|t| (t.root.as_deref(), t.first_line.as_str())).collect();
        assert_eq!(tabs, [(Some("/b"), "b idea"), (None, "a loose idea")], "empty drafts are skipped");
        assert_eq!(view.active.and_then(|t| t.root), Some("/b".to_owned()), "the last project's tab is active");
        s.import_legacy("w-1", vec![LegacyDraft { root: None, draft: "again".into() }], None).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().tabs.len(), 2, "a second import does nothing");
        assert!(!at(&dir).window_view("w-1").unwrap().fresh, "nor does the next launch offer one");
    }

    #[test]
    fn without_a_last_project_the_no_project_draft_is_active() {
        let (_dir, s) = store();
        let drafts = vec![
            LegacyDraft { root: Some("/a".into()), draft: "a".into() },
            LegacyDraft { root: None, draft: "loose".into() },
        ];
        s.import_legacy("w-1", drafts, None).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().active.map(|t| t.draft), Some("loose".to_owned()));
        let (_dir2, s2) = store();
        s2.import_legacy("w-1", vec![LegacyDraft { root: Some("/a".into()), draft: "a".into() }], Some("/gone".into())).unwrap();
        assert_eq!(s2.window_view("w-1").unwrap().active.map(|t| t.draft), Some("a".to_owned()), "no match: the first tab");
    }

    #[test]
    fn open_roots_are_the_projects_of_open_tabs() {
        let (_dir, s) = store();
        s.create_tab("w-1", Some("/a".into())).unwrap();
        let b = s.create_tab("w-1", Some("/b".into())).unwrap();
        s.create_tab("w-1", None).unwrap();
        let w2 = s.add_window().unwrap();
        s.create_tab(&w2, Some("/c".into())).unwrap();
        s.close_tab(&b.id).unwrap();
        let mut roots: Vec<String> = s.open_roots().into_iter().collect();
        roots.sort_unstable();
        assert_eq!(roots, ["/a", "/c"]);
        assert_eq!(s.window_of(&b.id), None);
    }

    fn check(claim: &str) -> CheckResult {
        CheckResult {
            claim: claim.into(),
            verdict: Verdict::Confirmed,
            confidence: Some(0.9),
            path: Some("src/a.rs".into()),
            line: Some(1),
        }
    }

    #[test]
    fn a_run_is_saved_at_done_with_the_checks_that_came_first() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", Some("/p".into())).unwrap();
        let mut rec = Recorder::new(&s, &tab.id, "the draft", "/p|1|");
        rec.notice("Jev was slow; used keyword order.").unwrap();
        rec.check(&CheckEvent::Checking { count: 2, total: 3 }).unwrap();
        rec.check(&CheckEvent::Result(check("first"))).unwrap();
        assert!(s.tab(&tab.id).unwrap().history.is_empty(), "nothing is saved before done");
        let id = rec.done("## Intent\n- x", false).unwrap();
        let t = s.tab(&tab.id).unwrap();
        let saved = &t.history[0];
        assert_eq!(saved.id, id);
        assert_eq!((saved.draft.as_str(), saved.context.as_str(), saved.markdown.as_str()), ("the draft", "/p|1|", "## Intent\n- x"));
        assert_eq!(saved.notices, ["Jev was slow; used keyword order."]);
        assert_eq!(saved.checks, [check("first")]);
        assert_eq!((saved.checks_total, saved.checks_faults, saved.checks_stopped, saved.partial), (Some(2), Some(3), false, false));
        assert!(saved.at > 0);
        // Later checks and notices update the saved snapshot.
        rec.check(&CheckEvent::Result(check("second"))).unwrap();
        rec.check(&CheckEvent::Notice("Jev is unavailable.".into())).unwrap();
        let t = s.tab(&tab.id).unwrap();
        assert_eq!(t.history.len(), 1);
        assert_eq!(t.history[0].checks, [check("first"), check("second")]);
        assert_eq!(t.history[0].notices, ["Jev was slow; used keyword order.", "Jev is unavailable."]);
        assert!(t.history[0].checks_stopped);
    }

    #[test]
    fn checks_cut_off_by_an_edit_stay_short_of_their_total() {
        // The next keystroke aborts the run after done: the snapshot keeps what arrived.
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        let mut rec = Recorder::new(&s, &tab.id, "d", "|0|");
        rec.check(&CheckEvent::Checking { count: 3, total: 3 }).unwrap();
        rec.done("## Faults\n- a", true).unwrap();
        rec.check(&CheckEvent::Result(check("first"))).unwrap();
        drop(rec);
        let t = s.tab(&tab.id).unwrap();
        assert_eq!((t.history[0].checks.len(), t.history[0].checks_total, t.history[0].checks_stopped), (1, Some(3), false));
        assert!(t.history[0].partial);
    }

    #[test]
    fn a_run_that_never_reaches_done_saves_nothing() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        let mut rec = Recorder::new(&s, &tab.id, "d", "|0|");
        rec.notice("Jev was slow; used keyword order.").unwrap();
        rec.check(&CheckEvent::Checking { count: 1, total: 1 }).unwrap();
        drop(rec);
        assert!(s.tab(&tab.id).unwrap().history.is_empty());
        let mut gone = Recorder::new(&s, "t-dead", "d", "|0|");
        assert_eq!(gone.done("## Intent", false).unwrap_err().to_string(), "This tab was closed.");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core sessions::store::`
Expected: compile errors, such as `no method named `close_tab` found for struct `SessionStore`` and `cannot find struct, variant or union type `Recorder``.

- [ ] **Step 3: Add the lifecycle methods**

In `crates/livebrainstorm-core/src/sessions/store.rs`, change the `use super::{…};` line to:

```rust
use super::{first_line, tab_id, valid_id, LegacyDraft, Snapshot, Tab, TabSummary, WindowView, Workspace, HISTORY_LEN};
use crate::jev::check::CheckEvent;
```

Inside `impl SessionStore`, add this block after `closed` and before the `// ---------- history ----------` comment:

```rust
    /// Closes a tab: its window drops it (the next tab becomes active) and its
    /// file gets `closed_at`, so it shows under Recently closed. Returns the
    /// window it was in, or `None` if it was already closed.
    pub fn close_tab(&self, id: &str) -> Result<Option<String>> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        let mut ws = self.read_ws(inner)?;
        let Some(window) = ws.window_of(id).map(str::to_owned) else { return Ok(None) };
        if let Some(entry) = ws.window_mut(&window) {
            entry.remove_tab(id);
        }
        // The workspace first: if the app stops before the tab file is written,
        // the tab is still closed (no window lists it) and sorts by its last edit.
        self.put_ws(inner, ws)?;
        tab.closed_at = Some(self.now_secs());
        self.put_tab(inner, tab)?;
        Ok(Some(window))
    }

    /// Closes every tab of `window` (they show under Recently closed) and
    /// removes the window. Returns the ids of the tabs it closed.
    pub fn close_window(&self, window: &str) -> Result<Vec<String>> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let pos = ws.windows.iter().position(|w| w.id == window).ok_or_else(window_closed)?;
        let entry = ws.windows.remove(pos);
        self.put_ws(inner, ws)?;
        let now = self.now_secs();
        for id in &entry.tabs {
            let Some(mut tab) = self.read_tab_logged(inner, id) else { continue };
            tab.closed_at = Some(now);
            if let Err(e) = self.put_tab(inner, tab) {
                // Still closed (no window lists it); it sorts by its last edit.
                eprintln!("LiveBrainstorm: couldn't mark tab {id} closed: {e}");
            }
        }
        Ok(entry.tabs)
    }

    /// Puts a closed tab back, at the end of `window`, as its active tab.
    pub fn reopen_tab(&self, id: &str, window: &str) -> Result<()> {
        validate(id)?;
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if ws.window_of(id).is_some() {
            return Err(open_elsewhere());
        }
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let mut tab = self.existing_tab(inner, id)?;
        tab.closed_at = None;
        self.put_tab(inner, tab)?;
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        entry.tabs.push(id.to_owned());
        entry.active = Some(id.to_owned());
        self.put_ws(inner, ws)
    }

    /// Deletes a closed tab's file for good.
    pub fn delete_tab(&self, id: &str) -> Result<()> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        if self.read_ws(inner)?.window_of(id).is_some() {
            return Err(Error::Precondition("Close this tab before deleting it.".into()));
        }
        match std::fs::remove_file(self.tab_path(id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        inner.tabs.remove(id);
        Ok(())
    }

    /// Moves an open tab to the end of window `to`, or of a new window
    /// (`None`), which then shows it. Its old window shows another tab.
    /// Returns `(from, to)`. The shell cancels the tab's run and makes it
    /// active in `to`.
    pub fn move_tab(&self, id: &str, to: Option<&str>) -> Result<(String, String)> {
        validate(id)?;
        if let Some(window) = to {
            validate(window)?;
        }
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let from = ws.window_of(id).map(str::to_owned).ok_or_else(tab_closed)?;
        let to = match to {
            Some(window) if window == from => return Ok((from.clone(), from)),
            Some(window) if ws.windows.iter().any(|w| w.id == window) => window.to_owned(),
            Some(_) => return Err(window_closed()),
            None => ws.push_window(),
        };
        if let Some(source) = ws.window_mut(&from) {
            source.remove_tab(id);
        }
        let target = ws.window_mut(&to).ok_or_else(window_closed)?;
        target.tabs.push(id.to_owned());
        if target.active.is_none() {
            target.active = Some(id.to_owned());
        }
        self.put_ws(inner, ws)?;
        Ok((from, to))
    }

    /// The first launch after the upgrade: every non-empty draft the old build
    /// kept becomes a tab at the end of `window`, and `active_root`'s tab (else
    /// the first one imported) is active. Runs once: while the workspace is
    /// `fresh`, which it then stops being. Later calls do nothing.
    pub fn import_legacy(&self, window: &str, drafts: Vec<LegacyDraft>, active_root: Option<String>) -> Result<()> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if !inner.fresh {
            return Ok(());
        }
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let mut imported = Vec::new();
        let mut active = None;
        for legacy in drafts {
            if legacy.draft.trim().is_empty() {
                continue;
            }
            let tab = self.new_tab(inner, legacy.root, legacy.draft);
            if active.is_none() && tab.root == active_root {
                active = Some(tab.id.clone());
            }
            imported.push(tab.id.clone());
            self.put_tab(inner, tab)?;
        }
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        if let Some(first) = imported.first() {
            entry.active = Some(active.unwrap_or_else(|| first.clone()));
        }
        entry.tabs.extend(imported);
        self.put_ws(inner, ws)?;
        inner.fresh = false;
        Ok(())
    }

    /// The projects of every open tab, in any window: what the shell keeps loaded.
    pub fn open_roots(&self) -> HashSet<String> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let Ok(ws) = self.read_ws(inner) else { return HashSet::new() };
        let mut roots = HashSet::new();
        for id in ws.windows.iter().flat_map(|w| &w.tabs) {
            if let Some(Tab { root: Some(root), .. }) = self.read_tab_logged(inner, id) {
                roots.insert(root);
            }
        }
        roots
    }
```

- [ ] **Step 4: Add `Recorder`**

In the same file, add this after the closing `}` of `impl SessionStore` and before `fn validate`:

```rust
/// Saves one run to its tab's history. Fact-check events can come before or
/// after `done`: before, they're kept and saved with the notes; after, they
/// update the saved snapshot. A run that never reaches `done` (a skip, an
/// error, an edit that aborted it) saves nothing.
pub struct Recorder<'a> {
    store: &'a SessionStore,
    tab: String,
    pending: Snapshot,
    saved: Option<u64>,
}

impl<'a> Recorder<'a> {
    pub fn new(store: &'a SessionStore, tab: &str, draft: &str, context: &str) -> Self {
        let pending = Snapshot { draft: draft.to_owned(), context: context.to_owned(), ..Snapshot::default() };
        Self { store, tab: tab.to_owned(), pending, saved: None }
    }

    /// A notice about these notes: the pre-phase's, or the missing sections'.
    pub fn notice(&mut self, message: &str) -> Result<()> {
        self.apply(|s| s.notices.push(message.to_owned()))
    }

    pub fn check(&mut self, ev: &CheckEvent) -> Result<()> {
        self.apply(|s| s.apply_check(ev))
    }

    /// Saves the notes and returns the snapshot's id. Call it before sending
    /// `done`, so a `get_tab` right after `done` includes them.
    pub fn done(&mut self, markdown: &str, partial: bool) -> Result<u64> {
        let snap = Snapshot { at: self.store.now_secs(), markdown: markdown.to_owned(), partial, ..self.pending.clone() };
        let id = self.store.record_snapshot(&self.tab, snap)?;
        self.saved = Some(id);
        Ok(id)
    }

    fn apply(&mut self, f: impl FnOnce(&mut Snapshot)) -> Result<()> {
        match self.saved {
            None => {
                f(&mut self.pending);
                Ok(())
            }
            Some(id) => self.store.update_snapshot(&self.tab, id, f),
        }
    }
}
```

In `crates/livebrainstorm-core/src/sessions/mod.rs`, change `pub use store::SessionStore;` to:

```rust
pub use store::{Recorder, SessionStore};
```

- [ ] **Step 5: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core sessions:: && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected:
- The 13 new tests PASS, along with Task 3's 13 and Task 2's nine.
- The whole core suite passes, and clippy prints no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/sessions
git commit -m "feat(core): close, reopen, delete and move tabs; one-time legacy import; Recorder saves a run's notes and checks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 5: Shell: the project registry, one run per tab, one brief build per project

`src-tauri` has no tests. This task is verified by `cargo check`, clippy and the core suite. Between this task and Task 9 the running app is broken, because the frontend still calls `analyze` and `build_brief` with their old arguments. Nobody runs the app until then.

**Files:**
- Modify: `src-tauri/src/state.rs` (replace the whole file)
- Create: `src-tauri/src/events.rs`
- Modify: `src-tauri/src/commands.rs:5-26` (imports), `:137-229` (the project section and `build_brief`), `:255-361` (`analyze` and `cancel_analysis`)
- Modify: `src-tauri/src/lib.rs` (`mod events;`, `commands::project_summary`)

**Interfaces:**
- Consumes:
  - Task 3: `SessionStore::new(dir: PathBuf) -> Self` and `SessionStore::open_tab_root(&self, id: &str) -> Result<Option<String>>`, which returns `Err(Precondition("This tab was closed."))` for a closed or unknown tab.
  - Task 4: `Recorder::new(store: &SessionStore, tab: &str, draft: &str, context: &str)`, `Recorder::notice(&mut self, message: &str) -> Result<()>`, `Recorder::check(&mut self, ev: &CheckEvent) -> Result<()>` and `Recorder::done(&mut self, markdown: &str, partial: bool) -> Result<u64>`.
  - Task 2: `valid_id(id: &str) -> bool`.
- Produces:
  - `state.rs`:
    ```rust
    pub struct ProjectEntry { pub index: Arc<ProjectIndex>, pub brief: Option<StoredBrief> }
    // AppState fields: projects: RwLock<HashMap<String, Arc<ProjectEntry>>>, analyses: Mutex<HashMap<String, JoinHandle<()>>>,
    //                  brief_tasks: Mutex<HashMap<String, JoinHandle<()>>>, sessions: SessionStore
    //                  (`project`, `brief`, `analysis` and `brief_task` are gone)
    impl AppState { pub fn new(data_dir: &Path, config_dir: &Path) -> Self; pub fn project_entry(&self, root: &str) -> Option<Arc<ProjectEntry>>; }
    pub fn replace_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str, task: JoinHandle<()>);
    pub fn cancel_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str);
    ```
  - `events.rs`: `pub const BRIEF_STATUS: &str = "brief-status";`, `#[serde(rename_all = "lowercase")] pub enum BriefState { Building, Done, Error }`, `#[serde(rename_all = "camelCase")] pub struct BriefStatus { pub root: String, pub state: BriefState, pub message: Option<String> }` (both `Clone, Serialize`), and `pub fn brief_status(app: &AppHandle, root: &str, state: BriefState, message: Option<String>)`.
  - Commands:
    - `open_project(path) -> ProjectSummary`: scans and inserts or replaces the registry entry.
    - `project_summary(root) -> ProjectSummary`: scans only if the root isn't loaded. A missing folder is `Precondition("That folder no longer exists. Open it again from its new place.")`.
    - `build_brief(root, on_event)`.
    - `analyze(tab_id, draft, previous_draft, instructions, context, on_event)`.
    - `cancel_analysis(tab_id) -> Result<()>`.
  - `analyze` binds `root: Option<String>` in the command body (`open_tab_root`, so a closed tab returns an `Err`) and moves it into the spawned task, which binds `state` (the `AppState`) and keeps today's lane loop. `build_brief`'s binds `task_app` and `index: Arc<ProjectIndex>`, and keeps today's three `provider.stream` calls (Phase 4 wraps them).

- [ ] **Step 1: State: the registry and keyed task slots**

Replace `src-tauri/src/state.rs` with:

```rust
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use livebrainstorm_core::instructions::InstructionStore;
use livebrainstorm_core::local::{LocalPaths, LocalServer};
use livebrainstorm_core::project::{BriefStore, ProjectIndex, StoredBrief};
use livebrainstorm_core::sessions::SessionStore;
use livebrainstorm_core::settings::Settings;
use tauri::async_runtime::JoinHandle;

/// One loaded project: its index (which keeps file contents) and its cached brief.
pub struct ProjectEntry {
    pub index: Arc<ProjectIndex>,
    pub brief: Option<StoredBrief>,
}

pub struct AppState {
    pub http: reqwest::Client,
    pub settings_path: std::path::PathBuf,
    pub settings: RwLock<Settings>,
    pub briefs: BriefStore,
    /// Session instructions and templates (`instructions.json`).
    pub instructions: InstructionStore,
    pub local_paths: LocalPaths,
    /// Loaded projects, by root, shared by every tab and window.
    pub projects: RwLock<HashMap<String, Arc<ProjectEntry>>>,
    /// One live analysis per tab: a new run for a tab aborts only that tab's previous run.
    pub analyses: Mutex<HashMap<String, JoinHandle<()>>>,
    /// One brief build per project root.
    pub brief_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Windows, tabs, drafts and saved analyses (`<app_data>/sessions`).
    pub sessions: SessionStore,
    /// Keyed by "runtime" or a model id.
    pub downloads: Mutex<HashMap<String, JoinHandle<()>>>,
    /// tokio Mutex: held across the (slow) server startup.
    pub local: tokio::sync::Mutex<Option<LocalServer>>,
}

impl AppState {
    pub fn new(data_dir: &Path, config_dir: &Path) -> Self {
        let settings_path = config_dir.join("settings.json");
        Self {
            http: reqwest::Client::new(),
            settings: RwLock::new(Settings::load(&settings_path)),
            settings_path,
            briefs: BriefStore::new(data_dir.join("briefs")),
            instructions: InstructionStore::new(config_dir.join("instructions.json")),
            local_paths: LocalPaths::new(data_dir),
            projects: RwLock::new(HashMap::new()),
            analyses: Mutex::new(HashMap::new()),
            brief_tasks: Mutex::new(HashMap::new()),
            sessions: SessionStore::new(data_dir.join("sessions")),
            downloads: Mutex::new(HashMap::new()),
            local: tokio::sync::Mutex::new(None),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().expect("settings lock").clone()
    }

    /// The loaded project for `root`, if any window has loaded it.
    pub fn project_entry(&self, root: &str) -> Option<Arc<ProjectEntry>> {
        self.projects.read().expect("projects lock").get(root).cloned()
    }
}

/// Swaps in a new task for `key`, aborting whatever was running for that key only.
pub fn replace_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str, task: JoinHandle<()>) {
    if let Some(old) = slots.lock().expect("task lock").insert(key.to_owned(), task) {
        old.abort();
    }
}

/// Aborts the task for `key`, if any. Aborting an analysis drops its `claude`
/// children too (`kill_on_drop`).
pub fn cancel_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str) {
    if let Some(task) = slots.lock().expect("task lock").remove(key) {
        task.abort();
    }
}
```

- [ ] **Step 2: Events every window receives**

Create `src-tauri/src/events.rs`:

```rust
//! Events every window receives (`app.emit`), unlike a command's own Channel.
//! Names and payloads mirror `src/lib/events.ts`.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const BRIEF_STATUS: &str = "brief-status";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BriefState {
    Building,
    Done,
    Error,
}

/// A brief build for `root` started, finished or failed. On `done`, windows
/// showing a tab of that project refetch its summary.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefStatus {
    pub root: String,
    pub state: BriefState,
    pub message: Option<String>,
}

pub fn brief_status(app: &AppHandle, root: &str, state: BriefState, message: Option<String>) {
    let _ = app.emit(BRIEF_STATUS, BriefStatus { root: root.to_owned(), state, message }); // a window may be gone
}
```

In `src-tauri/src/lib.rs`, add `mod events;` after `mod commands;`.

- [ ] **Step 3: Imports in `commands.rs`**

In `src-tauri/src/commands.rs`, replace the `use` lines (lines 5-26, from `use std::path::PathBuf;` to `use crate::state::{replace_task, AppState};`) with:

```rust
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use livebrainstorm_core::agents;
use livebrainstorm_core::analysis::{self, PreInput, PreOutcome, PRE_BUDGET};
use livebrainstorm_core::instructions::{InstructionsView, ProjectInstructions, Template};
use livebrainstorm_core::jev::key::KeyStatus;
use livebrainstorm_core::jev::check::{self, CheckEvent, CheckResult};
use livebrainstorm_core::llm::Chunk;
use livebrainstorm_core::local::{self, catalog, download, runtime, LocalInventory};
use livebrainstorm_core::project::explore;
use livebrainstorm_core::project::{self, BriefStatus, StoredBrief};
use livebrainstorm_core::sessions::{valid_id, Recorder};
use livebrainstorm_core::settings::{ProviderSettings, Settings, Tier};
use livebrainstorm_core::{prompt, Error, Result};
use serde::Serialize;
use tauri::ipc::Channel;
use tokio::sync::oneshot;
use tauri::{AppHandle, Manager, State};

use crate::events::{self, BriefState};
use crate::provider;
use crate::secrets;
use crate::state::{cancel_task, replace_task, AppState, ProjectEntry};
```

- [ ] **Step 4: The project section and `build_brief`**

Replace everything from `// ---------- project ----------` down to the end of `build_brief` (the `}` just above `// ---------- session instructions ----------`) with:

```rust
// ---------- project ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    name: String,
    root: String,
    indexed_files: usize,
    total_files: usize,
    brief_status: BriefStatus,
    brief: Option<StoredBrief>,
}

impl ProjectSummary {
    fn of(entry: &ProjectEntry) -> Self {
        let index = &entry.index;
        Self {
            name: index.name.clone(),
            root: index.root.display().to_string(),
            indexed_files: index.files.len(),
            total_files: index.tree.len(),
            brief_status: BriefStatus::of(entry.brief.as_ref(), index),
            brief: entry.brief.clone(),
        }
    }
}

/// Scans `root` on a blocking thread, loads its cached brief, and puts it in
/// the registry under its canonical root, replacing any earlier entry. Other
/// projects stay loaded.
async fn load_project(state: &AppState, root: PathBuf) -> Result<Arc<ProjectEntry>> {
    let index = tauri::async_runtime::spawn_blocking(move || project::scan(&root))
        .await
        .map_err(|e| Error::Provider(format!("Scan failed: {e}")))??;
    let brief = state.briefs.load(&index.root);
    let entry = Arc::new(ProjectEntry { index: Arc::new(index), brief });
    let key = entry.index.root.display().to_string();
    state.projects.write().expect("projects lock").insert(key, entry.clone());
    Ok(entry)
}

/// The loaded project for a tab's root, scanned first if no window has loaded
/// it since launch.
async fn project_or_scan(state: &AppState, root: &str) -> Result<Arc<ProjectEntry>> {
    if let Some(entry) = state.project_entry(root) {
        return Ok(entry);
    }
    if !Path::new(root).is_dir() {
        return Err(Error::Precondition("That folder no longer exists. Open it again from its new place.".into()));
    }
    load_project(state, PathBuf::from(root)).await
}

/// "Open a project…", and explicit refreshes: always rescans.
#[tauri::command]
pub async fn open_project(app: AppHandle, path: String) -> Result<ProjectSummary> {
    let state = app.state::<AppState>();
    let entry = load_project(&state, PathBuf::from(path)).await?;
    Ok(ProjectSummary::of(&entry))
}

/// The summary a window shows for a tab's project. Scans only a project that
/// isn't loaded yet.
#[tauri::command]
pub async fn project_summary(app: AppHandle, root: String) -> Result<ProjectSummary> {
    let state = app.state::<AppState>();
    let entry = project_or_scan(&state, &root).await?;
    Ok(ProjectSummary::of(&entry))
}

/// One build per project. While one runs for `root`, another request (from
/// any window) returns at once and follows it through `brief-status`, which
/// every window receives. The window that started it also gets the live text
/// and activity over its Channel.
#[tauri::command]
pub async fn build_brief(app: AppHandle, root: String, on_event: Channel<UiEvent>) -> Result<()> {
    let state = app.state::<AppState>();
    let index = project_or_scan(&state, &root).await?.index.clone();
    let mut tasks = state.brief_tasks.lock().expect("task lock");
    if tasks.get(&root).is_some_and(|task| !task.inner().is_finished()) {
        return Ok(());
    }
    events::brief_status(&app, &root, BriefState::Building, None);
    let task_app = app.clone();
    let task_root = root.clone();

    let task = tauri::async_runtime::spawn(async move {
        let result: Result<StoredBrief> = async {
            let settings = task_app.state::<AppState>().settings();
            let provider = provider::resolve(&task_app, Tier::Deep).await?;
            let slices = explore::slices(&index, settings.effective_brief_agents());
            let (text, agents) = if slices.len() <= 1 {
                // One explorer: today's path, unchanged.
                let req = prompt::brief_request(&index, &provider);
                (provider.stream(&req, &mut on_chunk(&on_event)).await?, 1)
            } else {
                // Explorers map their slices at once; their activity shows as
                // "Agent k". A `for` loop, not `.map(…)`, inside this spawned task.
                let n = slices.len();
                let mut explorers = Vec::new();
                for (k, slice) in slices.iter().enumerate() {
                    let req = prompt::explorer_request(&index, &provider, slice, k + 1, n);
                    let (provider, on_event) = (&provider, &on_event);
                    explorers.push(async move {
                        let lane = k + 1;
                        let mut sink = |chunk: Chunk| {
                            if let Chunk::Activity(label) = chunk {
                                send(on_event, UiEvent::Activity { label, lane });
                            }
                        };
                        provider.stream(&req, &mut sink).await
                    });
                }
                let (notes, unmapped) = explore::collect_notes(&slices, agents::run_limited(n, explorers).await)?;
                let req = prompt::merge_request(&index, &provider, &notes, &unmapped);
                (provider.stream(&req, &mut on_chunk(&on_event)).await?, n)
            };
            let brief = StoredBrief {
                root: index.root.display().to_string(),
                fingerprint: index.fingerprint.clone(),
                text: text.trim().to_owned(),
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                source: explore::brief_source(&settings.provider.label(), agents),
            };
            task_app.state::<AppState>().briefs.save(&brief)?;
            Ok(brief)
        }
        .await;
        let state = task_app.state::<AppState>();
        match result {
            Ok(brief) => {
                // Rescan, so every tab of the project gets the new brief and
                // the files changed while it was written.
                if load_project(&state, index.root.clone()).await.is_err() {
                    let entry = ProjectEntry { index: index.clone(), brief: Some(brief.clone()) };
                    state.projects.write().expect("projects lock").insert(task_root.clone(), Arc::new(entry));
                }
                events::brief_status(&task_app, &task_root, BriefState::Done, None);
                send(&on_event, UiEvent::Done { text: brief.text, partial: false });
            }
            Err(e) => {
                events::brief_status(&task_app, &task_root, BriefState::Error, Some(e.to_string()));
                send(&on_event, UiEvent::Error { message: e.to_string() });
            }
        }
    });
    tasks.insert(root, task);
    Ok(())
}
```

The `tasks` guard is taken after the last `.await` in this command and is never held across one.

- [ ] **Step 5: `analyze` per tab, saving its snapshot before `done`**

Replace everything from `// ---------- live analysis ----------` down to the end of `cancel_analysis` (the `}` just above `// ---------- local models ----------`) with:

```rust
// ---------- live analysis ----------

/// Every tab or window id from the webview passes this before it is used: a
/// tab id becomes a file name.
fn check_id(id: &str) -> Result<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(Error::Precondition("Unknown tab.".into()))
    }
}

/// A notice about this run's notes: shown now, and saved with them.
fn note(ch: &Channel<UiEvent>, recorder: &Mutex<Recorder<'_>>, message: String) {
    let _ = recorder.lock().expect("recorder lock").notice(&message); // saving never fails a run
    send(ch, UiEvent::Notice { message });
}

/// One run for one tab. A new run for a tab aborts only that tab's previous
/// run, so tabs analyse in parallel. The notes are saved to the tab's history
/// before `done` is sent, so a `get_tab` right after `done` includes them.
#[tauri::command]
pub async fn analyze(
    app: AppHandle,
    tab_id: String,
    draft: String,
    previous_draft: Option<String>,
    instructions: String,
    context: String,
    on_event: Channel<UiEvent>,
) -> Result<()> {
    check_id(&tab_id)?;
    // A closed or unknown tab can't analyse. Its project is read once, as the run starts.
    let root = app.state::<AppState>().sessions.open_tab_root(&tab_id)?;
    let task_app = app.clone();
    let task_tab = tab_id.clone();
    let task = tauri::async_runtime::spawn(async move {
        let state = task_app.state::<AppState>();
        let settings = state.settings();
        // A project no window has loaded yet is scanned now. One that can't be
        // read analyses without code context, like a tab with no project.
        let entry = match root.as_deref() {
            Some(root) => project_or_scan(&state, root).await.ok(),
            None => None,
        };
        let (index, brief) = match &entry {
            Some(entry) => (Some(entry.index.clone()), entry.brief.as_ref().map(|b| b.text.clone())),
            None => (None, None),
        };
        let provider = match provider::resolve(&task_app, Tier::Fast).await {
            Ok(provider) => provider,
            Err(e) => return send(&on_event, UiEvent::Error { message: e.to_string() }),
        };
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
        // This run's snapshot, saved at `done`. Checks that arrive first are
        // kept until then; later checks and notices update the saved snapshot.
        let recorder = Mutex::new(Recorder::new(&state.sessions, &task_tab, &draft, &context));
        if let Some(message) = notice {
            note(&on_event, &recorder, message);
        }

        // Each lane writes some of the sections; up to one agent per section.
        // A plain `for` loop, not `.map(…)`: closures over borrowed items trip
        // rustc's Send check inside this spawned task.
        let groups = prompt::section_groups(settings.effective_note_agents());
        let (faults_tx, faults_rx) = oneshot::channel::<String>();
        let mut faults_tx = Some(faults_tx);
        let mut lanes = Vec::new();
        for (lane, sections) in groups.iter().enumerate() {
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, sections, &instructions);
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
                Ok((text, notice)) => {
                    let partial = notice.is_some();
                    if let Some(message) = notice {
                        note(&on_event, &recorder, message);
                    }
                    // Saved before `done`, so a `get_tab` right after `done` includes it.
                    let saved = recorder.lock().expect("recorder lock").done(&text, partial);
                    if let Err(e) = saved {
                        send(&on_event, UiEvent::Notice { message: format!("Couldn't save these notes: {e}") });
                    }
                    send(&on_event, UiEvent::Done { text, partial });
                }
                Err(e) => send(&on_event, UiEvent::Error { message: e.to_string() }),
            }
        };
        // "Checked in code" starts as soon as the Faults lane finishes, while
        // other lanes may still stream; `done` never waits for it. The next
        // analysis of this tab aborts this task, lanes and checks included.
        let check = async {
            let Ok(text) = faults_rx.await else { return }; // that lane failed
            if !settings.jev_check_enabled() {
                return;
            }
            let (Some(index), Some(jev)) = (index.as_deref(), jev.as_ref()) else { return };
            if let Some(faults) = prompt::section_body(&text, "Faults") {
                check::run(jev, index, &faults, |ev| {
                    let _ = recorder.lock().expect("recorder lock").check(&ev); // saving never fails a run
                    send(&on_event, ev.into());
                })
                .await;
            }
        };
        tokio::join!(write, check);
    });
    replace_task(&app.state::<AppState>().analyses, &tab_id, task);
    Ok(())
}

#[tauri::command]
pub fn cancel_analysis(state: State<'_, AppState>, tab_id: String) -> Result<()> {
    check_id(&tab_id)?;
    cancel_task(&state.analyses, &tab_id); // drops the Claude CLI children too (kill_on_drop)
    Ok(())
}
```

The recorder's mutex is `std`. Every lock is a temporary inside a synchronous statement or closure, never held across `.await`.

- [ ] **Step 6: Register `project_summary`**

In `src-tauri/src/lib.rs`, add `commands::project_summary,` to `generate_handler![…]` right after `commands::open_project,`.

- [ ] **Step 7: Check, lint and test**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core; grep -nE 'state\.project\(\)|state\.brief\.|brief_task[^s]|\(\)\.analysis,|state\.analysis\.' src-tauri/src/*.rs`
Expected:
- The shell compiles and clippy prints no warnings. A Send error such as "FnOnce is not general enough" means a closure over a borrowed argument was added inside the spawned task: rewrite it as a `match` or a `for` loop.
- The core suite passes, including every `prompt::tests` test unchanged (the byte-identical guard).
- The `grep` prints nothing (its exit status is 1 when nothing matches, which is the expected result).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/events.rs src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(shell): project registry, one analysis per tab saved before done, one brief build per project with brief-status

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 6: Shell: the session commands, `workspace-changed`, and unloading unused projects

**Files:**
- Modify: `src-tauri/src/events.rs` (`workspace-changed`)
- Modify: `src-tauri/src/state.rs` (`evict_unused_projects`)
- Modify: `src-tauri/src/commands.rs` (imports; a new `// ---------- sessions ----------` section)
- Modify: `src-tauri/src/lib.rs` (handlers)

**Interfaces:**
- Consumes:
  - Tasks 3 and 4, every method on `SessionStore`:
    ```rust
    window_view(&self, window: &str) -> Result<WindowView>; tab(&self, id: &str) -> Result<Tab>;
    open_tab_root(&self, id: &str) -> Result<Option<String>>; create_tab(&self, window: &str, root: Option<String>) -> Result<Tab>;
    save_draft(&self, id: &str, draft: String) -> Result<bool>; rename_tab(&self, id: &str, title: Option<String>) -> Result<()>;
    activate(&self, window: &str, id: &str) -> Result<()>; close_tab(&self, id: &str) -> Result<Option<String>>;
    reopen_tab(&self, id: &str, window: &str) -> Result<()>; delete_tab(&self, id: &str) -> Result<()>;
    import_legacy(&self, window: &str, drafts: Vec<LegacyDraft>, active_root: Option<String>) -> Result<()>;
    window_of(&self, id: &str) -> Option<String>; open_roots(&self) -> HashSet<String>;
    ```
  - Task 5: `check_id(id: &str) -> Result<()>` (private, in `commands.rs`), `cancel_task(slots, key)`, `AppState.projects`, `AppState.analyses`.
- Produces:
  - `events.rs`: `pub const WORKSPACE_CHANGED: &str = "workspace-changed";`, `#[derive(Clone, Serialize)] #[serde(rename_all = "camelCase")] pub struct WorkspaceChanged { pub windows: Vec<String> }` and `pub fn workspace_changed(app: &AppHandle, windows: Vec<String>)`.
  - `pub fn evict_unused_projects(&self)` on `AppState`, which drops entries not in `sessions.open_roots()`.
  - Commands. Rust takes snake_case args; JS sends camelCase. Every id passes `check_id` first.
    - `list_workspace(window) -> WindowView`
    - `get_tab(tab_id) -> Tab`: open tabs only; "This tab was closed." otherwise.
    - `create_tab(window, root: Option<String>) -> Tab`
    - `save_draft(tab_id, draft)`
    - `rename_tab(tab_id, title: Option<String>)`
    - `activate_tab(window, tab_id)`
    - `close_tab(tab_id)`: cancels the tab's run, then evicts unused projects.
    - `reopen_tab(tab_id, window)`
    - `delete_tab(tab_id)`: evicts unused projects.
    - `import_legacy_drafts(window, drafts: Vec<LegacyDraft>, active_root: Option<String>)`
  - Each mutation emits `workspace-changed` listing the windows whose tab list changed. That list is empty when only the shared closed list changed. `save_draft` emits only when the draft's first line changed (`SessionStore::save_draft` returns `true`), since no list shows the rest.

- [ ] **Step 1: The `workspace-changed` event**

In `src-tauri/src/events.rs`, add after `pub const BRIEF_STATUS: &str = "brief-status";`:

```rust
pub const WORKSPACE_CHANGED: &str = "workspace-changed";
```

Add after the `BriefStatus` struct:

```rust
/// A session changed. `windows` lists the windows whose tab list changed
/// (empty when only the shared closed list did). Every window refetches the
/// closed list.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChanged {
    pub windows: Vec<String>,
}
```

Add at the end of the file:

```rust
pub fn workspace_changed(app: &AppHandle, windows: Vec<String>) {
    let _ = app.emit(WORKSPACE_CHANGED, WorkspaceChanged { windows });
}
```

- [ ] **Step 2: Unloading projects no open tab uses**

In `src-tauri/src/state.rs`, add inside `impl AppState`, after `project_entry`:

```rust
    /// Drops every loaded project that no open tab, in any window, uses: an
    /// index keeps file contents. Call after closing, moving or deleting tabs.
    pub fn evict_unused_projects(&self) {
        let open = self.sessions.open_roots(); // read before taking the projects lock
        self.projects.write().expect("projects lock").retain(|root, _| open.contains(root));
    }
```

- [ ] **Step 3: The session commands**

In `src-tauri/src/commands.rs`, change the sessions import to:

```rust
use livebrainstorm_core::sessions::{valid_id, LegacyDraft, Recorder, Tab, WindowView};
```

Add this section after `restore_starter_templates` and before `// ---------- live analysis ----------`:

```rust
// ---------- sessions ----------

/// This window's tabs, its full active tab, the shared closed list, and `fresh`.
#[tauri::command]
pub fn list_workspace(state: State<'_, AppState>, window: String) -> Result<WindowView> {
    check_id(&window)?;
    state.sessions.window_view(&window)
}

/// The full tab, when a window shows it. A closed tab must be reopened first.
#[tauri::command]
pub fn get_tab(state: State<'_, AppState>, tab_id: String) -> Result<Tab> {
    check_id(&tab_id)?;
    state.sessions.open_tab_root(&tab_id)?;
    state.sessions.tab(&tab_id)
}

#[tauri::command]
pub fn create_tab(app: AppHandle, window: String, root: Option<String>) -> Result<Tab> {
    check_id(&window)?;
    let tab = app.state::<AppState>().sessions.create_tab(&window, root)?;
    events::workspace_changed(&app, vec![window]);
    Ok(tab)
}

/// The frontend saves here 500 ms after typing stops. A closed tab still takes
/// a save that was already on its way. Only a new first line changes a list, so
/// only that announces itself: every pause would otherwise refetch every window.
#[tauri::command]
pub fn save_draft(app: AppHandle, tab_id: String, draft: String) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    if state.sessions.save_draft(&tab_id, draft)? {
        events::workspace_changed(&app, state.sessions.window_of(&tab_id).into_iter().collect());
    }
    Ok(())
}

/// `None` or a blank title returns the tab to its automatic title.
#[tauri::command]
pub fn rename_tab(app: AppHandle, tab_id: String, title: Option<String>) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    state.sessions.rename_tab(&tab_id, title)?;
    events::workspace_changed(&app, state.sessions.window_of(&tab_id).into_iter().collect());
    Ok(())
}

#[tauri::command]
pub fn activate_tab(app: AppHandle, window: String, tab_id: String) -> Result<()> {
    check_id(&window)?;
    check_id(&tab_id)?;
    app.state::<AppState>().sessions.activate(&window, &tab_id)?;
    events::workspace_changed(&app, vec![window]);
    Ok(())
}

/// Stops the tab's run, keeps its file for Recently closed, and unloads its
/// project if no open tab uses it any more.
#[tauri::command]
pub fn close_tab(app: AppHandle, tab_id: String) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    cancel_task(&state.analyses, &tab_id); // drops the Claude CLI children too (kill_on_drop)
    let window = state.sessions.close_tab(&tab_id)?;
    state.evict_unused_projects();
    events::workspace_changed(&app, window.into_iter().collect());
    Ok(())
}

#[tauri::command]
pub fn reopen_tab(app: AppHandle, tab_id: String, window: String) -> Result<()> {
    check_id(&tab_id)?;
    check_id(&window)?;
    app.state::<AppState>().sessions.reopen_tab(&tab_id, &window)?;
    events::workspace_changed(&app, vec![window]);
    Ok(())
}

/// Closed tabs only. Every window refetches its closed list.
#[tauri::command]
pub fn delete_tab(app: AppHandle, tab_id: String) -> Result<()> {
    check_id(&tab_id)?;
    let state = app.state::<AppState>();
    state.sessions.delete_tab(&tab_id)?;
    state.evict_unused_projects();
    events::workspace_changed(&app, Vec::new());
    Ok(())
}

/// The first launch after the upgrade: the old build's drafts become tabs in
/// `window`. Runs once; later calls do nothing.
#[tauri::command]
pub fn import_legacy_drafts(
    app: AppHandle,
    window: String,
    drafts: Vec<LegacyDraft>,
    active_root: Option<String>,
) -> Result<()> {
    check_id(&window)?;
    app.state::<AppState>().sessions.import_legacy(&window, drafts, active_root)?;
    events::workspace_changed(&app, vec![window]);
    Ok(())
}
```

- [ ] **Step 4: Register the commands**

In `src-tauri/src/lib.rs`, add these lines to `generate_handler![…]` right after `commands::cancel_analysis,`:

```rust
            commands::list_workspace,
            commands::get_tab,
            commands::create_tab,
            commands::save_draft,
            commands::rename_tab,
            commands::activate_tab,
            commands::close_tab,
            commands::reopen_tab,
            commands::delete_tab,
            commands::import_legacy_drafts,
```

- [ ] **Step 5: Check and lint**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: the shell compiles, clippy prints no warnings (every new item is used), and the core suite passes.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/events.rs src-tauri/src/state.rs src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(shell): session commands with workspace-changed; unused projects are unloaded

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 7: Frontend building blocks: session types and wrappers, events, the window id, titles, and the one-time import

**Files:**
- Modify: `src/lib/ipc.ts` (types after `ProjectSummary`; wrappers in `api`)
- Create: `src/lib/events.ts`, `src/lib/windowId.ts`, `src/lib/tabTitle.ts`, `src/lib/legacyDrafts.ts`
- Test: `src/lib/events.test.ts`, `src/lib/windowId.test.ts`, `src/lib/tabTitle.test.ts`, `src/lib/legacyDrafts.test.ts`

**Interfaces:**
- Consumes:
  - Task 6's commands: `list_workspace(window)`, `get_tab(tab_id)`, `create_tab(window, root)`, `save_draft(tab_id, draft)`, `rename_tab(tab_id, title)`, `activate_tab(window, tab_id)`, `close_tab(tab_id)`, `reopen_tab(tab_id, window)`, `delete_tab(tab_id)` and `import_legacy_drafts(window, drafts, active_root)`.
  - Task 5's `project_summary(root)` and its events `brief-status { root, state, message }` and `workspace-changed { windows }`.
  - `CheckResult` and `errorMessage`, already in `src/lib/ipc.ts`.
- Produces:
  - `src/lib/ipc.ts`:
    ```ts
    export interface Snapshot {
      id: number; at: number; draft: string; context: string; markdown: string; partial: boolean;
      notices: string[]; checks: CheckResult[]; checksTotal: number | null; checksFaults: number | null;
      checksStopped: boolean;
    }
    export interface Tab {
      id: string; root: string | null; title: string | null; createdAt: number; updatedAt: number;
      closedAt: number | null; draft: string; nextSnapshot: number; history: Snapshot[];   // newest first, at most 5
    }
    export interface TabSummary {
      id: string; root: string | null; title: string | null; firstLine: string; updatedAt: number; closedAt: number | null;
    }
    export interface WindowView {
      window: string; tabs: TabSummary[]; active: Tab | null; closed: TabSummary[]; fresh: boolean; unreadable: string[];
    }
    export interface LegacyDraft { root: string | null; draft: string }
    api.projectSummary(root: string): Promise<ProjectSummary>
    api.listWorkspace(window: string): Promise<WindowView>
    api.getTab(tabId: string): Promise<Tab>
    api.createTab(window: string, root: string | null): Promise<Tab>
    api.saveDraft(tabId: string, draft: string): Promise<void>
    api.renameTab(tabId: string, title: string | null): Promise<void>
    api.activateTab(window: string, tabId: string): Promise<void>
    api.closeTab(tabId: string): Promise<void>
    api.reopenTab(tabId: string, window: string): Promise<void>
    api.deleteTab(tabId: string): Promise<void>
    api.importLegacyDrafts(window: string, drafts: LegacyDraft[], activeRoot: string | null): Promise<void>
    ```
    (`analyze`, `buildBrief` and `cancelAnalysis` change in Task 9, together with their caller.)
  - `src/lib/events.ts`:
    - `export interface BriefStatusEvent { root: string; state: "building" | "done" | "error"; message: string | null }`
    - `export interface WorkspaceChangedEvent { windows: string[] }`
    - `export function onBriefStatus(fn: (e: BriefStatusEvent) => void): Promise<UnlistenFn>`
    - `export function onWorkspaceChanged(fn: (e: WorkspaceChangedEvent) => void): Promise<UnlistenFn>`
  - `src/lib/windowId.ts`: `export function currentWindowId(): string`. It returns `getCurrentWindow().label` when that matches `/^w-/`, else `"w-1"`.
  - `src/lib/tabTitle.ts`:
    - `export const TITLE_CHARS = 48`
    - `export function firstLine(draft: string): string`, which mirrors Rust `sessions::first_line`.
    - `export function tabTitle(t: { title: string | null; firstLine: string }): string`
  - `src/lib/legacyDrafts.ts`:
    - `collectLegacyDrafts(storage: Pick<Storage, "length" | "key" | "getItem">): { drafts: LegacyDraft[]; activeRoot: string | null; keys: string[] }`
    - `browserStorage(): Pick<Storage, "length" | "key" | "getItem" | "removeItem"> | undefined`
    - `importLegacy(windowId: string, storage: … | undefined, importDrafts: (window: string, drafts: LegacyDraft[], activeRoot: string | null) => Promise<void>): Promise<string | null>`, which returns the banner message on failure.

- [ ] **Step 1: Write the failing tests**

Create `src/lib/tabTitle.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { firstLine, tabTitle } from "./tabTitle";

describe("firstLine", () => {
  // The same strings as sessions::tests::first_line_matches_the_frontend in Rust.
  it("is the first non-empty line, trimmed, at most 48 characters", () => {
    expect(firstLine("")).toBe("");
    expect(firstLine("\n  \n\t Retry failed checkout payments  \nsecond line")).toBe("Retry failed checkout payments");
    expect(firstLine("line one\r\nline two")).toBe("line one");
    expect(firstLine("a".repeat(60))).toBe("a".repeat(48));
    expect(firstLine(`${"x".repeat(47)} yz`)).toBe("x".repeat(47));
    expect(firstLine("😀".repeat(50))).toBe("😀".repeat(48));
    // Unicode White_Space is trimmed (NEL, no-break and ideographic spaces); a BOM is not whitespace.
    expect(firstLine("\u0085 Retry payments　\nmore")).toBe("Retry payments");
    expect(firstLine("﻿Retry")).toBe("﻿Retry");
  });
});

describe("tabTitle", () => {
  it("is the title, else the first line, else Untitled", () => {
    expect(tabTitle({ title: "Checkout retry", firstLine: "Retry failed" })).toBe("Checkout retry");
    expect(tabTitle({ title: null, firstLine: "Retry failed" })).toBe("Retry failed");
    expect(tabTitle({ title: null, firstLine: "" })).toBe("Untitled");
  });
});
```

Create `src/lib/windowId.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";

const getCurrentWindow = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow }));

import { currentWindowId } from "./windowId";

describe("currentWindowId", () => {
  it("is the window's label when it is a workspace id", () => {
    getCurrentWindow.mockReturnValue({ label: "w-3" });
    expect(currentWindowId()).toBe("w-3");
  });

  it("is w-1 for the static main window, and outside Tauri", () => {
    getCurrentWindow.mockReturnValue({ label: "main" });
    expect(currentWindowId()).toBe("w-1");
    getCurrentWindow.mockImplementation(() => {
      throw new TypeError("no Tauri internals");
    });
    expect(currentWindowId()).toBe("w-1");
  });
});
```

Create `src/lib/events.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { onBriefStatus, onWorkspaceChanged } from "./events";

/** Calls the handler the n-th `listen` was given, as Tauri would. */
const deliver = (n: number, payload: unknown) => listen.mock.calls[n][1]({ event: listen.mock.calls[n][0], id: 1, payload });

beforeEach(() => {
  listen.mockReset();
  listen.mockResolvedValue(() => {});
});

describe("events", () => {
  it("brief-status hands its payload to the handler", async () => {
    const got: unknown[] = [];
    const unlisten = await onBriefStatus((e) => got.push(e));
    expect(typeof unlisten).toBe("function");
    expect(listen.mock.calls[0][0]).toBe("brief-status");
    deliver(0, { root: "/p", state: "done", message: null });
    expect(got).toEqual([{ root: "/p", state: "done", message: null }]);
  });

  it("workspace-changed hands its payload to the handler", async () => {
    const got: unknown[] = [];
    await onWorkspaceChanged((e) => got.push(e));
    expect(listen.mock.calls[0][0]).toBe("workspace-changed");
    deliver(0, { windows: ["w-1"] });
    expect(got).toEqual([{ windows: ["w-1"] }]);
  });
});
```

Create `src/lib/legacyDrafts.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import type { LegacyDraft } from "./ipc";
import { collectLegacyDrafts, importLegacy } from "./legacyDrafts";

/** A localStorage stand-in whose contents the test can read. */
function storage(entries: Record<string, string>) {
  const m = new Map(Object.entries(entries));
  return {
    m,
    get length() {
      return m.size;
    },
    key: (i: number) => [...m.keys()][i] ?? null,
    getItem: (k: string) => m.get(k) ?? null,
    removeItem: (k: string) => void m.delete(k),
  };
}

const importer = () => vi.fn((_window: string, _drafts: LegacyDraft[], _activeRoot: string | null) => Promise.resolve());

describe("collectLegacyDrafts", () => {
  it("collects every non-empty draft; the root 'none' is no project", () => {
    const s = storage({
      "livebrainstorm.draft./b": "b idea",
      "livebrainstorm.draft.none": "loose idea",
      "livebrainstorm.draft./a": "  ",
      "livebrainstorm.lastProject": "/b",
      other: "x",
    });
    expect(collectLegacyDrafts(s)).toEqual({
      drafts: [
        { root: "/b", draft: "b idea" },
        { root: null, draft: "loose idea" },
      ],
      activeRoot: "/b",
      keys: ["livebrainstorm.draft./b", "livebrainstorm.draft.none", "livebrainstorm.draft./a", "livebrainstorm.lastProject"],
    });
  });
});

describe("importLegacy", () => {
  it("imports, then removes only the old build's keys", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea", "livebrainstorm.lastProject": "/a", other: "x" });
    const importDrafts = importer();
    expect(await importLegacy("w-1", s, importDrafts)).toBeNull();
    expect(importDrafts).toHaveBeenCalledWith("w-1", [{ root: "/a", draft: "a idea" }], "/a");
    expect([...s.m.keys()]).toEqual(["other"]);
  });

  it("keeps the keys when the import fails, and says so", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea" });
    const failed = await importLegacy("w-1", s, () => Promise.reject(new Error("disk full")));
    expect(failed).toBe("Couldn't bring over your earlier drafts: disk full");
    expect(s.m.has("livebrainstorm.draft./a")).toBe(true);
  });

  it("only empty drafts: nothing to import, and the keys go", async () => {
    const s = storage({ "livebrainstorm.draft./a": "  " });
    const importDrafts = importer();
    expect(await importLegacy("w-1", s, importDrafts)).toBeNull();
    expect(importDrafts).not.toHaveBeenCalled();
    expect(s.m.size).toBe(0);
  });

  it("does nothing without storage or keys", async () => {
    const importDrafts = importer();
    expect(await importLegacy("w-1", undefined, importDrafts)).toBeNull();
    expect(await importLegacy("w-1", storage({ other: "x" }), importDrafts)).toBeNull();
    expect(importDrafts).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/tabTitle.test.ts src/lib/windowId.test.ts src/lib/events.test.ts src/lib/legacyDrafts.test.ts`
Expected: FAIL with `Failed to resolve import "./tabTitle"` (and likewise for `./windowId`, `./events` and `./legacyDrafts`).

- [ ] **Step 3: Session types and wrappers in `ipc.ts`**

In `src/lib/ipc.ts`, add after the `ProjectSummary` interface:

```ts
/** One saved analysis (`sessions::Snapshot`). `at` is unix seconds; `context` is the run's `analysisContextKey`. */
export interface Snapshot {
  id: number;
  at: number;
  draft: string;
  context: string;
  markdown: string;
  partial: boolean;
  notices: string[];
  checks: CheckResult[];
  /** The claim count from the last `checking` event. Fewer checks, without `checksStopped`, means an edit cut the run off. */
  checksTotal: number | null;
  /** The Faults count from the same event (`total`); null in files saved before it existed. */
  checksFaults: number | null;
  checksStopped: boolean;
}

/** One tab (`sessions::Tab`): its draft and its last analyses, newest first (at most 5). */
export interface Tab {
  id: string;
  root: string | null;
  title: string | null;
  createdAt: number;
  updatedAt: number;
  closedAt: number | null;
  draft: string;
  nextSnapshot: number;
  history: Snapshot[];
}

/** What lists show (`sessions::TabSummary`): no draft body or history. */
export interface TabSummary {
  id: string;
  root: string | null;
  title: string | null;
  /** The draft's first non-empty line, at most 48 characters (`firstLine` in tabTitle.ts). */
  firstLine: string;
  updatedAt: number;
  closedAt: number | null;
}

/** One window's tabs, its full active tab, and the shared closed list (`sessions::WindowView`). */
export interface WindowView {
  window: string;
  tabs: TabSummary[];
  active: Tab | null;
  closed: TabSummary[];
  /** The first launch of this build: import the old localStorage drafts. */
  fresh: boolean;
  /** Files kept as `.bad` since the app started. */
  unreadable: string[];
}

/** A draft the old build kept in localStorage (`sessions::LegacyDraft`). */
export interface LegacyDraft {
  root: string | null;
  draft: string;
}
```

In `api`, add after the `openProject` entry:

```ts
  projectSummary: (root: string) => invoke<ProjectSummary>("project_summary", { root }),
```

and after the `cancelAnalysis` entry:

```ts

  listWorkspace: (window: string) => invoke<WindowView>("list_workspace", { window }),
  getTab: (tabId: string) => invoke<Tab>("get_tab", { tabId }),
  createTab: (window: string, root: string | null) => invoke<Tab>("create_tab", { window, root }),
  saveDraft: (tabId: string, draft: string) => invoke<void>("save_draft", { tabId, draft }),
  renameTab: (tabId: string, title: string | null) => invoke<void>("rename_tab", { tabId, title }),
  activateTab: (window: string, tabId: string) => invoke<void>("activate_tab", { window, tabId }),
  closeTab: (tabId: string) => invoke<void>("close_tab", { tabId }),
  reopenTab: (tabId: string, window: string) => invoke<void>("reopen_tab", { tabId, window }),
  deleteTab: (tabId: string) => invoke<void>("delete_tab", { tabId }),
  importLegacyDrafts: (window: string, drafts: LegacyDraft[], activeRoot: string | null) =>
    invoke<void>("import_legacy_drafts", { window, drafts, activeRoot }),
```

- [ ] **Step 4: Create `src/lib/tabTitle.ts`**

```ts
// Tab titles. `firstLine` mirrors `sessions::first_line` in Rust, a
// cross-language contract: both test suites pin the same strings.

/** Characters (Unicode scalars, not UTF-16 units) a title takes from the draft. */
export const TITLE_CHARS = 48;

// Unicode White_Space, which Rust's `str::trim` uses. JS's own `trim` differs
// on two characters: it strips U+FEFF and keeps U+0085.
const LEADING = /^\p{White_Space}+/u;
const TRAILING = /\p{White_Space}+$/u;

/** The draft's first non-empty line, trimmed, at most TITLE_CHARS characters. */
export function firstLine(draft: string): string {
  for (const line of draft.split("\n")) {
    const t = line.replace(LEADING, "").replace(TRAILING, "");
    if (t) return Array.from(t).slice(0, TITLE_CHARS).join("").replace(TRAILING, "");
  }
  return "";
}

/** The explicit title, else the draft's first line, else "Untitled". */
export function tabTitle(t: { title: string | null; firstLine: string }): string {
  return t.title || t.firstLine || "Untitled";
}
```

- [ ] **Step 5: Create `src/lib/windowId.ts`**

```ts
// This window's workspace id. Windows are labelled with their id (`w-<n>`).
// A window with any other label (the one tauri.conf.json declares, while it
// declares one) is workspace window `w-1`.
import { getCurrentWindow } from "@tauri-apps/api/window";

export function currentWindowId(): string {
  let label = "";
  try {
    label = getCurrentWindow().label;
  } catch {
    // Outside Tauri (tests) there is no window.
  }
  return /^w-/.test(label) ? label : "w-1";
}
```

- [ ] **Step 6: Create `src/lib/events.ts`**

```ts
// Events every window receives from the shell (`app.emit`), unlike a
// command's own Channel. Names and payloads mirror src-tauri/src/events.rs.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A brief build for `root` started, finished or failed (`events::BriefStatus`). */
export interface BriefStatusEvent {
  root: string;
  state: "building" | "done" | "error";
  message: string | null;
}

/** A session changed (`events::WorkspaceChanged`): these windows' tab lists, and the shared closed list. */
export interface WorkspaceChangedEvent {
  windows: string[];
}

export function onBriefStatus(fn: (e: BriefStatusEvent) => void): Promise<UnlistenFn> {
  return listen<BriefStatusEvent>("brief-status", (e) => fn(e.payload));
}

export function onWorkspaceChanged(fn: (e: WorkspaceChangedEvent) => void): Promise<UnlistenFn> {
  return listen<WorkspaceChangedEvent>("workspace-changed", (e) => fn(e.payload));
}
```

- [ ] **Step 7: Create `src/lib/legacyDrafts.ts`**

```ts
// The one-time upgrade from the build that kept one draft per project in
// localStorage: each non-empty draft becomes a tab. The keys go only once the
// import has succeeded, so a failure loses nothing.
import { errorMessage, type LegacyDraft } from "./ipc";

const DRAFT_PREFIX = "livebrainstorm.draft.";
const LAST_PROJECT = "livebrainstorm.lastProject";

type LegacyStorage = Pick<Storage, "length" | "key" | "getItem" | "removeItem">;

/** Every saved draft (the root "none" is no project), the last project, and every key to remove afterwards. */
export function collectLegacyDrafts(storage: Pick<Storage, "length" | "key" | "getItem">): {
  drafts: LegacyDraft[];
  activeRoot: string | null;
  keys: string[];
} {
  const drafts: LegacyDraft[] = [];
  const keys: string[] = [];
  for (let i = 0; i < storage.length; i++) {
    const key = storage.key(i);
    if (key === null || !key.startsWith(DRAFT_PREFIX)) continue;
    keys.push(key);
    const draft = storage.getItem(key) ?? "";
    const root = key.slice(DRAFT_PREFIX.length);
    if (draft.trim()) drafts.push({ root: root === "none" ? null : root, draft });
  }
  const activeRoot = storage.getItem(LAST_PROJECT);
  if (activeRoot !== null) keys.push(LAST_PROJECT);
  // localStorage's key order isn't defined: projects by path, then the no-project draft.
  drafts.sort((a, b) => (a.root === null ? 1 : b.root === null ? -1 : a.root.localeCompare(b.root)));
  return { drafts, activeRoot, keys };
}

/** localStorage, or undefined where it can't be used. */
export function browserStorage(): LegacyStorage | undefined {
  try {
    return typeof localStorage === "undefined" ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

/** Imports the old drafts into this window. Returns the message to show if that failed, else null. */
export async function importLegacy(
  windowId: string,
  storage: LegacyStorage | undefined,
  importDrafts: (window: string, drafts: LegacyDraft[], activeRoot: string | null) => Promise<void>,
): Promise<string | null> {
  if (!storage) return null;
  const legacy = collectLegacyDrafts(storage);
  if (legacy.keys.length === 0) return null;
  try {
    if (legacy.drafts.length > 0) await importDrafts(windowId, legacy.drafts, legacy.activeRoot);
  } catch (e) {
    return `Couldn't bring over your earlier drafts: ${errorMessage(e)}`; // the keys stay, so nothing is lost
  }
  for (const key of legacy.keys) {
    try {
      storage.removeItem(key);
    } catch {
      // Left behind. Harmless: the workspace is no longer fresh, so it never imports again.
    }
  }
  return null;
}
```

- [ ] **Step 8: Run the tests and the typecheck**

Run: `npx vitest run src/lib/tabTitle.test.ts src/lib/windowId.test.ts src/lib/events.test.ts src/lib/legacyDrafts.test.ts && npm test && npm run typecheck`
Expected: the 11 new tests PASS (2 in `tabTitle`, 2 in `windowId`, 2 in `events`, 5 in `legacyDrafts`), the whole suite passes, and there are no type errors.

- [ ] **Step 9: Commit**

```bash
git add src/lib/ipc.ts src/lib/tabTitle.ts src/lib/tabTitle.test.ts src/lib/windowId.ts src/lib/windowId.test.ts src/lib/events.ts src/lib/events.test.ts src/lib/legacyDrafts.ts src/lib/legacyDrafts.test.ts
git commit -m "feat(ui): session types and wrappers, brief and workspace events, window id, tab titles, legacy draft import

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 8: `TabSession`: one tab's live analysis, outside React

**Files:**
- Create: `src/lib/tabSession.ts`, `src/lib/useTabSession.ts`
- Test: `src/lib/tabSession.test.ts`, `src/lib/useTabSession.test.ts`

**Interfaces:**
- Consumes:
  - `applyEvent`, `initialAnalysis`, `lastAnalyzedAfter`, `previousDraftFor`, `startAnalysis`, `AnalysisState` and `Analyzed` from `src/lib/analysisState.ts`, unchanged.
  - `errorMessage`, `Snapshot`, `Tab` and `UiEvent` from `src/lib/ipc.ts` (Task 7).
- Produces:
  ```ts
  // src/lib/tabSession.ts
  export const MIN_CHARS = 24;
  export const normalize: (s: string) => string;
  export interface TabSessionDeps {
    analyze: (tabId: string, draft: string, previousDraft: string | null, instructions: string, context: string,
              onEvent: (e: UiEvent) => void) => Promise<void>;
    saveDraft: (tabId: string, draft: string) => void;   // caller wires createSaver (500 ms) keyed by tab id
    debounceMs: () => number;
  }
  export interface TabView {                // a new object only when something changed (useSyncExternalStore)
    draft: string;
    analysis: AnalysisState;
    tooShort: boolean;
    stale: boolean;
    contextDirty: boolean;                  // always false in Phase 1
    viewing: number;                        // always 0 in Phase 1
    history: Snapshot[];                    // restore's copy in Phase 1
  }
  export class TabSession {
    constructor(tab: Tab, deps: TabSessionDeps);   // calls restore(tab)
    readonly id: string;
    readonly root: string | null;
    getState(): TabView;
    subscribe(fn: () => void): () => void;
    restore(tab: Tab): void;
    setDraft(text: string): void;
    setContext(key: string, instructions: string): void;
    activate(): void;      // Phase 1: nothing to defer, since one tab is shown
    deactivate(): void;    // leaves any timer or run alone
    rerun(): void;
    dispose(): void;
  }
  // src/lib/useTabSession.ts
  export function useTabSession(s: TabSession | null): TabView | null;
  ```
  Phase 2 replaces `tabSession.ts` and adds `view(index)`, a history refetch after `done`, and the hidden-tab context rule. This task's tests must keep passing against that replacement, so they only pin behaviour that both versions share.

- [ ] **Step 1: Write the failing tests**

Create `src/lib/tabSession.test.ts`:

```ts
// TabSession on the one tab the window shows: the pause, the gate's
// comparison point, restoring saved notes, and dropping superseded events.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CheckResult, Snapshot, Tab, UiEvent } from "./ipc";
import { TabSession, type TabSessionDeps } from "./tabSession";

const DRAFT = "Retry failed checkout payments with backoff in the worker";

const snap = (over: Partial<Snapshot> = {}): Snapshot => ({
  id: 1,
  at: 1760403600,
  draft: DRAFT,
  context: "/p|1|",
  markdown: "## Intent\n- retry",
  partial: false,
  notices: [],
  checks: [],
  checksTotal: null,
  checksFaults: null,
  checksStopped: false,
  ...over,
});

const tab = (over: Partial<Tab> = {}): Tab => ({
  id: "t-1",
  root: "/p",
  title: null,
  createdAt: 1,
  updatedAt: 1,
  closedAt: null,
  draft: DRAFT,
  nextSnapshot: 2,
  history: [snap()],
  ...over,
});

interface Call {
  tabId: string;
  draft: string;
  previousDraft: string | null;
  instructions: string;
  context: string;
  emit: (e: UiEvent) => void;
}

function harness(t: Tab, analyze?: TabSessionDeps["analyze"]) {
  const calls: Call[] = [];
  const saveDraft = vi.fn();
  const deps: TabSessionDeps = {
    analyze:
      analyze ??
      ((tabId, draft, previousDraft, instructions, context, onEvent) => {
        calls.push({ tabId, draft, previousDraft, instructions, context, emit: onEvent });
        return Promise.resolve();
      }),
    saveDraft,
    debounceMs: () => 1000,
  };
  return { s: new TabSession(t, deps), calls, saveDraft };
}

/** Lets promise callbacks run; fake timers don't touch microtasks. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("the pause", () => {
  it("analyses a moment after typing stops, with the tab, its instructions and its context", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setContext("/p|1|", "Focus on retries.");
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(999);
    expect(calls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ tabId: "t-1", draft: DRAFT, previousDraft: null, instructions: "Focus on retries.", context: "/p|1|" });
    expect(s.getState().analysis.status).toBe("thinking");
  });

  it("saves every edit, and nothing when the text is unchanged", () => {
    const { s, saveDraft } = harness(tab({ draft: "", history: [] }));
    s.setDraft("a");
    s.setDraft("ab");
    s.setDraft("ab");
    expect(saveDraft.mock.calls).toEqual([
      ["t-1", "a"],
      ["t-1", "ab"],
    ]);
  });

  it("a draft under 24 characters never runs", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft("a short idea");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().tooShort).toBe(true);
  });

  it("whitespace-only edits don't re-send the same draft", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    s.setDraft(`${DRAFT}  \n`);
    vi.advanceTimersByTime(5000);
    expect(calls).toHaveLength(1);
  });

  it("dispose stops a pending pause", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    s.dispose();
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
  });
});

describe("restore", () => {
  it("shows the latest analysis and starts no run when the draft matches it", () => {
    const { s, calls } = harness(tab({ draft: `  ${DRAFT}\n` }));
    s.setContext("/p|1|", "");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    const v = s.getState();
    expect(v.analysis.status).toBe("done");
    expect(v.analysis.markdown).toBe("## Intent\n- retry");
    expect(v.stale).toBe(false);
    expect(v.viewing).toBe(0);
    expect(v.history.map((h) => h.id)).toEqual([1]);
  });

  it("an edit since the latest analysis shows as stale, and the next pause compares against it", () => {
    const { s, calls } = harness(tab({ draft: `${DRAFT}, and alert` }));
    s.setContext("/p|1|", "");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]); // relaunching never starts a run by itself
    expect(s.getState().stale).toBe(true);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBe(DRAFT);
  });

  it("a partial latest analysis is never the gate's comparison point", () => {
    const { s, calls } = harness(tab({ history: [snap({ partial: true })] }));
    s.setContext("/p|1|", "");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBeNull();
  });

  it("a new context re-reads the same draft, past the gate", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|2|", "Focus on retries.");
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null, context: "/p|2|", instructions: "Focus on retries." });
  });

  it("checks an edit cut off read as stopped, as they did live", () => {
    const check: CheckResult = { claim: "`retry` is missing", verdict: "confirmed", confidence: 0.9, path: "src/pay.ts", line: 3 };
    const { s } = harness(tab({ history: [snap({ checks: [check], checksTotal: 3, checksFaults: 5 })] }));
    const a = s.getState().analysis;
    expect(a.checking).toEqual({ count: 3, total: 5 }); // "1 of 5 faults checked", as it read live
    expect(a.checks).toEqual([check]);
    expect(a.checksStopped).toBe(true);
    expect(a.notices).toEqual(["Checks stopped when you edited."]);
  });

  it("starts idle with no saved analysis", () => {
    const { s } = harness(tab({ draft: "", history: [] }));
    expect(s.getState().analysis.status).toBe("idle");
    expect(s.getState().history).toEqual([]);
  });
});

describe("runs", () => {
  it("started clears the old notes, text streams in, and done is current", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "started" });
    calls[0].emit({ type: "text", text: "## Intent\n- a", lane: 0 });
    expect(s.getState().analysis.markdown).toBe("## Intent\n- a");
    calls[0].emit({ type: "done", text: "## Intent\n- a" });
    expect(s.getState().analysis.status).toBe("done");
    expect(s.getState().stale).toBe(false);
  });

  it("drops events from a superseded request", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(`${DRAFT}, version one`);
    vi.advanceTimersByTime(1000);
    s.setDraft(`${DRAFT}, version two`);
    vi.advanceTimersByTime(1000);
    calls[1].emit({ type: "started" });
    calls[0].emit({ type: "text", text: "## Intent\n- stale", lane: 0 });
    expect(s.getState().analysis.markdown).toBe("");
  });

  it("try again re-analyses the same draft, past the gate", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    s.rerun();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null });
  });

  it("a failed call shows its error", async () => {
    const { s } = harness(tab({ draft: "", history: [] }), () => Promise.reject(new Error("Claude Code isn't installed.")));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    await settle();
    expect(s.getState().analysis).toMatchObject({ status: "error", error: "Claude Code isn't installed." });
  });

  it("subscribers hear each change, with a new state object", () => {
    const { s } = harness(tab({ draft: "", history: [] }));
    const before = s.getState();
    const heard = vi.fn();
    const stop = s.subscribe(heard);
    s.setDraft("a");
    expect(heard).toHaveBeenCalledTimes(1);
    expect(s.getState()).not.toBe(before);
    expect(s.getState()).toBe(s.getState());
    stop();
    s.setDraft("ab");
    expect(heard).toHaveBeenCalledTimes(1);
  });
});
```

Create `src/lib/useTabSession.test.ts`:

```ts
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { Tab } from "./ipc";
import { TabSession } from "./tabSession";
import { useTabSession } from "./useTabSession";

const tab: Tab = {
  id: "t-1",
  root: null,
  title: null,
  createdAt: 1,
  updatedAt: 1,
  closedAt: null,
  draft: "an idea about retries",
  nextSnapshot: 1,
  history: [],
};

function Probe({ s }: { s: TabSession | null }) {
  const view = useTabSession(s);
  return createElement("p", null, view ? `${view.draft} · ${view.analysis.status}` : "no tab");
}

describe("useTabSession", () => {
  it("reads the session's state, or null without a session", () => {
    const s = new TabSession(tab, { analyze: vi.fn(), saveDraft: vi.fn(), debounceMs: () => 1000 });
    expect(renderToString(createElement(Probe, { s }))).toBe("<p>an idea about retries · idle</p>");
    expect(renderToString(createElement(Probe, { s: null }))).toBe("<p>no tab</p>");
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/tabSession.test.ts src/lib/useTabSession.test.ts`
Expected: FAIL with `Failed to resolve import "./tabSession"` and `"./useTabSession"`.

- [ ] **Step 3: Create `src/lib/tabSession.ts`**

```ts
// One tab's live analysis, kept outside React so its pause timer and its run
// don't depend on a component staying mounted. Like a colleague reading over
// your shoulder, it waits until you pause, then comments; if you keep going,
// the last notes stay visible but marked out of date. If the edit didn't
// change the idea, the backend skips (Jev gate) and the notes stay. Old
// responses are dropped by request id; the backend also aborts the tab's
// superseded run (its task slots are keyed by tab id).
import {
  applyEvent,
  initialAnalysis,
  lastAnalyzedAfter,
  previousDraftFor,
  startAnalysis,
  type AnalysisState,
  type Analyzed,
} from "./analysisState";
import { errorMessage, type Snapshot, type Tab, type UiEvent } from "./ipc";

export const MIN_CHARS = 24;
export const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/** What a run's fact-check shows when an edit cut it off, live or restored. */
const CHECKS_CUT_OFF = "Checks stopped when you edited.";

export interface TabSessionDeps {
  analyze: (
    tabId: string,
    draft: string,
    previousDraft: string | null,
    instructions: string,
    context: string,
    onEvent: (e: UiEvent) => void,
  ) => Promise<void>;
  /** The caller wires createSaver (500 ms) keyed by tab id. */
  saveDraft: (tabId: string, draft: string) => void;
  debounceMs: () => number;
}

/** What React renders. A new object only when something changed (useSyncExternalStore). */
export interface TabView {
  draft: string;
  analysis: AnalysisState;
  tooShort: boolean;
  stale: boolean;
  /** A context change waiting for the tab to be shown. Always false while one tab is shown. */
  contextDirty: boolean;
  /** Which saved analysis is shown; 0 is the latest. */
  viewing: number;
  /** Newest first, at most 5 (sessions::HISTORY_LEN). */
  history: Snapshot[];
}

/**
 * A saved analysis as the notes pane shows it. Fewer checks than
 * `checksTotal`, without `checksStopped`, means an edit cut the run off: it
 * reads as stopped, as it did live.
 */
function fromSnapshot(s: Snapshot): AnalysisState {
  const cutOff = !s.checksStopped && s.checksTotal !== null && s.checks.length < s.checksTotal;
  return {
    status: "done",
    lanes: [],
    markdown: s.markdown,
    error: null,
    analyzedDraft: s.draft,
    checking: s.checksTotal === null ? null : { count: s.checksTotal, total: s.checksFaults ?? s.checksTotal },
    checks: s.checks,
    notices: cutOff && !s.notices.includes(CHECKS_CUT_OFF) ? [...s.notices, CHECKS_CUT_OFF] : s.notices,
    checksStopped: s.checksStopped || cutOff,
    partial: s.partial,
  };
}

export class TabSession {
  readonly id: string;
  readonly root: string | null;
  private readonly deps: TabSessionDeps;
  private listeners = new Set<() => void>();
  private state!: TabView;
  private draft = "";
  private analysis: AnalysisState = initialAnalysis;
  private history: Snapshot[] = [];
  private context = "";
  private instructions = "";
  private lastSent = "";
  private lastAnalyzed: Analyzed | null = null;
  private disposed = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private requestId = 0;

  constructor(tab: Tab, deps: TabSessionDeps) {
    this.id = tab.id;
    this.root = tab.root;
    this.deps = deps;
    this.restore(tab);
  }

  getState(): TabView {
    return this.state;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  /**
   * Starts from the tab's latest analysis. A draft that matches it starts no
   * run, and restoring never starts one by itself; partial notes are never
   * the gate's comparison point.
   */
  restore(tab: Tab): void {
    this.clearTimer();
    this.requestId++; // events from a run before the restore no longer apply
    const latest = tab.history[0];
    this.draft = tab.draft;
    this.history = tab.history;
    this.analysis = latest ? fromSnapshot(latest) : initialAnalysis;
    this.context = latest?.context ?? "";
    this.lastSent = latest && normalize(latest.draft) === normalize(tab.draft) ? normalize(tab.draft) : "";
    this.lastAnalyzed = latest && !latest.partial ? { draft: latest.draft, context: latest.context } : null;
    this.emit();
  }

  setDraft(text: string): void {
    if (text === this.draft || this.disposed) return;
    this.draft = text;
    this.deps.saveDraft(this.id, text);
    this.schedule();
    this.emit();
  }

  /**
   * The window calls this once it has loaded the tab's project summary and
   * instructions. A new key (another brief, other instructions) re-reads the
   * same draft, past the gate.
   */
  setContext(key: string, instructions: string): void {
    this.instructions = instructions;
    if (key === this.context || this.disposed) return;
    this.context = key;
    this.lastSent = "";
    this.lastAnalyzed = null;
    this.schedule();
  }

  /** The window shows one tab at a time, so nothing waits for it: setContext always re-reads. */
  activate(): void {}

  /** Leaves any timer or run alone: a hidden tab keeps analysing. */
  deactivate(): void {}

  /** "Try again": always re-analyses, past the gate. */
  rerun(): void {
    if (!this.disposed) this.run(this.draft, true);
  }

  /** Clears the timer and drops late events. close_tab cancels the backend run. */
  dispose(): void {
    this.disposed = true;
    this.clearTimer();
    this.requestId++;
    this.listeners.clear();
  }

  private schedule() {
    this.clearTimer();
    const norm = normalize(this.draft);
    if (norm.length < MIN_CHARS || norm === this.lastSent) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.run(this.draft, false);
    }, this.deps.debounceMs());
  }

  private clearTimer() {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private run(text: string, rerun: boolean) {
    this.clearTimer();
    const id = ++this.requestId;
    const ctx = this.context;
    this.lastSent = normalize(text);
    this.analysis = startAnalysis(this.analysis, text);
    this.emit();
    const mine = () => id === this.requestId;
    this.deps
      .analyze(this.id, text, previousDraftFor(this.lastAnalyzed, rerun, ctx), this.instructions, ctx, (ev) => {
        if (!mine()) return;
        this.lastAnalyzed = lastAnalyzedAfter(this.lastAnalyzed, ev, text, ctx);
        this.analysis = applyEvent(this.analysis, ev);
        this.emit();
      })
      .catch((e) => {
        if (!mine()) return;
        this.analysis = { ...this.analysis, status: "error", error: errorMessage(e) };
        this.emit();
      });
  }

  private emit() {
    const norm = normalize(this.draft);
    this.state = {
      draft: this.draft,
      analysis: this.analysis,
      tooShort: norm.length < MIN_CHARS,
      stale: this.analysis.status !== "idle" && normalize(this.analysis.analyzedDraft) !== norm,
      contextDirty: false,
      viewing: 0,
      history: this.history,
    };
    this.listeners.forEach((l) => l());
  }
}
```

- [ ] **Step 4: Create `src/lib/useTabSession.ts`**

```ts
import { useCallback, useSyncExternalStore } from "react";
import type { TabSession, TabView } from "./tabSession";

/** The session's state, re-rendering on each change; null without a session. */
export function useTabSession(s: TabSession | null): TabView | null {
  const subscribe = useCallback((onChange: () => void) => (s ? s.subscribe(onChange) : () => {}), [s]);
  const read = useCallback(() => (s ? s.getState() : null), [s]);
  // The same read serves as the server snapshot: there's no server render, but tests render to a string.
  return useSyncExternalStore(subscribe, read, read);
}
```

- [ ] **Step 5: Run the tests and the typecheck**

Run: `npx vitest run src/lib/tabSession.test.ts src/lib/useTabSession.test.ts && npm test && npm run typecheck`
Expected: the 17 new tests PASS (16 in `tabSession.test.ts`, 1 in `useTabSession.test.ts`), the whole suite passes, and there are no type errors.

- [ ] **Step 6: Commit**

```bash
git add src/lib/tabSession.ts src/lib/tabSession.test.ts src/lib/useTabSession.ts src/lib/useTabSession.test.ts
git commit -m "feat(ui): TabSession runs one tab's analysis outside React and restores its saved notes

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 9: The single-pane window runs on its active tab

**Files:**
- Create: `src/lib/tabContext.ts`
- Test: `src/lib/tabContext.test.ts`
- Modify: `src/lib/ipc.ts` (`buildBrief`, `analyze`, `cancelAnalysis`)
- Modify: `src/components/NotesPane.tsx:4,8` (the `Analysis` type)
- Replace: `src/App.tsx`
- Delete: `src/lib/useLiveAnalysis.ts`

**Interfaces:**
- Consumes:
  - Task 7:
    - `api.projectSummary`, `api.listWorkspace`, `api.getTab`, `api.createTab`, `api.saveDraft`, `api.activateTab` and `api.importLegacyDrafts`, with the types `Tab`, `WindowView` and `LegacyDraft`.
    - `onBriefStatus(fn: (e: { root: string; state: "building" | "done" | "error"; message: string | null }) => void): Promise<UnlistenFn>`
    - `currentWindowId(): string`
    - `browserStorage()` and `importLegacy(windowId, storage, importDrafts): Promise<string | null>`
  - Task 8: `TabSession`, `TabSessionDeps`, `TabView` and `useTabSession(s: TabSession | null): TabView | null`.
  - Unchanged:
    - `createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void): { schedule(key, value): void; flush(): Promise<void> }` (`src/lib/saver.ts`).
    - From `src/lib/instructions.ts`: `analysisContextKey(root: string | undefined, briefCreatedAt: number | undefined, instructions: string): string` (`${root ?? ""}|${briefCreatedAt ?? 0}|${instructions.trim()}`), `EMPTY_INSTRUCTIONS` and `instructionsLabel`.
    - `activityLine` (`src/lib/brief.ts`), `providerLabel` (`src/lib/providerOptions.ts`), `initialAnalysis` (`src/lib/analysisState.ts`).
    - `TopBar`, `DraftPane`, `BriefDrawer`, `InstructionsDrawer`, `TemplateList` and `SettingsDialog`, with today's props.
- Produces:
  - `src/lib/tabContext.ts`:
    - `export interface LoadedProject { root: string; summary: ProjectSummary | null; error: string | null }`
    - `export interface LoadedInstructions { root: string; current: string }`
    - `export function tabContext(root: string | null, project: LoadedProject | null, instructions: LoadedInstructions | null): { key: string; instructions: string } | null`
  - `api.buildBrief(root: string, onEvent): Promise<void>`
  - `api.analyze(tabId: string, draft: string, previousDraft: string | null, instructions: string, context: string, onEvent: (e: UiEvent) => void): Promise<void>`
  - `api.cancelAnalysis(tabId: string): Promise<void>`
  - `NotesPane` props: `{ analysis: AnalysisState & { stale: boolean; rerun: () => void }; hasProject: boolean; briefMissing: boolean; onBuildBrief: () => void }`. Its `RuleState` and `RULE` lines stay exactly as they are; Phase 2's Task 1 moves them into `ui.tsx`.
  - `App.tsx`:
    - The window drives one `TabSession` for its active tab.
    - On a fresh workspace it imports the `localStorage` drafts, then removes those keys.
    - *Open a project* goes to this window's tab for that root, creating one if there is none.
    - A window with no tabs gets one with no project, so it always takes a draft.

- [ ] **Step 1: Write the failing test**

Create `src/lib/tabContext.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { ProjectSummary } from "./ipc";
import { tabContext } from "./tabContext";

const summary = (createdAt: number | null): ProjectSummary => ({
  name: "p",
  root: "/p",
  indexedFiles: 1,
  totalFiles: 1,
  briefStatus: createdAt ? "fresh" : "missing",
  brief: createdAt ? { root: "/p", fingerprint: "f", text: "brief", createdAt, source: "test" } : null,
});

describe("tabContext", () => {
  it("waits for both the project summary and the instructions", () => {
    expect(tabContext("/p", { root: "/p", summary: null, error: null }, { root: "/p", current: "Focus." })).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, null)).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, { root: "/p", current: " Focus. " })).toEqual({
      key: "/p|7|Focus.",
      instructions: " Focus. ",
    });
  });

  it("never mixes in another project's data", () => {
    expect(tabContext("/p", { root: "/q", summary: summary(7), error: null }, { root: "/p", current: "" })).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, { root: "/q", current: "" })).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, { root: "", current: "" })).toBeNull();
  });

  it("a project that couldn't be read analyses without its brief", () => {
    const gone = { root: "/p", summary: null, error: "That folder no longer exists. Open it again from its new place." };
    expect(tabContext("/p", gone, { root: "/p", current: "" })).toEqual({ key: "/p|0|", instructions: "" });
  });

  it("a tab with no project needs only the no-project instructions", () => {
    expect(tabContext(null, null, { root: "", current: "Be brief." })).toEqual({ key: "|0|Be brief.", instructions: "Be brief." });
    expect(tabContext(null, null, { root: "/p", current: "" })).toBeNull();
  });

  it("a project with no brief yet uses 0, as a saved analysis did", () => {
    expect(tabContext("/p", { root: "/p", summary: summary(null), error: null }, { root: "/p", current: "" })).toEqual({
      key: "/p|0|",
      instructions: "",
    });
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/lib/tabContext.test.ts`
Expected: FAIL with `Failed to resolve import "./tabContext"`.

- [ ] **Step 3: Create `src/lib/tabContext.ts`**

```ts
// When the shown tab's analysis context is ready, and what it is. The context
// key comes from the project's brief and the instructions, so it is computed
// only once both have loaded: a key computed earlier ("root|0|") would differ
// from the saved notes' and start a spurious run at launch.
import type { ProjectSummary } from "./ipc";
import { analysisContextKey } from "./instructions";

/** The shown tab's project as the window loaded it: its summary, or why it couldn't be read. */
export interface LoadedProject {
  root: string;
  summary: ProjectSummary | null;
  error: string | null;
}

/** The current instructions loaded for one root ("" for no project). */
export interface LoadedInstructions {
  root: string;
  current: string;
}

/**
 * The tab's context key and instructions, or null while its project summary
 * or its instructions are still loading. A project that couldn't be read has
 * no brief: it analyses without code context.
 */
export function tabContext(
  root: string | null,
  project: LoadedProject | null,
  instructions: LoadedInstructions | null,
): { key: string; instructions: string } | null {
  if (!instructions || instructions.root !== (root ?? "")) return null;
  let briefCreatedAt: number | undefined;
  if (root !== null) {
    if (!project || project.root !== root || (project.summary === null && project.error === null)) return null;
    briefCreatedAt = project.summary?.brief?.createdAt;
  }
  return { key: analysisContextKey(root ?? undefined, briefCreatedAt, instructions.current), instructions: instructions.current };
}
```

- [ ] **Step 4: Run the test**

Run: `npx vitest run src/lib/tabContext.test.ts`
Expected: the 5 tests PASS.

- [ ] **Step 5: The per-tab wrappers in `ipc.ts`**

In `src/lib/ipc.ts`, replace the `buildBrief` entry with:

```ts
  buildBrief: (root: string, onEvent: (e: UiEvent) => void) => stream("build_brief", { root }, onEvent),
```

and replace the `analyze` and `cancelAnalysis` entries with:

```ts
  analyze: (
    tabId: string,
    draft: string,
    previousDraft: string | null,
    instructions: string,
    context: string,
    onEvent: (e: UiEvent) => void,
  ) => stream("analyze", { tabId, draft, previousDraft, instructions, context }, onEvent),
  cancelAnalysis: (tabId: string) => invoke<void>("cancel_analysis", { tabId }),
```

- [ ] **Step 6: `NotesPane` reads a tab's analysis**

In `src/components/NotesPane.tsx`, replace the line

```ts
import type { useLiveAnalysis } from "../lib/useLiveAnalysis";
```

with

```ts
import type { AnalysisState } from "../lib/analysisState";
```

and replace the line

```ts
type Analysis = ReturnType<typeof useLiveAnalysis>;
```

with

```ts
/** The shown tab's analysis, whether the draft has moved on since, and "Try again". */
type Analysis = AnalysisState & { stale: boolean; rerun: () => void };
```

Leave the rest of the file, including `type RuleState` and `const RULE`, as it is.

- [ ] **Step 7: Replace `src/App.tsx`**

```tsx
import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { initialAnalysis } from "./lib/analysisState";
import { activityLine } from "./lib/brief";
import { onBriefStatus } from "./lib/events";
import { api, errorMessage, type ProjectInstructions, type ProjectSummary, type Settings, type Tab, type Template } from "./lib/ipc";
import { EMPTY_INSTRUCTIONS, instructionsLabel } from "./lib/instructions";
import { browserStorage, importLegacy } from "./lib/legacyDrafts";
import { providerLabel } from "./lib/providerOptions";
import { createSaver } from "./lib/saver";
import { tabContext, type LoadedProject } from "./lib/tabContext";
import { TabSession, type TabSessionDeps } from "./lib/tabSession";
import { useTabSession } from "./lib/useTabSession";
import { currentWindowId } from "./lib/windowId";
import { TopBar } from "./components/TopBar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { InstructionsDrawer } from "./components/InstructionsDrawer";
import { TemplateList } from "./components/TemplateList";
import { SettingsDialog } from "./components/SettingsDialog";

/** One project's brief build, as this window sees it. */
interface BriefRun {
  building: boolean;
  live: string;
  activity: string[];
  error: string | null;
}

const IDLE_BRIEF: BriefRun = { building: false, live: "", activity: [], error: null };

/** A file the backend kept as `.bad`, named for the banner. */
function unreadableNotice(path: string): string {
  const file = path.split(/[\\/]/).at(-1) ?? path;
  return file.startsWith("workspace.json")
    ? `Couldn't read the saved windows; kept them as ${file}`
    : `Couldn't read a tab; kept it as ${file}`;
}

export default function App() {
  const windowId = useMemo(() => currentWindowId(), []);
  const [settings, setSettings] = useState<Settings | null>(null);
  const settingsRef = useRef<Settings | null>(null);
  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  // The window's active tab: its live analysis, and the project it reads.
  const [session, setSession] = useState<TabSession | null>(null);
  const sessionRef = useRef<TabSession | null>(null);
  const view = useTabSession(session);
  const root = session?.root ?? null;
  const [project, setProject] = useState<LoadedProject | null>(null);
  const [opening, setOpening] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [notices, setNotices] = useState<string[]>([]);

  const [instructions, setInstructions] = useState<ProjectInstructions>(EMPTY_INSTRUCTIONS);
  // Whose instructions `instructions` holds ("" = no project); null while they load.
  const [instructionsRoot, setInstructionsRoot] = useState<string | null>(null);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [instructionsError, setInstructionsError] = useState<string | null>(null);
  const [showInstructions, setShowInstructions] = useState(false);

  const [briefs, setBriefs] = useState<Record<string, BriefRun>>({});
  const [showBrief, setShowBrief] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Draft edits save 500 ms after typing stops, each under the tab it was typed in.
  const draftSaver = useMemo(
    () =>
      createSaver<string>(
        (tabId, draft) => api.saveDraft(tabId, draft).then(() => setSaveError(null)),
        500,
        (e) => setSaveError(`Couldn't save your draft: ${errorMessage(e)}`),
      ),
    [],
  );
  // Instruction edits, the same way, each under the project they were typed in.
  const instructionsSaver = useMemo(
    () =>
      createSaver<ProjectInstructions>(api.saveProjectInstructions, 500, (e) =>
        setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`),
      ),
    [],
  );
  useEffect(() => {
    // Closing the window doesn't unmount React, so flush when the page goes away too.
    const flush = () => {
      void draftSaver.flush();
      void instructionsSaver.flush();
    };
    window.addEventListener("pagehide", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      flush();
    };
  }, [draftSaver, instructionsSaver]);

  const deps = useMemo<TabSessionDeps>(
    () => ({
      analyze: api.analyze,
      saveDraft: (tabId, draft) => draftSaver.schedule(tabId, draft),
      debounceMs: () => settingsRef.current?.debounceMs ?? 1200,
    }),
    [draftSaver],
  );

  const patchBrief = (r: string, f: (b: BriefRun) => BriefRun) =>
    setBriefs((all) => ({ ...all, [r]: f(all[r] ?? IDLE_BRIEF) }));

  /** The shown tab's project summary. A folder that can't be read keeps its message. */
  const loadSummary = async (r: string) => {
    try {
      const summary = await api.projectSummary(r);
      if (sessionRef.current?.root === r) setProject({ root: r, summary, error: null });
    } catch (e) {
      // The tab still shows its draft and notes, and analyses without code context.
      if (sessionRef.current?.root === r) setProject({ root: r, summary: null, error: errorMessage(e) });
    }
  };

  /** The shown tab's instructions ("" for no project), and the shared templates. */
  const loadInstructions = async (r: string | null) => {
    await instructionsSaver.flush(); // a pending edit must not lose to the read below
    try {
      const v = await api.getInstructions(r ?? "");
      setTemplates(v.templates);
      if (sessionRef.current?.root !== r) return;
      setInstructions(v.project);
      setInstructionsError(null);
    } catch (e) {
      if (sessionRef.current?.root !== r) return;
      setInstructions(EMPTY_INSTRUCTIONS);
      setInstructionsError(`Couldn't load instructions: ${errorMessage(e)}`);
    }
    setInstructionsRoot(r ?? "");
  };

  /** Shows `tab`: a new session for it, then its project and instructions. `known` is a summary just fetched. */
  const show = (tab: Tab, known?: ProjectSummary) => {
    sessionRef.current?.dispose(); // a run it started finishes in the backend and saves to its own tab
    const s = new TabSession(tab, deps);
    sessionRef.current = s;
    setSession(s);
    setShowBrief(false);
    setShowInstructions(false);
    setInstructionsRoot(null);
    setProject(tab.root === null ? null : { root: tab.root, summary: known ?? null, error: null });
    void loadInstructions(tab.root);
    if (tab.root !== null && !known) void loadSummary(tab.root);
  };

  // Load this window's tabs once. StrictMode runs effects twice; the ref keeps it to one load.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    api.getSettings().then(setSettings);
    void api.warmUp();
    // Templates are shared, so load them even if this window's tabs don't load
    // (a template save writes the whole list).
    void api.getInstructions("").then((v) => setTemplates(v.templates));
    void (async () => {
      try {
        let ws = await api.listWorkspace(windowId);
        if (ws.fresh) {
          // The first launch of this build: the drafts the old build kept in localStorage become tabs.
          const failed = await importLegacy(windowId, browserStorage(), api.importLegacyDrafts);
          if (failed) setLoadError(failed);
          ws = await api.listWorkspace(windowId);
        }
        setNotices(ws.unreadable.map(unreadableNotice));
        let tab = ws.active;
        if (!tab && ws.tabs.length > 0) {
          await api.activateTab(windowId, ws.tabs[0].id);
          tab = await api.getTab(ws.tabs[0].id);
        }
        // An empty window still takes a draft: a tab with no project, as before tabs.
        show(tab ?? (await api.createTab(windowId, null)));
      } catch (e) {
        setLoadError(errorMessage(e));
      }
    })();
  }, []);

  // A brief built in any window: the rescanned summary carries the new brief,
  // and the new context re-reads the draft.
  useEffect(() => {
    const unlisten = onBriefStatus((e) => {
      if (e.state === "building") {
        patchBrief(e.root, (b) => (b.building ? b : { ...b, building: true, error: null }));
      } else if (e.state === "done") {
        patchBrief(e.root, (b) => ({ ...b, building: false }));
        if (sessionRef.current?.root === e.root) void loadSummary(e.root);
      } else {
        const message = e.message ?? "Couldn't build the project brief.";
        patchBrief(e.root, (b) => ({ ...b, building: false, error: b.error ?? message }));
      }
    });
    return () => void unlisten.then((u) => u());
  }, []);

  // The tab's context reaches its session only once both its project summary
  // and its instructions have loaded.
  const context = tabContext(root, project, instructionsRoot === null ? null : { root: instructionsRoot, current: instructions.current });
  useEffect(() => {
    if (session && context) session.setContext(context.key, context.instructions);
  }, [session, context?.key, context?.instructions]);

  // Esc closes either drawer. The settings <dialog> handles its own Esc.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showSettings) {
        setShowBrief(false);
        setShowInstructions(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  /** "Open a project…": this window's tab for that project, or a new one, as drafts were per project before tabs. */
  const openFolder = async (path: string) => {
    setOpening(true);
    setLoadError(null);
    try {
      await draftSaver.flush(); // the tab you leave keeps its last edit
      const summary = await api.openProject(path); // scans, replacing any loaded index
      if (sessionRef.current?.root === summary.root) {
        setProject({ root: summary.root, summary, error: null }); // a rescan of the project on screen
        return;
      }
      const ws = await api.listWorkspace(windowId);
      const existing = ws.tabs.find((t) => t.root === summary.root);
      let tab: Tab;
      if (existing) {
        await api.activateTab(windowId, existing.id);
        tab = await api.getTab(existing.id);
      } else {
        tab = await api.createTab(windowId, summary.root);
      }
      show(tab, summary);
    } catch (e) {
      setLoadError(errorMessage(e));
    } finally {
      setOpening(false);
    }
  };

  const pickProject = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir === "string") await openFolder(dir);
  };

  const changeInstructions = (next: ProjectInstructions) => {
    setInstructions(next);
    setInstructionsError(null);
    instructionsSaver.schedule(root ?? "", next);
  };

  const changeTemplates = async (next: Template[]) => {
    setTemplates(next);
    try {
      await api.saveTemplates(next);
      setInstructionsError(null);
    } catch (e) {
      setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`);
    }
  };

  const restoreStarters = async () => {
    try {
      setTemplates(await api.restoreStarterTemplates());
      setInstructionsError(null);
    } catch (e) {
      setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`);
    }
  };

  const buildBrief = () => {
    if (root === null || briefs[root]?.building) return;
    const r = root;
    setShowInstructions(false);
    setShowBrief(true);
    patchBrief(r, () => ({ building: true, live: "", activity: [], error: null }));
    api
      .buildBrief(r, (ev) => {
        if (ev.type === "text") {
          const text = ev.text;
          patchBrief(r, (b) => ({ ...b, live: b.live + text }));
        } else if (ev.type === "activity") {
          const line = activityLine(ev.label, ev.lane);
          patchBrief(r, (b) => ({ ...b, activity: [...b.activity, line] }));
        } else if (ev.type === "done") {
          // The summary refreshes on `brief-status: done`, which every window receives.
          patchBrief(r, (b) => ({ ...b, building: false }));
        } else if (ev.type === "error") {
          const message = ev.message;
          patchBrief(r, (b) => ({ ...b, building: false, error: message }));
        }
      })
      .catch((e) => patchBrief(r, (b) => ({ ...b, building: false, error: errorMessage(e) })));
  };

  const saveSettings = async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
  };

  const summary = project !== null && project.root === root ? project.summary : null;
  const brief = summary?.brief ?? null;
  const run = root !== null ? (briefs[root] ?? IDLE_BRIEF) : IDLE_BRIEF;
  const loadingProject = root !== null && summary === null && (project?.error ?? null) === null;
  const errors = [loadError, project !== null && project.root === root ? project.error : null, saveError].filter(
    (m): m is string => !!m,
  );
  const analysis = view
    ? { ...view.analysis, stale: view.stale, rerun: () => session?.rerun() }
    : { ...initialAnalysis, stale: false, rerun: () => {} };

  return (
    <div className="flex h-full flex-col">
      <TopBar
        project={summary}
        opening={opening || loadingProject}
        briefStatus={summary?.briefStatus ?? null}
        briefBuilding={run.building}
        providerLabel={providerLabel(settings)}
        instructionsLabel={instructionsLabel(instructions, templates)}
        onOpenProject={pickProject}
        onShowBrief={() => {
          setShowInstructions(false);
          setShowBrief((v) => !v);
        }}
        onShowInstructions={() => {
          setShowBrief(false);
          setShowInstructions((v) => !v);
        }}
        onOpenSettings={() => setShowSettings(true)}
      />
      {errors.map((m, i) => (
        <p key={i} className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-fault" role="alert">
          {m}
        </p>
      ))}
      {notices.map((m, i) => (
        <p key={i} className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-muted">
          {m}
        </p>
      ))}

      {/* Both panes fill the remaining height and scroll on their own. */}
      <main className="relative grid min-h-0 flex-1 grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]">
        <DraftPane value={view?.draft ?? ""} onChange={(text) => session?.setDraft(text)} hasProject={!!summary} />
        <NotesPane analysis={analysis} hasProject={!!summary} briefMissing={!brief && !run.building} onBuildBrief={buildBrief} />
        {showBrief && summary && (
          <BriefDrawer
            brief={brief}
            status={summary.briefStatus}
            building={run.building}
            liveText={run.live}
            activity={run.activity}
            error={run.error}
            onBuild={buildBrief}
            onClose={() => setShowBrief(false)}
          />
        )}
        {showInstructions && instructionsRoot !== null && (
          <InstructionsDrawer
            key={instructionsRoot}
            instructions={instructions}
            templates={templates}
            localModel={settings?.provider.kind === "local"}
            error={instructionsError}
            onChange={changeInstructions}
            onClose={() => setShowInstructions(false)}
          >
            <TemplateList templates={templates} current={instructions.current} onSave={changeTemplates} onRestore={restoreStarters} />
          </InstructionsDrawer>
        )}
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
```

- [ ] **Step 8: Delete the old hook and check nothing uses it**

Run:
```bash
git rm src/lib/useLiveAnalysis.ts
grep -rn "useLiveAnalysis\|livebrainstorm\.draft\.\|livebrainstorm\.lastProject" src
```
Expected: `git rm` succeeds. The `grep` lists only `src/lib/legacyDrafts.ts` and `src/lib/legacyDrafts.test.ts`: the old keys are read once, for the import, and nothing writes them any more.

- [ ] **Step 9: Typecheck and test**

Run: `npm run typecheck && npm test && cargo check -p livebrainstorm`
Expected: no type errors, every vitest test passes, and the shell compiles.

- [ ] **Step 10: Manual check in the running app (the user runs `npm run tauri dev`)**

Hand this list to the user. On macOS the session files are under `~/Library/Application Support/app.livebrainstorm.desktop/sessions/`.

1. **Upgrade.** Before switching to this branch, run today's build and type a draft in two projects and one with no project open. Then launch this branch.
   - The last project opens with its draft.
   - *Open a project…* on the other project shows its draft.
   - The no-project draft became a tab too, although this single-pane build has no way to show it: `grep -l '"root": null' ~/Library/Application\ Support/app.livebrainstorm.desktop/sessions/tabs/*.json` lists one file, and that file holds the draft.
   - In the webview's devtools, `Object.keys(localStorage)` lists no `livebrainstorm.draft.*` key and no `livebrainstorm.lastProject`.
2. **Notes survive a restart.** Type until notes appear, then quit with ⌘Q and relaunch.
   - The same notes show at once, and the rule between the panes is solid (not animated).
   - `pgrep -fl "claude -p"` prints nothing: no model call started.
3. **A stale draft.** Type a few more words, wait about half a second (the draft saves 500 ms after you stop), and quit before the rule starts to animate. On relaunch the old notes show with the dashed rule. Nothing runs until you type again.
4. **Switching projects.** While project A shows "Thinking", open project B: B shows its own draft and notes. Open A again: the notes from the run that finished in the background are there.
5. **Brief.** Build the brief: the drawer streams it. When it's done, the top bar reads "Project brief is current" and the notes re-run once.
6. **A missing folder.** Build the project's brief first, so the saved notes ran with it. Then rename the open project's folder on disk and relaunch.
   - The draft and the saved notes show, and the banner reads "That folder no longer exists. Open it again from its new place."
   - About a second later the notes re-run once, without code context. That's expected in this phase: with no readable folder there's no brief, so the context key (`/p|0|…`) differs from the one the saved notes ran with.
   - Typing still analyses, without code context.
   - Rename the folder back and relaunch: the banner is gone, and the notes re-run once more, with the brief.
7. **A damaged tab file.** Quit, replace one file in `sessions/tabs/` with `garbage`, and relaunch. The banner reads "Couldn't read a tab; kept it as t-….json.bad", and that file sits next to the others.

- [ ] **Step 11: Commit**

```bash
git add src/lib/tabContext.ts src/lib/tabContext.test.ts src/lib/ipc.ts src/components/NotesPane.tsx src/App.tsx
git commit -m "feat(ui): the window runs on its active tab; notes survive restarts; old drafts import once

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

(`git rm` in Step 8 already staged the deletion of `src/lib/useLiveAnalysis.ts`.)

---

### Task 10: AGENTS.md, and the final checks

**Files:**
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: every name the earlier tasks produced.
- Produces: documentation only. Later phases add their lines next to these. Phase 4 adds its ledger line "after the line about sessions" in Persistence, and Phase 3 adds its events line to the cross-language contracts.

- [ ] **Step 1: Architecture**

In `AGENTS.md`, in the Architecture table:
- Replace the row that starts with `| UI |` with:
  ```markdown
  | UI | `src/` (React 19 + Vite, no state library) | Draft, notes, brief drawer, settings; state lives in `App.tsx`, and each tab's live analysis in a `TabSession` (`src/lib/tabSession.ts`) outside React |
  ```
- Replace the row that starts with `| Core |` with:
  ```markdown
  | Core | `crates/livebrainstorm-core/` | Scan/index/retrieve, brief cache, prompts, providers, local runtime, sessions (tabs, windows, saved analyses). **Must not depend on Tauri** |
  ```

- [ ] **Step 2: Live analysis flow**

In "### Live analysis flow", replace step 1 (the line that starts with `1.` and names `useLiveAnalysis.ts`) with:

```markdown
1. `TabSession` (`src/lib/tabSession.ts`), one per tab and outside React, waits for a typing pause (`debounceMs`, default 1200). It needs at least 24 characters after whitespace normalization and skips the call if the normalized draft hasn't changed. When the project, brief or instructions change (`setContext`), it re-sends the same draft. The session instructions are part of the context key (`analysisContextKey` in `src/lib/instructions.ts`), so editing them re-reads the same draft without the gate, and every `analyze` call carries the current text and the key. The window sets a tab's context only after it has loaded both that project's summary and its instructions: a key computed earlier would differ from the saved notes' and start a run at launch. A restored tab shows its latest saved analysis. When its draft matches that analysis (ignoring whitespace) and the key is the one the analysis ran with, no run starts; a draft that moved on shows the notes as stale until the next edit. A tab with no saved analysis, or whose key changed (a new brief, other instructions, a folder that can no longer be read), is read again once its context arrives.
```

Replace step 2 (the line that starts with `2.` and names the `AppState.analysis` slot) with:

```markdown
2. `analyze(tabId, draft, previousDraft, instructions, context)` (`src-tauri/src/commands.rs`) checks that the tab is open (`SessionStore::open_tab_root`, else "This tab was closed."), then returns immediately after spawning a task into `AppState.analyses`, keyed by tab id. `replace_task` aborts only that tab's previous task, so tabs analyse in parallel, and dropping that future kills its `claude` children (`kill_on_drop`). `cancel_analysis(tabId)` and `close_tab` abort it too. The task takes the index and brief from the registry entry for the tab's root (`AppState.projects`, scanned on first use); a tab with no project, or whose folder can't be read, runs without code context.
```

At the end of step 4 (the line that ends with the list of events, `progress`), append this text to the same line:

```markdown
 Before `done` is sent, the task saves the run to the tab's history (`sessions::Recorder`): the text, `partial`, the notices so far, and any checks that arrived first. Later checks, notices and `checksStopped` update that snapshot (`update_snapshot`). Skips and errors save nothing; a save that fails becomes one notice ("Couldn't save these notes: …") and never fails the analysis.
```

Replace the paragraph after step 5 (the one that starts with `Brief building`) with:

```markdown
Downloads (`install_runtime`, `download_model`) follow the same pattern: spawn, keep the handle in an `AppState` slot, stream over a Channel, then send a final `done` or `error`. So does `build_brief(root)` (`Tier::Deep`), with one build per project: `AppState.brief_tasks` is keyed by root, and a request for a root whose build is still running returns at once. Every window receives `brief-status { root, state: building | done | error, message }`; only the window that started the build gets the live text over its Channel. On success the task saves the brief, rescans the project into the registry, then emits `done`, and each window refetches `project_summary(root)`.
```

- [ ] **Step 3: Project understanding**

In "### Project understanding: two layers", add after the `- **Retrieval (every pause).**` bullet:

```markdown
- **Loaded projects.** `AppState.projects` holds one `ProjectEntry { index, brief }` per root, so tabs on several projects analyse at once. `open_project` always rescans and replaces the entry; `project_summary` scans only a root that isn't loaded. An index keeps file contents, so after `close_tab` and `delete_tab` the shell drops every entry no open tab uses (`AppState::evict_unused_projects`).
```

- [ ] **Step 4: Persistence**

Replace the `- Session instructions and templates:` line with:

```markdown
- Session instructions and templates: `<app_config>/instructions.json` (`instructions::InstructionStore`), written atomically under a mutex (`store::write_atomic`). A missing file starts with the five starter templates. One that doesn't parse is renamed to `instructions.json.bad` (then `.bad.1`, …; `store::load_json`) rather than overwritten, because it holds the user's writing. Projects are keyed by root path, and `""` is the session with no project open. The frontend saves edits 500 ms after typing stops (`src/lib/saver.ts`) and flushes before switching projects.
```

Replace the line that starts with `- Frontend: drafts are stored per project` with these three lines:

```markdown
- Sessions: `<app_data>/sessions/` (`sessions::SessionStore`). `workspace.json` lists each window's tabs and its active tab. `tabs/<id>.json` holds one tab's root, title, draft and its last `HISTORY_LEN` (5) analyses, newest first (`Snapshot`, with its checks, `checksTotal` and `checksFaults`). A closed tab is any tab file no window lists, so there's no closed list to drift; *Recently closed* sorts by `closedAt`, else `updatedAt`. Writes are atomic (`store::write_atomic`). A file that doesn't parse is kept as `.bad`, `.bad.1`, … (`store::load_json`, never replacing an earlier copy) and skipped; a file that can't be read is reported and never overwritten. One `std` mutex guards the workspace and a cache of the tabs read so far, held only for file I/O. The frontend saves drafts 500 ms after typing stops (`createSaver`, keyed by tab id) and flushes before switching tabs and on `pagehide`.
- Upgrade: on the first launch of the tabs build (`WindowView.fresh`), the drafts the old build kept in `localStorage` (`livebrainstorm.draft.<root>`, with `none` for no project, and `livebrainstorm.lastProject`) become tabs through `import_legacy_drafts`, which runs once. The keys are removed only after it succeeds.
- Frontend: download progress lives in a module-level store (`src/lib/downloads.ts`, `useSyncExternalStore`) so it survives closing Settings.
```

- [ ] **Step 5: Cross-language contracts**

Add after the `- **Session instructions:**` line:

```markdown
- **Sessions:** `Snapshot` / `Tab` / `TabSummary` / `WindowView` / `LegacyDraft` in `sessions/mod.rs` ↔ `src/lib/ipc.ts`. `sessions::first_line` ↔ `firstLine` in `src/lib/tabTitle.ts` (the first non-empty line, trimmed of Unicode White_Space, at most 48 Unicode scalars; TS uses `\p{White_Space}`, not `trim()`, which differs on U+FEFF and U+0085); both test suites pin the same strings.
- **Brief and workspace events:** `brief-status` (`events::BriefStatus { root, state, message }`, `state` lowercase) and `workspace-changed` (`events::WorkspaceChanged { windows }`) in `src-tauri/src/events.rs` ↔ `onBriefStatus` / `onWorkspaceChanged` in `src/lib/events.ts`. Every session command emits `workspace-changed`, listing the windows whose tab list changed; `save_draft` emits it only when the draft's first line changed.
```

- [ ] **Step 6: Deliberate decisions**

In "## Deliberate decisions to preserve", replace the line that starts with `- Live analysis is stateless` with these two lines:

```markdown
- Live analysis is stateless: each request spawns a fresh `claude -p`. A persistent stream-json session would start faster but would accumulate conversation history. A tab's saved analyses are for the user to browse; they're never sent to the model.
- Every tab or window id from the webview passes `sessions::valid_id` (`^[tw]-[0-9a-f-]{1,40}$`) before it's used, in the command and again in `SessionStore`. A tab id becomes a file name (`tabs/<id>.json`), so this check is what stops path traversal. Anything else is `Precondition("Unknown tab.")`.
```

- [ ] **Step 7: Final checks**

Run:
```bash
cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets && cargo check -p livebrainstorm && npm run typecheck && npm test
grep -n 'useLiveAnalysis\|AppState\.analysis`\|drafts are stored per project' AGENTS.md
```
Expected:
- Everything passes, and clippy prints no warnings.
- The `grep` prints nothing, because AGENTS.md no longer describes the single-slot app. Its exit status is 1 when nothing matches, which is the expected result here.

- [ ] **Step 8: Commit**

```bash
git add AGENTS.md
git commit -m "docs: sessions, per-tab runs, per-project briefs and their events in AGENTS.md

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```
