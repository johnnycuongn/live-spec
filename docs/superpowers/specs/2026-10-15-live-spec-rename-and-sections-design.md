# Live Spec: rename, spec-oriented sections, per-tab section choice

Date: 2026-10-15 · Status: approved in conversation, awaiting written-spec review

## Goal

LiveBrainstorm becomes **Live Spec**. The draft on the left is a spec the user is writing for an AI coding agent to implement against the open project. The notes on the right make that spec precise enough that the agent needs no further discovery or guessing: grounded in the real code, unambiguous, complete on what matters, and no longer than it needs to be. Fewer tokens and fewer clarifying rounds once the spec is handed over.

Three changes, built as one project:

1. **Rename**, all the way down to the bundle identifier, with a one-time migration of the data folder and the keychain entries.
2. **A catalogue of spec-oriented sections** replaces the four fixed brainstorming sections. The prompt, parser, lanes and fact-check key on the catalogue.
3. **Each tab chooses its sections** from the catalogue. Templates carry a section set as well as instruction text.

Plus one small addition: **Copy spec** puts the draft on the clipboard.

## Research that shaped the sections

From Addy Osmani's "How to write a good spec for AI agents" (Jan 2026), GitHub Spec Kit and Kiro:

- A spec states the what and why, names the user and what success looks like, and leaves the how to a plan.
- Every ambiguity is marked, never guessed at. Spec Kit's `[NEEDS CLARIFICATION: …]` markers must all be resolved before a spec is complete.
- Acceptance criteria are written so they can become tests. Non-goals and boundaries are explicit, because agents expand scope when a boundary is unwritten.
- Specs are short and scaled to the task; vagueness is the leading failure in GitHub's study of 2,500 agent files.
- The spec should be specific about what already exists: structure, conventions, files touched.

Live Spec already grounds notes against the codebase, flags conflicts and asks questions. The new sections add the two things the research stresses most: ambiguities to resolve, and gaps an implementing agent would hit.

## Decisions (from the brainstorm)

| Topic | Decision |
|---|---|
| How sections are chosen | A fixed catalogue; a tab picks a subset. Headings, colours and the fact-check hook stay predictable. |
| Where the choice lives | **Per tab**, stored in the tab file. Templates carry a set too. |
| Catalogue | Intent, Conflicts with code, Gaps, Ambiguities, Questions. Directions, "Where it lands" and "Tighten" were considered and dropped. |
| Default set | All five. |
| Getting the spec to the agent | **Copy to clipboard** only. No file writing. |
| Rename depth | **Everything, with migration**: product name, identifier, keychain service, crates, packages, UI, prompts, docs. |

## Non-goals

- Custom headings or guide text, reordering sections, or sections beyond the five.
- Saving the spec into the project folder, or any other export.
- Changes to the brief, the Jev gate or the rerank.
- Enforcing a spec notation (EARS, Spec Kit templates).
- Rewriting the dated documents under `docs/superpowers`; they are records.
- Migrating webview storage (the sidebar preference). It is per bundle id and falls back to the width rule.

## Success criteria

- Launching Live Spec over an existing LiveBrainstorm install keeps every tab, draft, saved analysis, brief, setting, usage record and API key, with no action from the user beyond at most one macOS Keychain prompt.
- A tab with the default sections and blank instructions does not re-run at launch after the upgrade.
- The single-lane prompt with all five sections and blank instructions is byte-pinned by a test, as today's four-section prompt is.
- Toggling a section on the shown tab re-reads the draft after one pause with no gate. On a hidden tab it waits until the tab is shown.
- Saved notes from the old build, with `## Faults` and `## Directions` headings, still render in history.
- With Conflicts unselected, no `checking` event is sent.
- Every test suite passes and `cargo clippy --workspace --all-targets` stays warning-free.

---

## Part 1: Rename and migration

### Names

| Where | Old | New |
|---|---|---|
| `tauri.conf.json` `productName` | LiveBrainstorm | Live Spec |
| `tauri.conf.json` `identifier` | `app.livebrainstorm.desktop` | `app.livespec.desktop` |
| Keychain service (`secrets::SERVICE`) | `app.livebrainstorm.desktop` | `app.livespec.desktop` |
| Core crate | `livebrainstorm-core` | `livespec-core` |
| Shell package / lib | `livebrainstorm` / `livebrainstorm_lib` | `livespec` / `livespec_lib` |
| npm package | `livebrainstorm` | `livespec` |
| Window title (`windowTitle`) | `LiveBrainstorm — <tab>` | `Live Spec — <tab>` |
| Sidebar storage key | `livebrainstorm.sidebar.<windowId>` | `livespec.sidebar.<windowId>` |
| stderr prefixes | `LiveBrainstorm:` | `Live Spec:` |
| Prompt identity | "You are LiveBrainstorm, a sharp brainstorming partner…" | "You are Live Spec, …" (Part 2) |

Every user-visible string, comment and doc that says LiveBrainstorm changes, including README.md, AGENTS.md and CLAUDE.md, which are rewritten for the new purpose. The legacy draft import keys (`livebrainstorm.draft.*`, `livebrainstorm.lastProject`) stay as they are: they name the pre-tabs build's storage, and that code path only runs when `workspace.json` is missing.

### Data folder migration

Tauri resolves `app_data_dir()` and `app_config_dir()` as `<base>/<identifier>`. A new core module `migrate.rs` holds the path derivation and the rename-or-copy as functions over plain paths (the shell has no tests, so the logic lives in core). The shell calls `migrate::move_app_dirs(&[data_dir, config_dir], OLD_IDENTIFIER)` in `setup`, before `AppState::new`:

1. For each of the two resolved paths, derive the old path by replacing the final component with `app.livebrainstorm.desktop`. If the two new paths are the same folder (macOS), handle it once.
2. If the new folder does not exist and the old one does, `fs::rename(old, new)`. This is one atomic directory move, so the local runtime and models move without copying.
3. If the rename fails (for example across devices), copy recursively into the new folder and leave the old one in place.
4. Log each step to stderr with the `Live Spec:` prefix. No error stops the launch: the app continues with whatever folder exists, and the old data is never deleted.

The path derivation and rename-or-copy are pure functions over paths, tested on temp dirs. The window-state plugin's file lives in the config folder, so window positions move with it.

### Keychain migration

`secrets::migrate()` runs from `setup` after the folder move. For each account (`anthropic-api-key`, `typesafe-api-key`): if the new service has no entry and the old service has one, write it under the new service. The old entry is left alone. Errors are logged, never surfaced. On macOS the first read of the old item may show one Keychain prompt because the app's bundle changed.

---

## Part 2: Section catalogue and prompt

### Catalogue

A new core module `sections.rs` (re-exported through `prompt`) holds the catalogue, in this fixed order:

| id | Heading the model writes | UI title | Dot colour | Fact-checked |
|---|---|---|---|---|
| `intent` | Intent | Intent | `bg-muted` (grey) | no |
| `conflicts` | Conflicts | Conflicts with code | `bg-fault` (red) | **yes** |
| `gaps` | Gaps | Gaps | `bg-accent` (blue) | no |
| `ambiguities` | Ambiguities | Ambiguities | `bg-question` (amber) | no |
| `questions` | Questions | Questions for you | `bg-question` (amber) | no |

Amber means an open item; blue means something to add; red means a clash with the code. `DEFAULT_SECTIONS` is all five in this order.

```rust
pub struct SectionSpec { pub id: &'static str, pub heading: &'static str, pub guide: &'static str, pub checkable: bool }
pub const CATALOGUE: [SectionSpec; 5];
pub const DEFAULT_SECTIONS: [&str; 5];
/// Keeps known ids, in catalogue order, deduplicated; empty → the default set.
pub fn resolve(ids: &[String]) -> Vec<&'static SectionSpec>;
```

### Guide text (the prompt's per-section instruction)

- **Intent:** "What the spec asks for, for whom, and what done looks like, in one or two bullets. If the spec doesn't say, name what it fails to say."
- **Conflicts:** "Where the spec contradicts how the project works, re-does something that exists, or assumes wrongly. Cite file paths when the project context supports it. If nothing is worth raising, write \"- Nothing so far.\""
- **Gaps:** "What an agent implementing this spec would need but can't find in it: acceptance criteria it could turn into tests, non-goals, error and edge cases, data or state the project doesn't have, commands to run. Name the gap, not a fix."
- **Ambiguities:** "Phrases an agent would have to guess at. Quote the phrase, give the two readings, and propose precise wording."
- **Questions:** "One to three questions whose answers would most change the spec."

### Intro and rules

`ANALYSIS_INTRO` becomes:

> You are Live Spec, a reviewer sitting beside a software engineer who is writing a spec that an AI coding agent will implement against the project described in `<project>`. The spec is unfinished and typed in real time. Your job is to make it precise enough that the agent needs no further discovery or guessing: grounded in how the code actually works, unambiguous, complete on what matters, and no longer than it needs to be. Never rewrite the spec.

`ANALYSIS_RULES` keeps "bullets only, at most 3 per section, each under 35 words", grounding in the project context, "Never restate the spec", "Do not use tools", and adds "Quote the spec only to point at a phrase."

### Prompt API

- `analysis_system(sections: &[&SectionSpec], instructions: &str)` replaces the `&[&str]` form. The lead-in counts the sections it lists ("exactly these three sections, in this order, and nothing else") for any length from 1 to 5.
- `section_groups(n: usize, chosen: &[&SectionSpec]) -> Vec<Vec<&SectionSpec>>` splits `chosen` into `min(n, chosen.len())` contiguous groups of near-equal size (earlier groups take the remainder). With one group it is the whole list.
- `analysis_request(…, sections: &[&SectionSpec], instructions)` as today.
- The pinned baseline test becomes `all_five_sections_is_the_single_lane_prompt`: `analysis_system(&resolve(&[]), "")` equals the single-agent prompt byte for byte.

### Section parsing (Rust ↔ TypeScript contract)

Both parsers key on the heading's first word, case-insensitively, with the same alias table:

| Heading word starts with | id |
|---|---|
| `intent` | `intent` |
| `conflict`, `fault` | `conflicts` |
| `gap` | `gaps` |
| `ambigu` | `ambiguities` |
| `question` | `questions` |
| `direction` | `directions` (TypeScript only, display-only legacy) |

`CATALOGUE` (ids, order, titles, dots) and `DEFAULT_SECTIONS` in Rust ↔ `SECTIONS` and `DEFAULT_SECTIONS` in `src/lib/sections.ts`, kept in sync by hand like the other IPC contracts; both test suites pin the id list. `prompt::section_body(markdown, id)` takes the id. The TypeScript `SECTIONS` list adds `{ key: "directions", title: "Directions", legacy: true }` with the `bg-accent` dot; it is never requested, only rendered when a saved analysis contains it. `MiniMarkdown` is unchanged.

---

## Part 3: Per-tab sections

### Data model

`Tab` gains `sections: Vec<String>` with `#[serde(default)]`: an empty list means the default set, so every existing tab file loads unchanged. `Snapshot` is unchanged.

`SessionStore`:
- `set_sections(id, Vec<String>) -> Result<()>`: normalises with `sections::resolve` (unknown ids dropped, catalogue order, deduplicated) and stores the ids. A list that resolves to nothing is `Precondition("Keep at least one section.")`. Writes the tab atomically like `rename_tab`.
- `create_tab(window, root, sections: Vec<String>)`: the new tab starts with the given list, normalised the same way.

### Commands and IPC

- `set_tab_sections(tab_id, sections)`: `check_id`, then the store call. No `workspace-changed`: a tab belongs to one window and `TabSummary` doesn't carry sections.
- `create_tab(window, root, sections)`: the frontend passes the shown tab's sections when the new tab is for the shown tab's project (⌘T, and the + beside that project), else `[]`.
- `analyze(tab_id, draft, previous_draft, instructions, sections, context)`: the shell resolves `sections` with the catalogue before `section_groups`, so an empty or stale list still runs with the default set.
- `ipc.ts` mirrors: `Tab.sections: string[]`, `createTab(window, root, sections)`, `setTabSections(tabId, sections)`, and `analyze` with `sections`.

### Context key and re-runs

The project key stays `${root}|${briefCreatedAt}|${instructions}`. `TabSession` keeps the project key and its own `sections` separately and recombines them whenever either changes:

```ts
// src/lib/instructions.ts
export const tabContextKey = (projectKey: string, sections: string[]) =>
  isDefaultSections(sections) ? projectKey : `${projectKey}|${sections.join(",")}`;
```

Default sections add nothing to the key, so a tab saved by the old build has the same key as before and starts no run at launch. `TabSession.setSections(ids)` stores the ids and, if the combined key changed, follows the same path as a new key from `setContext`: the shown tab re-reads the draft after one pause with no gate; a hidden tab sets `contextDirty` and runs when shown. `tabs.setSections(tabId, ids)` calls the command, updates the tab in the store's view, and calls the session.

### Fact-check

In the analysis task the oneshot is created only when a lane carries `conflicts`; `section_body(&text, "conflicts")` feeds `check::run`. With Conflicts unselected nothing is sent on the `checking` channel. `checkSummary` says "conflict" / "conflicts". `Snapshot.checksFaults` keeps its name (a serde contract) and its doc comment says it is the Conflicts count. The Jev switch label reads "Fact-check Conflicts against the code".

### Lanes

`commands::analyze` groups the resolved sections with `section_groups(settings.effective_note_agents(), &chosen)`. `analysis::merge_lanes` takes groups of `&SectionSpec` and reports missing sections by heading. `src/lib/lanes.ts` is unchanged: lane order is still group order.

---

## Part 4: UI, templates and copy

### Section picker

- A `Sections ▾` mini button sits at the top of the notes pane, right of the status line. It opens the shared `Menu` at its position. `MenuItem` gains `checked?: boolean`, rendered as a leading ✓ and `aria-checked` with `role="menuitemcheckbox"`; choosing an item toggles it and keeps the menu open.
- The five catalogue entries appear in catalogue order with their UI titles. The last remaining checked entry is `disabled`.
- Over an older analysis (`viewing > 0`) the button is disabled: the sections shown are whatever that snapshot has.
- `NotesPane` renders sections in catalogue order from the parsed markdown, as today, including a legacy Directions section when present.

### Copy spec

A `Copy spec` link button in the draft footer, beside the word count. It calls `navigator.clipboard.writeText(draft)`; on success the footer shows "Copied" for two seconds, on failure "Couldn't copy. Select the text and copy it instead." Disabled when the draft is empty. No plugin and no capability entry.

### Templates

- `Template` gains `sections: Vec<String>` (`#[serde(default)]`). Empty means "leave the tab's sections alone", so templates from existing files behave as before.
- `applyTemplate` returns the next instructions and the next sections (the template's, or the tab's current ones when the template has none); `undo` restores both. The drawer calls `tabs.setSections` for the shown tab when they differ.
- The template editor gets the five toggles under the text, in the same style as the picker. "+ New template" starts from the current instructions and the shown tab's sections.
- The drawer's lead text becomes: "These steer the notes for this project: what Live Spec looks for. Which sections appear is chosen per tab, above the notes."

### Starter templates

`STARTERS` becomes five spec-oriented templates. Ids are new except Devil's advocate, which keeps `devils-advocate` so an edited copy is never overwritten by *Restore starter templates*.

| id | Name | Sections | Text |
|---|---|---|---|
| `feature-spec` | Feature spec | all five | "I'm writing a spec for a new feature that an AI agent will implement.<br>- Conflicts: features or code that already do part of this, integration points the spec skips, and data or state the project doesn't have yet.<br>- Gaps: acceptance criteria an agent could turn into tests, and non-goals.<br>- Questions: who this is for and what done looks like." |
| `bugfix-spec` | Bug fix spec | intent, conflicts, gaps, questions | "This spec describes a bug and its fix. Treat the described cause as a hypothesis.<br>- Conflicts: code paths that contradict the described cause or the described current behaviour.<br>- Gaps: current behaviour, expected behaviour, what must not change, and the test that proves the fix.<br>- Questions: the evidence that would confirm or rule out the cause." |
| `refactor-spec` | Refactor spec | conflicts, gaps, ambiguities, questions | "This spec describes a refactor or an architectural change.<br>- Conflicts: callers, contracts and invariants the change would break, including anything the project's docs say to preserve.<br>- Gaps: an order of steps that keeps the code working after each one, and how we'd know nothing regressed.<br>- Questions: what must not change." |
| `handoff` | Hand-off pass | ambiguities, gaps | "This spec is nearly ready to hand to an agent. Hunt for anything it would have to guess: vague verbs, unnamed files, \"etc.\", implied defaults, missing error cases. Flag what it needs to start without asking." |
| `devils-advocate` | Devil's advocate | conflicts, questions | "Be a skeptical reviewer.<br>- Conflicts: give the strongest objection to the plan rather than a list of small issues.<br>- Questions: ask what evidence would change my mind." |

### Copy changes

- Draft placeholders: with a project, "Write the spec here. Live Spec reads along and replies when you pause."; without, "Open the project the spec is for, so Live Spec can check it against the code, or just start writing."
- Notes empty state: "Start writing your spec on the left. When you pause, notes appear here:" followed by one sentence built from the chosen sections' titles.
- Top bar brand: "Live Spec". Settings group "Where Live Spec thinks" and the other settings strings follow the rename.
- README: the opening paragraph describes spec writing; the Session instructions section documents templates with sections; a new Sections paragraph documents the picker and Copy spec; the Jev paragraph says Conflicts.
- AGENTS.md: "What this is", the live-analysis flow, the section contract rows and the instructions rows are updated; the new `sections` contract (catalogue ↔ `SECTIONS` in `sections.ts`, alias table in both parsers) is added.

---

## Testing

**Rust (`livespec-core`)**
- `sections::resolve`: unknown ids dropped, order normalised, duplicates removed, empty → default.
- `section_groups`: lengths 1–5 for n in 1..=4, contiguous, near-equal, never more groups than sections.
- `analysis_system` for subsets lists only those headings; the full-default no-instructions prompt is byte-pinned.
- `section_body` with every alias, including `Faults` → `conflicts`.
- `Tab` without `sections` deserialises to an empty list; `set_sections` validation and the "at least one" precondition.
- `migrate`: old path derivation; rename when the new folder is missing; nothing when it exists; copy fallback.
- `Template` without `sections` deserialises to empty; starters have the sets above.

**TypeScript**
- `parseSections` with the five headings, `**Conflicts:**`, `### Faults`, and a legacy `## Directions`.
- `tabContextKey` adds nothing for the default set and a suffix otherwise.
- `TabSession.setSections`: shown tab re-schedules without the gate; hidden tab marks `contextDirty`; the same set is a no-op.
- Toggle rules: the last checked item is disabled.
- `applyTemplate` with and without sections, and undo.
- `checkSummary` wording.

**Manual**
- Launch over an existing LiveBrainstorm install: tabs, drafts, history, briefs, settings, usage and keys survive; one Keychain prompt at most.
- Toggle sections while notes stream; browse history with old Faults/Directions notes.
- Copy spec, then paste into Claude Code.

## Implementation order (for the plan)

1. Catalogue and prompt in core, with the parser alias tables on both sides.
2. Per-tab sections: `Tab`, store, commands, `analyze`, `TabSession`, context key.
3. UI: picker, templates with sections, starters, Copy spec, copy changes.
4. Rename everything, migration module, docs.

The rename goes last so the earlier steps land against stable names and the migration is tested against real data the earlier steps produced.
