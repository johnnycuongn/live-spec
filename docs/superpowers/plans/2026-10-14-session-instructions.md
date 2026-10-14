# Phase B: Session instructions and templates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each project has a default and a current instruction set that steers the live notes. The user edits, clears, resets or saves it from a top-bar drawer, fills it from templates, and adds, edits, deletes or restores templates.

**Architecture:**
- **Core:**
  - `prompt::analysis_system` and `prompt::analysis_request` take the instruction text. It's clipped and placed in a `<session_instructions>` block before the format rules. Blank text leaves today's prompt byte-identical.
  - New module `livebrainstorm-core::instructions` owns the types, five starter templates, and an `InstructionStore` that keeps `<app_config>/instructions.json`.
- **Shell:** four commands load and save it. `analyze` takes the current text as a parameter, the same way it takes the draft.
- **Frontend:**
  - Pure helpers: `src/lib/instructions.ts` (label, context key, actions) and `src/lib/saver.ts` (debounced, project-keyed saves).
  - Components: `InstructionsDrawer` and `TemplateList`.
  - Wiring: state in `App.tsx`.

**Tech Stack:** Rust 2021 (serde, serde_json, tempfile for tests), Tauri v2, React 19 + Tailwind v4, vitest 3.

**Spec:** `docs/superpowers/specs/2026-10-14-session-instructions-and-claude-options-design.md`, Part B ("Session instructions"), plus "Testing", "Implementation order" and "Documentation updates".

**Prerequisite:** Phase A (`docs/superpowers/plans/2026-10-14-claude-options.md`) is done on the same branch. This plan's test helpers use `ClaudeCli::new` from Phase A, Task 2, and its `App.tsx` imports `providerLabel` from `src/lib/providerOptions.ts` (Phase A, Task 4).

## Global Constraints

- **Instruction cap:** `MAX_INSTRUCTION_CHARS = 4_000` characters (not bytes). The effective cap is `min(4_000, provider.context_budget_chars() / 10)`.
- **Prompt block,** placed after the section guide (and after the "written separately" line for lanes) and before `ANALYSIS_RULES`, with the instructions trimmed:
  ```
  <session_instructions>
  {instructions}
  </session_instructions>
  These instructions come from the person drafting. Follow them, but the headings above, their order, and the rules below always win.
  ```
- **Blank or whitespace-only instructions add nothing.** `analysis_system(&SECTIONS, "")` must equal today's prompt.
- **Scope:** only the live notes see instructions. The brief, explorers, merge, Jev gate, rerank and fact-check don't.
- **Storage:**
  - File `<app_config>/instructions.json`, written atomically (tmp file + rename) under a `std::sync::Mutex`.
  - Missing file: the five starters and no projects.
  - Unreadable or invalid file: renamed to `instructions.json.bad`, then the starters.
  - `"templates": []` stays empty.
  - Projects are keyed by root path, with `""` for no project. An entry equal to the default is removed.
- **Starter ids, names and texts:** verbatim from the spec (`feature-design`, `bug-hunt`, `refactor`, `product-ux`, `devils-advocate`).
- **Top-bar label:**

  | State | Label |
  |---|---|
  | `current` is blank | "No instructions" |
  | Applied template, text unchanged | template name |
  | Applied template, text changed | "{name} (edited)" |
  | Applied template was deleted | "Custom" |
  | No template and `current === default` | "Project default" |
  | Anything else | "Custom" |

  The button reads "Instructions: {label}".
- **Copy, verbatim:** "Session instructions", "No default for this project", "Couldn't save instructions: {error}", "Local models read fewer characters, depending on the model", "Template applied.", "Undo", "Clear", "Reset to default", "Save as default", "+ New template", "Delete {name}?", "Yes", "No", "Restore starter templates".
- **Saving:** a debounce of 500 ms after the last edit. A pending save is flushed before a project switch and on unmount. Template changes save immediately.
- **No browser dialogs:** no `alert`, `confirm` or `prompt`.
- **Process:**
  - Branch `feat/instructions-and-claude-options`.
  - Commit trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
  - Never run `cargo fmt` across the repo.
  - `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests.
  - UI uses semantic tokens and `ui.tsx` primitives.
  - The user runs `npm run tauri dev`.

## Review Focus

1. **Switching projects with an edit still waiting to save.** Expected: the edit is saved to the project it was typed in, never the newly opened one. Pinned in Task 4 (`an edit pending for one project is saved to that project when another starts`), and Task 5's `loadProject` awaits `saver.flush()`.
2. **Undo after switching projects.** Expected: it never puts one project's text into another. The drawer is keyed by project root, so it remounts with no undo state (Task 5, manual check 4).
3. **Multi-byte text at the cap.** Expected: it's clipped on a character boundary and never panics, and every lane gets the same text. Pinned in Task 1 (`every_lane_gets_the_same_clipped_instructions`).
4. **The applied template was later deleted or edited.** Expected: the label says "Custom" or "(edited)", never a stale name or a crash. Pinned in Task 4 (`covers every state in the spec's table`, `a template edited after it was applied reads as edited`).
5. **A corrupt or unreadable instructions file.** Expected: it never blocks opening a project, and the user's templates survive as `.bad`. `get_instructions` can't fail. Pinned in Task 2 (`a_corrupt_file_is_kept_aside_not_overwritten`). The templates are also loaded at startup even when the last project can't be opened, so a template save never writes a list that's missing the file's templates (Task 5, `startup` step).

---

### Task 1: Core: instructions in the analysis prompt

**Files:**
- Modify: `crates/livebrainstorm-core/src/prompt.rs`
- Modify: `crates/livebrainstorm-core/tests/notes_live.rs:32` (new argument)
- Modify: `src-tauri/src/commands.rs:277` (passes `""` until Task 3)

**Interfaces:**
- Produces:
  - `prompt::MAX_INSTRUCTION_CHARS: usize = 4_000`
  - `prompt::analysis_system(sections: &[&str], instructions: &str) -> String`
  - `prompt::analysis_request(draft, index, brief, snippets, provider, sections, instructions: &str) -> ChatRequest`: the new last parameter is clipped to `min(MAX_INSTRUCTION_CHARS, budget / 10)` characters.

- [ ] **Step 1: Write the failing tests**

In `crates/livebrainstorm-core/src/prompt.rs`, inside `mod tests`, add these tests after `a_lane_prompt_has_only_its_own_headings`:

```rust
    #[test]
    fn blank_instructions_leave_the_prompt_unchanged() {
        assert_eq!(analysis_system(&SECTIONS, ""), TODAYS_PROMPT);
        assert_eq!(analysis_system(&SECTIONS, "  \n\t "), TODAYS_PROMPT);
    }

    #[test]
    fn instructions_sit_between_the_sections_and_the_rules() {
        let p = analysis_system(&SECTIONS, "  Focus on concurrency.\n");
        let block = p
            .find("<session_instructions>\nFocus on concurrency.\n</session_instructions>\n")
            .expect("the block is present, with the text trimmed");
        assert!(p.find("## Questions").unwrap() < block, "after the section guide");
        assert!(block < p.find("Rules: bullets only").unwrap(), "before the rules, so the rules win");
        assert!(p.contains("Follow them, but the headings above, their order, and the rules below always win."));
        let lane = analysis_system(&["Faults"], "Focus on concurrency.");
        assert!(lane.find("written separately").unwrap() < lane.find("<session_instructions>").unwrap());
    }

    #[test]
    fn every_lane_gets_the_same_clipped_instructions() {
        let kept = |system: &str| {
            let inner = system.split("<session_instructions>\n").nth(1).unwrap();
            inner.split("\n</session_instructions>").next().unwrap().chars().count()
        };
        let long = "é".repeat(5_000);
        for sections in section_groups(4) {
            let req = analysis_request("add caching to search", None, None, &[], &claude(), &sections, &long);
            assert_eq!(kept(&req.system), MAX_INSTRUCTION_CHARS, "{sections:?}");
        }
        // A 4,096-token local model has a 6,144-character budget; it keeps a tenth.
        let req = analysis_request("add caching to search", None, None, &[], &local(), &SECTIONS, &long);
        assert_eq!(kept(&req.system), 614);
    }
```

Update the existing calls in the same module to the new signatures:
- In `analysis_without_project_still_works`: `analysis_request("add caching to search", None, None, &[], &local(), &SECTIONS, "")`.
- In `all_four_sections_is_todays_prompt`: `analysis_system(&SECTIONS, "")`.
- In `a_lane_prompt_has_only_its_own_headings`: `analysis_system(&["Faults"], "")` and `analysis_system(&["Directions", "Questions"], "")`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core prompt::tests`
Expected: compile errors, such as `this function takes 1 argument but 2 arguments were supplied` and `cannot find value 'MAX_INSTRUCTION_CHARS'`.

- [ ] **Step 3: Implement**

In `crates/livebrainstorm-core/src/prompt.rs`, below `const ANALYSIS_RULES: &str = …;`, add:

```rust
/// Session instructions longer than this are clipped (small local models keep less).
pub const MAX_INSTRUCTION_CHARS: usize = 4_000;
```

Replace `analysis_system` with:

```rust
/// The system prompt for an agent writing `sections`, steered by the user's
/// session instructions. With all four sections and no instructions it is
/// exactly the single-agent prompt.
pub fn analysis_system(sections: &[&str], instructions: &str) -> String {
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
    // Before the rules, which win: instructions steer what the notes say, never their shape.
    let instructions = instructions.trim();
    if !instructions.is_empty() {
        prompt.push_str(&format!(
            "\n\n<session_instructions>\n{instructions}\n</session_instructions>\n\
             These instructions come from the person drafting. Follow them, but the headings above, \
             their order, and the rules below always win."
        ));
    }
    prompt.push_str(ANALYSIS_RULES);
    prompt
}
```

In `analysis_request`, add the parameter `instructions: &str,` after `sections: &[&str],`. Replace the `system:` line of the returned `ChatRequest` with:

```rust
        system: analysis_system(sections, clip_chars(instructions, MAX_INSTRUCTION_CHARS.min(budget / 10))),
```

Next to `fn clip`, add:

```rust
/// The first `max` characters (not bytes), so the editor's counter matches.
fn clip_chars(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(i, _)| &s[..i])
}
```

- [ ] **Step 4: Update the other callers**

In `crates/livebrainstorm-core/tests/notes_live.rs`, change the request line to:

```rust
            let req = prompt::analysis_request(draft, Some(&index), None, &snippets, &provider, sections, "");
```

In `src-tauri/src/commands.rs` (inside `analyze`'s lane loop), change the request line to the one below. Task 3 replaces `""`.

```rust
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, sections, "");
```

- [ ] **Step 5: Run the tests and checks**

Run: `cargo test -p livebrainstorm-core && cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: all tests PASS, including the three new tests, `all_four_sections_is_todays_prompt` and `a_lane_prompt_has_only_its_own_headings`. The shell compiles and clippy prints no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/prompt.rs crates/livebrainstorm-core/tests/notes_live.rs src-tauri/src/commands.rs
git commit -m "feat(core): session instructions in every lane's prompt, before the rules

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Core: instruction store and starter templates

**Files:**
- Create: `crates/livebrainstorm-core/src/instructions.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod instructions;`)

**Interfaces:**
- Produces, all in `livebrainstorm_core::instructions` and all `Serialize`/`Deserialize` camelCase:
  - `Template { id: String, name: String, text: String }`
  - `ProjectInstructions { default: String, current: String, template_id: Option<String> }` (`Default`; serde `default`)
  - `InstructionsFile { templates: Vec<Template>, projects: BTreeMap<String, ProjectInstructions> }`, where `Default` gives the starters and no projects
  - `InstructionsView { templates: Vec<Template>, project: ProjectInstructions }` (Serialize only)
  - `pub fn starters() -> Vec<Template>`
  - `InstructionStore`:
    - `new(path: PathBuf) -> Self`
    - `load(&self) -> InstructionsFile`
    - `view(&self, root: &str) -> InstructionsView`
    - `save_project(&self, root: &str, p: ProjectInstructions) -> Result<()>`
    - `save_templates(&self, t: Vec<Template>) -> Result<()>`
    - `restore_starters(&self) -> Result<Vec<Template>>`

- [ ] **Step 1: Write the module with its tests first, with stub bodies**

Create `crates/livebrainstorm-core/src/instructions.rs`:

```rust
//! Session instructions: per-project text that steers the live notes, and the
//! templates that fill it. One JSON file next to settings.json.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    pub text: String,
}

/// One project's instructions. `current` steers the notes; `default` is what
/// "Reset to default" restores. Both start empty, so notes behave as before.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectInstructions {
    pub default: String,
    pub current: String,
    /// The template applied last, for the top-bar label.
    pub template_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct InstructionsFile {
    pub templates: Vec<Template>,
    /// Keyed by project root; "" is the session with no project open.
    pub projects: BTreeMap<String, ProjectInstructions>,
}

impl Default for InstructionsFile {
    fn default() -> Self {
        Self { templates: starters(), projects: BTreeMap::new() }
    }
}

/// What the UI loads for one project.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionsView {
    pub templates: Vec<Template>,
    pub project: ProjectInstructions,
}

pub fn starters() -> Vec<Template> {
    unimplemented!()
}

pub struct InstructionStore {
    path: PathBuf,
    /// Serializes load-modify-write, so concurrent saves never lose an update.
    lock: Mutex<()>,
}

impl InstructionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    pub fn load(&self) -> InstructionsFile {
        unimplemented!()
    }

    pub fn view(&self, _root: &str) -> InstructionsView {
        unimplemented!()
    }

    pub fn save_project(&self, _root: &str, _p: ProjectInstructions) -> Result<()> {
        unimplemented!()
    }

    pub fn save_templates(&self, _templates: Vec<Template>) -> Result<()> {
        unimplemented!()
    }

    pub fn restore_starters(&self) -> Result<Vec<Template>> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, InstructionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = InstructionStore::new(dir.path().join("instructions.json"));
        (dir, store)
    }

    #[test]
    fn a_missing_file_starts_with_the_starters() {
        let (_dir, store) = store();
        let view = store.view("/p");
        let ids: Vec<_> = view.templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["feature-design", "bug-hunt", "refactor", "product-ux", "devils-advocate"]);
        assert_eq!(view.project, ProjectInstructions::default());
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_not_overwritten() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("instructions.json"), "{ my precious templates").unwrap();
        assert_eq!(store.load(), InstructionsFile::default());
        let bad = dir.path().join("instructions.json.bad");
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ my precious templates");
        store.save_project("/p", ProjectInstructions { current: "x".into(), ..Default::default() }).unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ my precious templates", "later saves never touch the .bad copy");
        assert_eq!(store.view("/p").project.current, "x");
    }

    #[test]
    fn deleting_every_template_sticks_until_restore() {
        let (_dir, store) = store();
        store.save_templates(vec![]).unwrap();
        assert!(store.load().templates.is_empty(), "an empty list is the user's choice, not a missing file");
        let mut edited = starters()[1].clone();
        edited.text = "my own bug hunt".into();
        store.save_templates(vec![edited.clone()]).unwrap();
        let restored = store.restore_starters().unwrap();
        assert_eq!(restored.len(), 5);
        assert_eq!(restored[0], edited, "an edited starter keeps its edits and its place");
        assert_eq!(restored.iter().filter(|t| t.id == "bug-hunt").count(), 1);
        assert_eq!(store.load().templates, restored);
    }

    #[test]
    fn projects_round_trip_and_empty_ones_are_dropped() {
        let (dir, store) = store();
        let p = ProjectInstructions { default: "d".into(), current: "c".into(), template_id: Some("bug-hunt".into()) };
        store.save_project("/a", p.clone()).unwrap();
        store.save_project("", ProjectInstructions { current: "no project".into(), ..Default::default() }).unwrap();
        assert_eq!(store.view("/a").project, p);
        assert_eq!(store.view("").project.current, "no project");
        store.save_project("/a", ProjectInstructions::default()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("instructions.json")).unwrap()).unwrap();
        assert!(json["projects"].get("/a").is_none(), "nothing set means no entry");
        assert_eq!(json["projects"][""], serde_json::json!({"default": "", "current": "no project", "templateId": null}));
    }

    #[test]
    fn concurrent_saves_never_lose_an_update() {
        let (_dir, store) = store();
        std::thread::scope(|s| {
            for i in 0..8 {
                let store = &store;
                s.spawn(move || {
                    let p = ProjectInstructions { current: format!("{i}"), ..Default::default() };
                    store.save_project(&format!("/p{i}"), p).unwrap();
                });
            }
        });
        assert_eq!(store.load().projects.len(), 8);
    }

    #[test]
    fn starters_stay_within_the_notes_format() {
        for t in starters() {
            assert!(!t.text.contains('#'), "{}: headings belong to the prompt, not a template", t.id);
            assert!(t.text.chars().count() < 600, "{}: short enough for small local models", t.id);
        }
    }
}
```

In `crates/livebrainstorm-core/src/lib.rs`, add `pub mod instructions;` between `pub mod error;` and `pub mod jev;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core instructions::tests`
Expected: the tests FAIL with `not implemented` panics.

- [ ] **Step 3: Implement the starters**

Replace the `pub fn starters()` stub with:

```rust
/// The templates a new install starts with, and what "Restore starter
/// templates" brings back. Each steers within the four sections and the rules.
const STARTERS: [(&str, &str, &str); 5] = [
    (
        "feature-design",
        "Feature design",
        "I'm designing a new feature.
- Faults: look for features or code that already do part of this, integration points I haven't mentioned, and data or state the feature needs that the project doesn't have yet.
- Directions: favour the smallest version that would prove the idea works.
- Questions: ask about scope, who the feature is for, and what success looks like.",
    ),
    (
        "bug-hunt",
        "Bug hunt",
        "I'm investigating a bug. Treat my draft as a hypothesis, not a fact.
- Faults: point to code paths that explain or contradict the symptoms I describe, especially error handling, concurrency, caching, and state that outlives a request.
- Directions: suggest the cheapest experiment, log line, or test that would confirm or rule out each cause.
- Questions: ask for the evidence that would narrow it down fastest.",
    ),
    (
        "refactor",
        "Refactor or architecture",
        "I'm planning a refactor or an architectural change.
- Faults: name the callers, contracts, and invariants the change would break, including anything the project's docs say to preserve.
- Directions: propose an order of steps that keeps the code working after each one.
- Questions: ask what must not change, and how we'd know nothing regressed.",
    ),
    (
        "product-ux",
        "Product and UX",
        "I'm thinking about the user's experience, not the implementation.
- Faults: point out confusing states, missing feedback, empty and error states, and accessibility gaps in what I describe.
- Directions: suggest changes to what the user sees and does. Mention code only to say what exists today.
- Questions: ask who the user is and what they're trying to get done.",
    ),
    (
        "devils-advocate",
        "Devil's advocate",
        "Be a skeptical reviewer.
- Faults: give the strongest objection to the plan rather than a list of small issues.
- Directions: include one alternative that rejects the premise of my draft.
- Questions: ask what evidence would change my mind.",
    ),
];

pub fn starters() -> Vec<Template> {
    STARTERS.iter().map(|(id, name, text)| Template { id: (*id).into(), name: (*name).into(), text: (*text).into() }).collect()
}
```

- [ ] **Step 4: Implement the store**

Replace the whole `impl InstructionStore { … }` block with:

```rust
impl InstructionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    /// Never fails: a missing file gives the starters, and an unreadable one is
    /// kept as `.bad` first. Instructions must never block opening a project.
    pub fn load(&self) -> InstructionsFile {
        let _guard = self.lock.lock().expect("instructions lock");
        self.read()
    }

    pub fn view(&self, root: &str) -> InstructionsView {
        let file = self.load();
        InstructionsView { project: file.projects.get(root).cloned().unwrap_or_default(), templates: file.templates }
    }

    /// Stores one project's instructions; an entry with nothing set is removed.
    pub fn save_project(&self, root: &str, p: ProjectInstructions) -> Result<()> {
        self.update(|f| {
            if p == ProjectInstructions::default() {
                f.projects.remove(root);
            } else {
                f.projects.insert(root.to_owned(), p);
            }
        })
    }

    pub fn save_templates(&self, templates: Vec<Template>) -> Result<()> {
        self.update(|f| f.templates = templates)
    }

    /// Re-adds each starter whose id is missing, in starter order, at the end;
    /// never overwrites an edited one. Returns the merged list.
    pub fn restore_starters(&self) -> Result<Vec<Template>> {
        let mut merged = Vec::new();
        self.update(|f| {
            for s in starters() {
                if !f.templates.iter().any(|t| t.id == s.id) {
                    f.templates.push(s);
                }
            }
            merged = f.templates.clone();
        })?;
        Ok(merged)
    }

    fn update(&self, change: impl FnOnce(&mut InstructionsFile)) -> Result<()> {
        let _guard = self.lock.lock().expect("instructions lock");
        let mut file = self.read();
        change(&mut file);
        self.write(&file)
    }

    fn read(&self) -> InstructionsFile {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| self.set_aside()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => InstructionsFile::default(),
            Err(_) => self.set_aside(),
        }
    }

    /// Keeps a file we can't read instead of overwriting the user's templates
    /// on the next save (settings.json just resets; this holds their writing).
    fn set_aside(&self) -> InstructionsFile {
        let _ = std::fs::rename(&self.path, self.path.with_extension("json.bad"));
        InstructionsFile::default()
    }

    /// Atomic write: never leaves a half-written file behind.
    fn write(&self, file: &InstructionsFile) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(file)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
}
```

- [ ] **Step 5: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core instructions::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: the six instructions tests PASS, the whole core suite passes, and clippy prints no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/livebrainstorm-core/src/instructions.rs crates/livebrainstorm-core/src/lib.rs
git commit -m "feat(core): instruction store with five starter templates; corrupt files kept as .bad

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Shell and IPC: the four commands, and instructions sent with each analysis

**Files:**
- Modify: `src-tauri/src/state.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`
- Modify: `src/lib/ipc.ts`, `src/lib/useLiveAnalysis.ts`, `src/App.tsx` (passes `""` until Task 5)

**Interfaces:**
- Consumes:
  - `InstructionStore`, `InstructionsView`, `ProjectInstructions`, `Template` (Task 2)
  - The 7-argument `prompt::analysis_request` (Task 1)
- Produces:
  - **Rust commands:**
    - `get_instructions(root: String) -> InstructionsView`
    - `save_project_instructions(root: String, instructions: ProjectInstructions) -> Result<()>`
    - `save_templates(templates: Vec<Template>) -> Result<()>`
    - `restore_starter_templates() -> Result<Vec<Template>>`
    - `analyze(draft, previous_draft, instructions: String, on_event)`
  - **TS types:** `Template`, `ProjectInstructions`, `InstructionsView`.
  - **TS wrappers:**
    - `api.getInstructions(root)`
    - `api.saveProjectInstructions(root, instructions)`
    - `api.saveTemplates(templates)`
    - `api.restoreStarterTemplates()`
    - `api.analyze(draft, previousDraft, instructions, onEvent)`
  - **Hook:** `useLiveAnalysis(draft, debounceMs, contextKey, instructions: string)`

`src-tauri` has no tests. This task is verified by `cargo check`, clippy, the typecheck and vitest.

- [ ] **Step 1: State**

In `src-tauri/src/state.rs`, add `use livebrainstorm_core::instructions::InstructionStore;`. Add a field after `pub briefs: BriefStore,`:

```rust
    /// Session instructions and templates (`instructions.json`).
    pub instructions: InstructionStore,
```

In `AppState::new`, after `briefs: …,`, add:

```rust
            instructions: InstructionStore::new(config_dir.join("instructions.json")),
```

- [ ] **Step 2: Commands**

In `src-tauri/src/commands.rs`, add `use livebrainstorm_core::instructions::{InstructionsView, ProjectInstructions, Template};`. Add this section above `// ---------- live analysis ----------`:

```rust
// ---------- session instructions ----------

/// Never fails: a missing or unreadable file gives the starter templates, so
/// instructions can't block opening a project.
#[tauri::command]
pub fn get_instructions(state: State<'_, AppState>, root: String) -> InstructionsView {
    state.instructions.view(&root)
}

#[tauri::command]
pub fn save_project_instructions(state: State<'_, AppState>, root: String, instructions: ProjectInstructions) -> Result<()> {
    state.instructions.save_project(&root, instructions)
}

#[tauri::command]
pub fn save_templates(state: State<'_, AppState>, templates: Vec<Template>) -> Result<()> {
    state.instructions.save_templates(templates)
}

#[tauri::command]
pub fn restore_starter_templates(state: State<'_, AppState>) -> Result<Vec<Template>> {
    state.instructions.restore_starters()
}
```

Change the `analyze` signature to:

```rust
pub async fn analyze(
    app: AppHandle,
    draft: String,
    previous_draft: Option<String>,
    instructions: String,
    on_event: Channel<UiEvent>,
) -> Result<()> {
```

In its lane loop, replace the `""` from Task 1 with `&instructions`:

```rust
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, sections, &instructions);
```

In `src-tauri/src/lib.rs`, add these four lines to `generate_handler![…]`, after `commands::open_project,`:

```rust
            commands::get_instructions,
            commands::save_project_instructions,
            commands::save_templates,
            commands::restore_starter_templates,
```

- [ ] **Step 3: Check the shell**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: compiles with no warnings.

- [ ] **Step 4: IPC types and wrappers**

In `src/lib/ipc.ts`, add after the `JevStatus` interface:

```ts
/** Mirrors `instructions::Template`. Starters have fixed ids; new ones use crypto.randomUUID(). */
export interface Template {
  id: string;
  name: string;
  text: string;
}

/** One project's instructions (`instructions::ProjectInstructions`). */
export interface ProjectInstructions {
  default: string;
  current: string;
  templateId: string | null;
}

export interface InstructionsView {
  templates: Template[];
  project: ProjectInstructions;
}
```

In `api`, replace the `analyze` entry with the first entry below, and add the other four after `openProject`:

```ts
  analyze: (draft: string, previousDraft: string | null, instructions: string, onEvent: (e: UiEvent) => void) =>
    stream("analyze", { draft, previousDraft, instructions }, onEvent),
```

```ts
  getInstructions: (root: string) => invoke<InstructionsView>("get_instructions", { root }),
  saveProjectInstructions: (root: string, instructions: ProjectInstructions) =>
    invoke<void>("save_project_instructions", { root, instructions }),
  saveTemplates: (templates: Template[]) => invoke<void>("save_templates", { templates }),
  restoreStarterTemplates: () => invoke<Template[]>("restore_starter_templates"),
```

- [ ] **Step 5: The hook sends the instructions**

In `src/lib/useLiveAnalysis.ts`, change the signature and add a ref next to `context`:

```ts
export function useLiveAnalysis(draft: string, debounceMs: number, contextKey: string, instructions: string) {
```

```ts
  // Also read at call time: each request sends the instructions current when it starts.
  const instructionsRef = useRef(instructions);
  instructionsRef.current = instructions;
```

In `run`, change the `api.analyze(…)` call's first line to:

```ts
      .analyze(text, previousDraftFor(lastAnalyzed.current, rerun, ctx), instructionsRef.current, (ev) => {
```

In `src/App.tsx`, change the hook call to pass `""` for now. Task 5 wires the real state.

```ts
  const analysis = useLiveAnalysis(draft, settings?.debounceMs ?? 1200, `${project?.root ?? ""}|${brief?.createdAt ?? 0}`, "");
```

- [ ] **Step 6: Typecheck and test**

Run: `npm run typecheck && npm test && cargo test -p livebrainstorm-core`
Expected: no type errors, and all vitest and core tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src src/lib/ipc.ts src/lib/useLiveAnalysis.ts src/App.tsx
git commit -m "feat: instruction commands; analyze carries the session instructions

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Frontend rules: label, context key, actions, and project-keyed saving

**Files:**
- Create: `src/lib/instructions.ts`, `src/lib/saver.ts`
- Test: `src/lib/instructions.test.ts`, `src/lib/saver.test.ts`

**Interfaces:**
- Consumes: `ProjectInstructions`, `Template` (Task 3).
- Produces:
  - **`instructions.ts`:**
    - `MAX_INSTRUCTION_CHARS = 4000`
    - `EMPTY_INSTRUCTIONS: ProjectInstructions`
    - `instructionsLabel(p, templates): string`
    - `analysisContextKey(root: string | undefined, briefCreatedAt: number | undefined, instructions: string): string`
    - `counterText(length: number): { text: string; over: boolean }`
    - `firstLine(text: string): string`
    - `applyTemplate(p, t): { next: ProjectInstructions; undo: ProjectInstructions }`
    - `clearInstructions(p)`, `resetToDefault(p)`, `saveAsDefault(p)`, each returning `ProjectInstructions`
    - `upsertTemplate(list, t): Template[]`, `removeTemplate(list, id): Template[]`
  - **`saver.ts`:** `createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void): { schedule(key: string, value: T): void; flush(): Promise<void> }`

- [ ] **Step 1: Write the failing tests**

Create `src/lib/instructions.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { ProjectInstructions, Template } from "./ipc";
import {
  EMPTY_INSTRUCTIONS,
  analysisContextKey,
  applyTemplate,
  clearInstructions,
  counterText,
  firstLine,
  instructionsLabel,
  removeTemplate,
  resetToDefault,
  saveAsDefault,
  upsertTemplate,
} from "./instructions";

const t = (id: string, name: string, text: string): Template => ({ id, name, text });
const bug = t("bug-hunt", "Bug hunt", "Treat my draft as a hypothesis.");
const feat = t("feature-design", "Feature design", "I'm designing a new feature.");
const p = (fields: Partial<ProjectInstructions>): ProjectInstructions => ({ ...EMPTY_INSTRUCTIONS, ...fields });

describe("instructionsLabel", () => {
  it("covers every state in the spec's table", () => {
    expect(instructionsLabel(p({}), [bug])).toBe("No instructions");
    expect(instructionsLabel(p({ current: "  \n", templateId: "bug-hunt" }), [bug])).toBe("No instructions");
    expect(instructionsLabel(p({ current: bug.text, templateId: "bug-hunt" }), [bug])).toBe("Bug hunt");
    expect(instructionsLabel(p({ current: `${bug.text} Also logs.`, templateId: "bug-hunt" }), [bug])).toBe("Bug hunt (edited)");
    expect(instructionsLabel(p({ current: bug.text, templateId: "bug-hunt" }), [feat])).toBe("Custom");
    expect(instructionsLabel(p({ current: "d", default: "d" }), [bug])).toBe("Project default");
    expect(instructionsLabel(p({ current: "mine", default: "d" }), [bug])).toBe("Custom");
  });

  it("a template edited after it was applied reads as edited", () => {
    expect(instructionsLabel(p({ current: bug.text, templateId: "bug-hunt" }), [{ ...bug, text: "Different now." }])).toBe(
      "Bug hunt (edited)",
    );
  });
});

describe("analysisContextKey", () => {
  it("changes with the instructions, ignoring surrounding whitespace", () => {
    const key = analysisContextKey("/p", 1, "Focus on tests.");
    expect(analysisContextKey("/p", 1, "  Focus on tests.\n")).toBe(key);
    expect(analysisContextKey("/p", 1, "Focus on docs.")).not.toBe(key);
    expect(analysisContextKey("/p", 2, "Focus on tests.")).not.toBe(key);
    expect(analysisContextKey("/q", 1, "Focus on tests.")).not.toBe(key);
    expect(analysisContextKey(undefined, undefined, "")).toBe("|0|");
  });
});

describe("actions", () => {
  it("apply replaces the text and keeps what was there for undo", () => {
    const before = p({ current: "mine", default: "d" });
    const { next, undo } = applyTemplate(before, bug);
    expect(next).toEqual({ default: "d", current: bug.text, templateId: "bug-hunt" });
    expect(undo).toBe(before);
  });

  it("clear, reset to default, save as default", () => {
    const s = p({ default: "d", current: bug.text, templateId: "bug-hunt" });
    expect(clearInstructions(s)).toEqual({ default: "d", current: "", templateId: null });
    expect(resetToDefault(s)).toEqual({ default: "d", current: "d", templateId: null });
    expect(saveAsDefault(s)).toEqual({ default: bug.text, current: bug.text, templateId: "bug-hunt" });
  });

  it("templates: edit in place, add at the end, remove by id", () => {
    expect(upsertTemplate([feat, bug], { ...bug, name: "Bugs" }).map((x) => x.name)).toEqual(["Feature design", "Bugs"]);
    expect(upsertTemplate([feat], bug).map((x) => x.id)).toEqual(["feature-design", "bug-hunt"]);
    expect(removeTemplate([feat, bug], "feature-design")).toEqual([bug]);
  });

  it("counter and the default's first line", () => {
    expect(counterText(1234)).toEqual({ text: "1,234 / 4,000", over: false });
    expect(counterText(4001)).toEqual({ text: "4,001 / 4,000 (the rest is ignored)", over: true });
    expect(firstLine("\n  Focus on tests.\nMore")).toBe("Focus on tests.");
  });
});
```

Create `src/lib/saver.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSaver } from "./saver";

describe("createSaver", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("saves once, a moment after the last change", () => {
    const save = vi.fn((_key: string, _value: string) => Promise.resolve());
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "x");
    vi.advanceTimersByTime(300);
    s.schedule("/a", "xy");
    vi.advanceTimersByTime(499);
    expect(save).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("/a", "xy");
  });

  it("an edit pending for one project is saved to that project when another starts", () => {
    const save = vi.fn((_key: string, _value: string) => Promise.resolve());
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "typed in a");
    s.schedule("/b", "typed in b");
    expect(save).toHaveBeenCalledWith("/a", "typed in a");
    vi.advanceTimersByTime(500);
    expect(save).toHaveBeenLastCalledWith("/b", "typed in b");
    expect(save).toHaveBeenCalledTimes(2);
  });

  it("flush saves now, and only once", async () => {
    const save = vi.fn((_key: string, _value: string) => Promise.resolve());
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "x");
    await s.flush();
    await s.flush();
    vi.advanceTimersByTime(1000);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("/a", "x");
  });

  it("reports a failed save instead of throwing", async () => {
    const onError = vi.fn();
    const s = createSaver(() => Promise.reject(new Error("disk full")), 500, onError);
    s.schedule("/a", "x");
    await s.flush();
    expect(onError).toHaveBeenCalledWith(new Error("disk full"));
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/instructions.test.ts src/lib/saver.test.ts`
Expected: FAIL, `Failed to resolve import "./instructions"` and `"./saver"`.

- [ ] **Step 3: Create `src/lib/instructions.ts`**

```ts
// Session-instruction rules, kept pure so they're testable. Types mirror
// crates/livebrainstorm-core/src/instructions.rs; the cap mirrors
// MAX_INSTRUCTION_CHARS in prompt.rs.
import type { ProjectInstructions, Template } from "./ipc";

export const MAX_INSTRUCTION_CHARS = 4000;

export const EMPTY_INSTRUCTIONS: ProjectInstructions = { default: "", current: "", templateId: null };

/** The top-bar label: which template, edited or not, or where the text came from. */
export function instructionsLabel(p: ProjectInstructions, templates: Template[]): string {
  if (!p.current.trim()) return "No instructions";
  if (p.templateId !== null) {
    const t = templates.find((x) => x.id === p.templateId);
    if (!t) return "Custom";
    return t.text === p.current ? t.name : `${t.name} (edited)`;
  }
  return p.current === p.default ? "Project default" : "Custom";
}

/** Changing the instructions re-reads the same draft, like a new project or brief. */
export const analysisContextKey = (root: string | undefined, briefCreatedAt: number | undefined, instructions: string) =>
  `${root ?? ""}|${briefCreatedAt ?? 0}|${instructions.trim()}`;

const count = (n: number) => n.toLocaleString("en-US");

export function counterText(length: number): { text: string; over: boolean } {
  const over = length > MAX_INSTRUCTION_CHARS;
  const text = `${count(length)} / ${count(MAX_INSTRUCTION_CHARS)}`;
  return { text: over ? `${text} (the rest is ignored)` : text, over };
}

export const firstLine = (text: string) => text.trim().split("\n")[0];

/** Applying replaces the current text; `undo` is exactly what was there. */
export function applyTemplate(p: ProjectInstructions, t: Template): { next: ProjectInstructions; undo: ProjectInstructions } {
  return { next: { ...p, current: t.text, templateId: t.id }, undo: p };
}

export const clearInstructions = (p: ProjectInstructions): ProjectInstructions => ({ ...p, current: "", templateId: null });
export const resetToDefault = (p: ProjectInstructions): ProjectInstructions => ({ ...p, current: p.default, templateId: null });
export const saveAsDefault = (p: ProjectInstructions): ProjectInstructions => ({ ...p, default: p.current });

export function upsertTemplate(list: Template[], t: Template): Template[] {
  return list.some((x) => x.id === t.id) ? list.map((x) => (x.id === t.id ? t : x)) : [...list, t];
}

export const removeTemplate = (list: Template[], id: string) => list.filter((t) => t.id !== id);
```

- [ ] **Step 4: Create `src/lib/saver.ts`**

```ts
/**
 * Saves a value a moment after the last change. A pending save remembers its
 * own key (the project root), so switching projects can never save one
 * project's text under another's. `flush` saves now; call it before switching
 * projects and on unmount. Failures go to `onError`, never thrown.
 */
export function createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void) {
  let pending: { key: string; value: T } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const flush = (): Promise<void> => {
    if (timer) clearTimeout(timer);
    timer = null;
    const p = pending;
    pending = null;
    return p ? save(p.key, p.value).catch(onError) : Promise.resolve();
  };

  return {
    schedule(key: string, value: T) {
      if (pending && pending.key !== key) void flush(); // another project's edit goes out first
      pending = { key, value };
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void flush(), delayMs);
    },
    flush,
  };
}
```

- [ ] **Step 5: Run the tests and the typecheck**

Run: `npx vitest run src/lib/instructions.test.ts src/lib/saver.test.ts && npm test && npm run typecheck`
Expected: the new tests PASS (seven in `instructions.test.ts`, four in `saver.test.ts`), the suite passes, and there are no type errors.

- [ ] **Step 6: Commit**

```bash
git add src/lib/instructions.ts src/lib/instructions.test.ts src/lib/saver.ts src/lib/saver.test.ts
git commit -m "feat(ui): instruction label, context key, actions and project-keyed saving

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The Instructions drawer: picker, editor and actions, wired into the app

**Files:**
- Create: `src/components/InstructionsDrawer.tsx`
- Modify: `src/components/TopBar.tsx`, `src/App.tsx`

**Interfaces:**
- Consumes:
  - Every export of `src/lib/instructions.ts` and `createSaver` (Task 4)
  - `api.getInstructions`, `api.saveProjectInstructions` and the 4-argument `useLiveAnalysis` (Task 3)
  - `providerLabel` (Phase A)
- Produces:
  - `InstructionsDrawer` props: `{ instructions: ProjectInstructions; templates: Template[]; localModel: boolean; error: string | null; onChange: (next: ProjectInstructions) => void; onClose: () => void; children?: ReactNode }`. Task 6 passes `TemplateList` as `children`.
  - New `TopBar` props: `instructionsLabel: string`, `onShowInstructions: () => void`.
  - In `App.tsx`: state `templates`, setter `setTemplates`, and `setInstructionsError`, which Task 6 uses.

- [ ] **Step 1: Create `src/components/InstructionsDrawer.tsx`**

```tsx
import { useState, type ReactNode } from "react";
import type { ProjectInstructions, Template } from "../lib/ipc";
import { applyTemplate, clearInstructions, counterText, firstLine, resetToDefault, saveAsDefault } from "../lib/instructions";
import { Button, Field, cx, inputClass } from "./ui";

interface Props {
  instructions: ProjectInstructions;
  templates: Template[];
  localModel: boolean;
  error: string | null;
  onChange: (next: ProjectInstructions) => void;
  onClose: () => void;
  /** The templates section (TemplateList). */
  children?: ReactNode;
}

/** Same shell as BriefDrawer. App keys it by project root, so Undo never crosses projects. */
export function InstructionsDrawer(p: Props) {
  const [picked, setPicked] = useState("");
  const [undo, setUndo] = useState<ProjectInstructions | null>(null);
  const i = p.instructions;
  const pickedId = p.templates.some((t) => t.id === picked) ? picked : (p.templates[0]?.id ?? "");
  const counter = counterText(i.current.length);

  const change = (next: ProjectInstructions) => {
    setUndo(null);
    p.onChange(next);
  };
  const apply = () => {
    const t = p.templates.find((x) => x.id === pickedId);
    if (!t) return;
    const { next, undo } = applyTemplate(i, t);
    p.onChange(next);
    setUndo(undo);
  };

  return (
    <div
      className="absolute inset-y-0 right-0 flex w-[min(560px,100%)] flex-col border-l border-line bg-page shadow-[-12px_0_32px_rgb(0_0_0/0.08)]"
      role="dialog"
      aria-label="Session instructions"
    >
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-5">
        <h2 className="flex-1 text-[15px] font-semibold">Session instructions</h2>
        <Button variant="ghost" onClick={p.onClose} aria-label="Close instructions">
          Close
        </Button>
      </header>

      <div className="grid min-h-0 flex-1 content-start gap-5 overflow-y-auto px-6 pt-4 pb-10">
        <p className="text-xs text-muted">These steer the notes for this project: what LiveBrainstorm looks for, not the four sections.</p>

        {p.templates.length > 0 && (
          <div className="flex items-end gap-2">
            <Field label="Template">
              <select className={inputClass} value={pickedId} onChange={(e) => setPicked(e.target.value)}>
                {p.templates.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
              </select>
            </Field>
            <Button className="shrink-0" onClick={apply}>
              Apply
            </Button>
          </div>
        )}
        {undo && (
          <p className="flex items-center gap-2 text-xs text-muted">
            Template applied.
            <Button
              variant="link"
              onClick={() => {
                p.onChange(undo);
                setUndo(null);
              }}
            >
              Undo
            </Button>
          </p>
        )}

        <Field label="Instructions">
          <textarea
            className={cx(inputClass, "min-h-48 resize-y leading-relaxed")}
            value={i.current}
            placeholder="For example: focus Faults on error handling and race conditions."
            onChange={(e) => change({ ...i, current: e.target.value })}
          />
          <small className={cx("text-xs tabular-nums", counter.over ? "text-fault" : "text-faint")}>{counter.text}</small>
          {p.localModel && <small className="text-xs text-muted">Local models read fewer characters, depending on the model.</small>}
        </Field>

        <div className="grid gap-2">
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => change(clearInstructions(i))} disabled={!i.current}>
              Clear
            </Button>
            <Button onClick={() => change(resetToDefault(i))} disabled={i.current === i.default}>
              Reset to default
            </Button>
            <Button onClick={() => change(saveAsDefault(i))} disabled={i.current === i.default}>
              Save as default
            </Button>
          </div>
          <p className="truncate text-xs text-muted">{i.default.trim() ? `Default: ${firstLine(i.default)}` : "No default for this project"}</p>
        </div>

        {p.error && (
          <p className="text-[13px] text-fault" role="alert">
            {p.error}
          </p>
        )}
        {p.children}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Top-bar button**

In `src/components/TopBar.tsx`, add to `Props`:

```ts
  instructionsLabel: string;
  onShowInstructions: () => void;
```

Insert this immediately after the `{p.project && ( … brief button … )}` block, before `<span className="flex-1" />`:

```tsx
      <Button variant="ghost" className="min-w-0" onClick={p.onShowInstructions} title="Session instructions">
        <span className="max-w-64 truncate">Instructions: {p.instructionsLabel}</span>
      </Button>
```

- [ ] **Step 3: Wire it into `App.tsx`**

Replace `src/App.tsx` with the code below. It's the current file plus the instruction state, startup and loading, saving, drawer toggles and Esc.

```tsx
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type InstructionsView, type ProjectInstructions, type ProjectSummary, type Settings, type Template } from "./lib/ipc";
import { useLiveAnalysis } from "./lib/useLiveAnalysis";
import { activityLine } from "./lib/brief";
import { providerLabel } from "./lib/providerOptions";
import { EMPTY_INSTRUCTIONS, analysisContextKey, instructionsLabel } from "./lib/instructions";
import { createSaver } from "./lib/saver";
import { TopBar } from "./components/TopBar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { InstructionsDrawer } from "./components/InstructionsDrawer";
import { SettingsDialog } from "./components/SettingsDialog";

const LAST_PROJECT = "livebrainstorm.lastProject";
const draftKey = (root: string | undefined) => `livebrainstorm.draft.${root ?? "none"}`;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [opening, setOpening] = useState(false);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const [instructions, setInstructions] = useState<ProjectInstructions>(EMPTY_INSTRUCTIONS);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [instructionsError, setInstructionsError] = useState<string | null>(null);
  const [showInstructions, setShowInstructions] = useState(false);
  // Which project's instructions are on screen: null until one opens.
  const openRoot = useRef<string | null>(null);

  const [briefBuilding, setBriefBuilding] = useState(false);
  const [briefLive, setBriefLive] = useState("");
  const [briefActivity, setBriefActivity] = useState<string[]>([]);
  const [briefError, setBriefError] = useState<string | null>(null);
  const [showBrief, setShowBrief] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Edits save a moment after typing stops, each under the project it was typed in.
  const saver = useMemo(
    () =>
      createSaver<ProjectInstructions>(api.saveProjectInstructions, 500, (e) =>
        setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`),
      ),
    [],
  );
  useEffect(() => () => void saver.flush(), [saver]);

  // Opening (or re-opening) a project rescans it, so its summary is the
  // single source of truth for the brief and whether it's stale.
  const brief = project?.brief ?? null;
  const briefStatus = project?.briefStatus ?? null;
  // Changing the instructions re-reads the same draft, like a new project or brief.
  const analysis = useLiveAnalysis(
    draft,
    settings?.debounceMs ?? 1200,
    analysisContextKey(project?.root, brief?.createdAt, instructions.current),
    instructions.current,
  );

  const showView = (view: InstructionsView) => {
    setInstructions(view.project);
    setTemplates(view.templates);
    setInstructionsError(null);
  };

  const loadProject = useCallback(
    async (path: string) => {
      setOpening(true);
      setProjectError(null);
      await saver.flush(); // a pending edit belongs to the project it was typed in
      try {
        const summary = await api.openProject(path);
        // Fetched before any state changes, so the project and its instructions
        // land in one render and the first analysis already has them.
        const view = await api.getInstructions(summary.root);
        openRoot.current = summary.root;
        setProject(summary);
        showView(view);
        setBriefError(null);
        setDraft(localStorage.getItem(draftKey(summary.root)) ?? "");
        localStorage.setItem(LAST_PROJECT, summary.root);
      } catch (e) {
        setProjectError(errorMessage(e));
      } finally {
        setOpening(false);
      }
    },
    [saver],
  );

  useEffect(() => {
    api.getSettings().then(setSettings);
    void api.warmUp();
    // Templates are shared, so load them even if the last project fails to
    // open (a template save writes the whole list). The no-project
    // instructions apply until a project opens.
    void api.getInstructions("").then((view) => {
      setTemplates(view.templates);
      if (openRoot.current === null) setInstructions(view.project);
    });
    const last = localStorage.getItem(LAST_PROJECT);
    if (last) void loadProject(last);
    else setDraft(localStorage.getItem(draftKey(undefined)) ?? "");
  }, [loadProject]);

  // Drafts are per project and survive restarts.
  useEffect(() => {
    localStorage.setItem(draftKey(project?.root), draft);
  }, [draft, project?.root]);

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

  const pickProject = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir === "string") await loadProject(dir);
  };

  const changeInstructions = (next: ProjectInstructions) => {
    setInstructions(next);
    setInstructionsError(null);
    saver.schedule(project?.root ?? "", next);
  };

  const buildBrief = () => {
    setShowInstructions(false);
    setShowBrief(true);
    setBriefBuilding(true);
    setBriefLive("");
    setBriefActivity([]);
    setBriefError(null);
    api
      .buildBrief((ev) => {
        if (ev.type === "text") setBriefLive((t) => t + ev.text);
        else if (ev.type === "activity") setBriefActivity((a) => [...a, activityLine(ev.label, ev.lane)]);
        else if (ev.type === "done") {
          setBriefBuilding(false);
          // Rescan: picks up the saved brief and any files changed meanwhile.
          if (project) void loadProject(project.root);
        } else if (ev.type === "error") {
          setBriefBuilding(false);
          setBriefError(ev.message);
        }
      })
      .catch((e) => {
        setBriefBuilding(false);
        setBriefError(errorMessage(e));
      });
  };

  const saveSettings = async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
  };

  return (
    <div className="flex h-full flex-col">
      <TopBar
        project={project}
        opening={opening}
        briefStatus={briefStatus}
        briefBuilding={briefBuilding}
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
      {projectError && (
        <p className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-fault" role="alert">
          {projectError}
        </p>
      )}

      {/* Both panes fill the remaining height and scroll on their own. */}
      <main className="relative grid min-h-0 flex-1 grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]">
        <DraftPane value={draft} onChange={setDraft} hasProject={!!project} />
        <NotesPane analysis={analysis} hasProject={!!project} briefMissing={!brief && !briefBuilding} onBuildBrief={buildBrief} />
        {showBrief && project && (
          <BriefDrawer
            brief={brief}
            status={briefStatus}
            building={briefBuilding}
            liveText={briefLive}
            activity={briefActivity}
            error={briefError}
            onBuild={buildBrief}
            onClose={() => setShowBrief(false)}
          />
        )}
        {showInstructions && (
          <InstructionsDrawer
            key={project?.root ?? ""}
            instructions={instructions}
            templates={templates}
            localModel={settings?.provider.kind === "local"}
            error={instructionsError}
            onChange={changeInstructions}
            onClose={() => setShowInstructions(false)}
          />
        )}
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
```

- [ ] **Step 4: Typecheck and test**

Run: `npm run typecheck && npm test`
Expected: no type errors (`showView` is used, and every new prop is passed), and all vitest tests pass.

- [ ] **Step 5: Manual check in the running app (the user runs `npm run tauri dev`)**

1. The top bar shows "Instructions: No instructions". Clicking it opens the drawer, and clicking the brief button switches to the brief drawer: only one is open at a time. Esc closes either.
2. Pick "Bug hunt" and click Apply: the textarea fills, the label reads "Instructions: Bug hunt", "Template applied. Undo" appears, and the notes refresh after one pause. Undo restores the previous text.
3. Edit one word: the label reads "Bug hunt (edited)". Click Save as default, then Clear ("No instructions"), then Reset to default: the text returns and the label reads "Project default".
4. Restart the app: the same instructions come back. Open a second project: it starts with "No instructions", and its drawer has no Undo left over from the first. Switch back: the first project's text is intact.
5. Type more than 4,000 characters: the counter turns red and says the rest is ignored.

- [ ] **Step 6: Commit**

```bash
git add src/components/InstructionsDrawer.tsx src/components/TopBar.tsx src/App.tsx
git commit -m "feat(ui): Instructions drawer with templates, reset and save as default

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Managing templates, plus the docs

**Files:**
- Create: `src/components/TemplateList.tsx`
- Modify: `src/App.tsx` (template handlers; `TemplateList` as the drawer's children)
- Modify: `README.md`, `AGENTS.md`

**Interfaces:**
- Consumes:
  - `upsertTemplate` and `removeTemplate` (Task 4)
  - `api.saveTemplates` and `api.restoreStarterTemplates` (Task 3)
  - `templates`, `setTemplates` and `setInstructionsError` in `App.tsx`, and the drawer's `children` slot (Task 5)
- Produces: `TemplateList` props `{ templates: Template[]; current: string; onSave: (next: Template[]) => void; onRestore: () => void }`.

- [ ] **Step 1: Create `src/components/TemplateList.tsx`**

```tsx
import { useState } from "react";
import type { Template } from "../lib/ipc";
import { removeTemplate, upsertTemplate } from "../lib/instructions";
import { Button, Eyebrow, Field, cx, inputClass } from "./ui";

interface Props {
  templates: Template[];
  /** The current instructions: a new template starts from them. */
  current: string;
  onSave: (next: Template[]) => void;
  onRestore: () => void;
}

/** Add, edit, delete and restore templates. Confirmations are inline, never browser dialogs. */
export function TemplateList({ templates, current, onSave, onRestore }: Props) {
  const [editing, setEditing] = useState<Template | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const isNew = editing !== null && !templates.some((t) => t.id === editing.id);

  const commit = () => {
    if (!editing?.name.trim()) return;
    onSave(upsertTemplate(templates, { ...editing, name: editing.name.trim() }));
    setEditing(null);
  };
  const editor = editing && <TemplateEditor value={editing} onChange={setEditing} onSave={commit} onCancel={() => setEditing(null)} />;

  return (
    <section className="grid gap-3 border-t border-line pt-5" aria-label="Templates">
      <div className="flex items-center justify-between">
        <h3>
          <Eyebrow>Templates</Eyebrow>
        </h3>
        <Button variant="ghost" onClick={() => setEditing({ id: crypto.randomUUID(), name: "", text: current })} disabled={editing !== null}>
          + New template
        </Button>
      </div>
      {isNew && editor}
      <ul className="grid gap-1">
        {templates.map((t) =>
          editing?.id === t.id ? (
            <li key={t.id}>{editor}</li>
          ) : deleting === t.id ? (
            <li key={t.id} className="flex items-center gap-2 text-[13px]">
              <span className="min-w-0 flex-1 truncate">Delete {t.name}?</span>
              <Button
                onClick={() => {
                  onSave(removeTemplate(templates, t.id));
                  setDeleting(null);
                }}
              >
                Yes
              </Button>
              <Button variant="ghost" onClick={() => setDeleting(null)}>
                No
              </Button>
            </li>
          ) : (
            <li key={t.id} className="flex items-center gap-2 text-[13px]">
              <span className="min-w-0 flex-1 truncate">{t.name}</span>
              <Button variant="ghost" onClick={() => setEditing(t)} disabled={editing !== null}>
                Edit
              </Button>
              <Button variant="ghost" onClick={() => setDeleting(t.id)}>
                Delete
              </Button>
            </li>
          ),
        )}
      </ul>
      {templates.length === 0 && <p className="text-xs text-muted">No templates.</p>}
      <div>
        <Button variant="link" onClick={onRestore}>
          Restore starter templates
        </Button>
      </div>
    </section>
  );
}

function TemplateEditor(props: { value: Template; onChange: (t: Template) => void; onSave: () => void; onCancel: () => void }) {
  const v = props.value;
  return (
    <div className="grid gap-2 rounded-lg border border-line p-3">
      <Field label="Name">
        <input className={inputClass} value={v.name} onChange={(e) => props.onChange({ ...v, name: e.target.value })} autoFocus />
      </Field>
      <Field label="Instructions">
        <textarea
          className={cx(inputClass, "min-h-32 resize-y leading-relaxed")}
          value={v.text}
          onChange={(e) => props.onChange({ ...v, text: e.target.value })}
        />
      </Field>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={props.onCancel}>
          Cancel
        </Button>
        <Button variant="primary" onClick={props.onSave} disabled={!v.name.trim()}>
          Save template
        </Button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Wire it into `App.tsx`**

In `src/App.tsx`, add `import { TemplateList } from "./components/TemplateList";`. Add these handlers after `changeInstructions`:

```tsx
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
```

Replace the self-closing `<InstructionsDrawer … />` element with:

```tsx
          <InstructionsDrawer
            key={project?.root ?? ""}
            instructions={instructions}
            templates={templates}
            localModel={settings?.provider.kind === "local"}
            error={instructionsError}
            onChange={changeInstructions}
            onClose={() => setShowInstructions(false)}
          >
            <TemplateList templates={templates} current={instructions.current} onSave={changeTemplates} onRestore={restoreStarters} />
          </InstructionsDrawer>
```

- [ ] **Step 3: Typecheck and test**

Run: `npm run typecheck && npm test`
Expected: no type errors and all tests pass.

- [ ] **Step 4: Update the README**

In `README.md`, insert this new subsection after the second Jev paragraph (the one ending "…which promises nothing leaves your computer.") and before `### Requirements for the Claude subscription option`:

```markdown
### Session instructions

Click **Instructions** in the top bar to steer the notes for the project you have open, for example "focus Faults on error handling". Start from a template (Feature design, Bug hunt, Refactor or architecture, Product and UX, Devil's advocate), edit it, or write your own. Each project keeps its current instructions and a default you can reset to; new projects start with none. Instructions change what the notes look for, never their four sections. Add, edit and delete templates in the same drawer; *Restore starter templates* brings back any you deleted.
```

- [ ] **Step 5: Update AGENTS.md**

In "UI and styling", replace `Esc for the brief drawer is handled in `App.tsx`.` with:

```markdown
Esc for the brief and instructions drawers is handled in `App.tsx`; only one drawer is open at a time.
```

In "Live analysis flow", append to the end of step 1:

```markdown
 The session instructions are part of `contextKey` (`analysisContextKey` in `src/lib/instructions.ts`), so editing them re-reads the same draft without the gate, and every `analyze` call carries the current text.
```

In step 3, replace `builds the prompt with `prompt::analysis_request` and streams.` with:

```markdown
builds the prompt with `prompt::analysis_request` (which puts the session instructions in every lane's system prompt, before the rules, clipped to `min(MAX_INSTRUCTION_CHARS, budget / 10)` characters) and streams.
```

In "Persistence", add after the `- API key:` line:

```markdown
- Session instructions and templates: `<app_config>/instructions.json` (`instructions::InstructionStore`), written atomically under a mutex. A missing file starts with the five starter templates. An unreadable one is renamed to `instructions.json.bad` rather than overwritten, because it holds the user's writing. Projects are keyed by root path, and `""` is the session with no project open. The frontend saves edits 500 ms after typing stops (`src/lib/saver.ts`) and flushes before switching projects.
```

In "Cross-language contracts", add after the `- **Check events:**` line:

```markdown
- **Session instructions:** `Template` / `ProjectInstructions` / `InstructionsView` in `instructions.rs` ↔ `src/lib/ipc.ts`; `MAX_INSTRUCTION_CHARS` in `prompt.rs` ↔ `src/lib/instructions.ts`.
```

In the `- **Lanes:**` line, replace `` `analysis_system(&SECTIONS)` must stay byte-identical`` with `` `analysis_system(&SECTIONS, "")` (no instructions) must stay byte-identical``.

In "Extension points", add:

```markdown
- Starter templates: `STARTERS` in `crates/livebrainstorm-core/src/instructions.rs`. Existing users get new ones through *Restore starter templates*.
```

- [ ] **Step 6: Manual check in the running app (the user runs `npm run tauri dev`)**

1. *+ New template*: the editor opens with the current instructions. Name it "Perf" and save: it appears last in the list and in the Template picker.
2. *Edit* "Perf", change the name and text, and save: both update. *Cancel* discards changes.
3. *Delete* a starter: the row asks "Delete Bug hunt? Yes / No". No keeps it; Yes removes it. If it was the applied template, the label now reads "Custom".
4. Delete every template: "No templates." shows and the picker disappears. *Restore starter templates* brings all five back, and a template you edited keeps your edits.
5. Restart: the templates survive. Then corrupt `~/Library/Application Support/app.livebrainstorm.desktop/instructions.json` (macOS config dir) by hand and restart: the app opens with the starters, and `instructions.json.bad` holds the old content.

- [ ] **Step 7: Final checks and commit**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets && npm run typecheck && npm test`
Expected: everything passes, with no clippy warnings.

```bash
git add src/components/TemplateList.tsx src/App.tsx README.md AGENTS.md
git commit -m "feat(ui): add, edit, delete and restore instruction templates; docs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
