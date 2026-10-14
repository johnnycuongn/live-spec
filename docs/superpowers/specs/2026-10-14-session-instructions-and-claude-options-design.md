# Session instructions and Claude options: design

Date: 2026-10-14 · Status: approved in conversation, awaiting written-spec review

## Goal

Two independent features, built as two phases:

1. **Claude options (phase A).** With the Claude Code provider, the user can authenticate with their own Anthropic API key instead of their Claude Code login, pick any model (an alias or a full id), and set an effort level. Effort is set separately for live notes and for the project brief. The Anthropic API provider gets the same effort settings.
2. **Session instructions (phase B).** Each project has a *default* instruction set and a *current* one. The current set steers the live notes. The user can edit it, clear it, reset it to the default, save it as the default, or fill it from a template. LiveBrainstorm ships five starter templates for different kinds of brainstorming, and users can add, edit, delete and restore templates.

## Decisions (from the brainstorm)

| Topic | Decision | Source |
|---|---|---|
| What instructions control | Steer within the four fixed sections (focus, tone, what counts as a fault). Headings, their order and the format rules never change. | User (option A) |
| Current set on reopen | Whatever it was last time; saved per project, like drafts | User |
| Default for a new project | Empty, so notes behave exactly as today until the user sets one | User |
| Where instructions live in the UI | Top-bar button that opens a right-hand drawer, like the brief drawer | User |
| Storage | Core-owned `instructions.json`; the current text is sent with each `analyze` call, like the draft | User (approach 1) |
| Starter templates | Five (below); the user may rewrite or trim them | User approved |
| Effort granularity | Per tier: "While you type" and "Project brief" each have their own | User |
| API key for Claude Code | The same keychain entry as the Anthropic API provider, passed as `ANTHROPIC_API_KEY` | User approved |
| Effort for the API provider | Yes, as `output_config.effort` | User |
| "Claude‑B" | The Claude Code CLI provider ("Claude subscription" in Settings) | Assumed; not contradicted |

## Non-goals

- Instructions that rename, add or drop sections. The headings are a Rust↔TypeScript contract, lanes split by section, and the fact-check reads Faults.
- Instructions for the project brief, the Jev gate, the rerank or the fact-check.
- Instructions read from the repository (for example `.livebrainstorm/instructions.md`). An untrusted clone would write into the system prompt.
- A global default for new projects.
- Separate keys for the Claude Code and API providers.
- Any "Sign in with Claude" flow or reading Claude Code's credentials. The key is an Anthropic Console API key that the user pastes in.

## Success criteria

- With blank instructions and default Claude options, the analysis prompt and the `claude` arguments are byte-identical to today. Covered by tests.
- A `settings.json` from before this change loads with the new defaults. Covered by a test.
- Changing the instructions refreshes the notes after one typing pause, without the Jev gate.
- Instructions and templates survive a restart. A corrupt `instructions.json` is kept as `instructions.json.bad`, never silently overwritten.
- With API-key auth, the `claude` child gets `ANTHROPIC_API_KEY` in its environment. The key never appears in argv, settings.json, IPC or logs.
- `--effort` is passed only when a level is set. The API provider sends `output_config.effort` only when a level is set.

---

## Part A: Claude options

### Pre-implementation check

Before writing code, confirm with the installed CLI (2.1.296):
1. `ANTHROPIC_API_KEY=sk-ant-invalid claude -p --restricted --strict-mcp-config --model haiku --max-turns 1 --tools "" <<< "hi"` fails with an authentication error. That proves print mode uses the key over the subscription login, even under `--restricted`.
2. `--effort low` is accepted with `--model haiku` and with `--model sonnet`.

If check 1 fails, stop and revisit the design. Workarounds like `--settings '{"env":…}'` would put the key in argv.

**Result (2026-10-14, CLI 2.1.296):**
- Check 1 passed. The init event reported `"apiKeySource":"ANTHROPIC_API_KEY"`. The run ended with `is_error: true`, `api_error_status: 401` and the result `Invalid API key · Fix external API key`.
- The error text arrived only in an `assistant` message and the `result` event, with no `stream_event` text deltas. `ClaudeCli::stream` therefore returns it as an error rather than as notes.
- `--effort low` passed argument parsing.
- Check 2 (whether `haiku` and `sonnet` accept effort on a real call) still needs one call on the user's subscription.

### Settings

New types in `settings.rs`:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effort { Low, Medium, High, Xhigh, Max }

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeAuth { #[default] Login, ApiKey }
```

New fields, all `#[serde(default)]` so existing files load unchanged:

| Variant | New fields | Default |
|---|---|---|
| `ClaudeCli` | `fast_effort: Option<Effort>`, `deep_effort: Option<Effort>`, `auth: ClaudeAuth` | `None`, `None`, `Login` |
| `AnthropicApi` | `fast_effort: Option<Effort>`, `deep_effort: Option<Effort>` | `None`, `None` |

`None` means "Default": no flag and no request field, so the model's own default applies. `ProviderSettings::label()` reads "Claude Code (API key, haiku)" when `auth` is `ApiKey`; the brief's "Written by …" line uses this label.

### Provider resolution and the CLI

- `provider::resolve` picks the effort for the tier, the same way it picks the model. With `ClaudeAuth::ApiKey`, it reads `secrets::api_key()`. If there's no key, it returns `Precondition("Add your Anthropic API key in Settings, or switch Claude Code back to your login.")`.
- `llm::ClaudeCli` gains `effort: Option<Effort>` and `api_key: Option<Secret>`.
- `llm::AnthropicApi` gains `effort: Option<Effort>`.
- New `llm::Secret(String)` newtype whose `Debug` prints `Secret(***)`. Both `ClaudeCli.api_key` and the existing `AnthropicApi.api_key` use it, so a derived `Debug` never prints a key.
- `claude_cli::args` adds `--effort <level>` when the effort is set, and nothing otherwise.
- `ClaudeCli::command()` sets `ANTHROPIC_API_KEY` on the child when `api_key` is set. The key is never added to argv, where `ps` can see it. In `Login` mode the environment is left as it is today, so an inherited variable still applies, as the README documents.
- `AnthropicApi::stream` adds `"output_config": {"effort": "<level>"}` to the body when the effort is set.
- `cli_error`: an "unknown option" error now says "Claude Code is too old for LiveBrainstorm. Update it with `claude update`." It no longer blames `--restricted` specifically, because `--effort` can trigger it too.
- `cli_error`: a result containing "Invalid API key" says "Claude Code rejected the Anthropic API key. Check the key in Settings, or switch Claude Code back to your login."

### Settings UI

In `SettingsDialog.tsx`:
- **Shared `ApiKeyField`.** The key input plus the Save/Remove flow is extracted from `ApiFields`, unchanged in behaviour. Both providers use it.
- **`ClaudeCliFields`.** New radio pair "Sign in with: My Claude Code login / My Anthropic API key". With the API key selected, `ApiKeyField` appears with the note "Usage is billed to your API account, not your Claude subscription."
- **`ModelPicker`** replaces `ModelSelect` for Claude Code. Options are `haiku`, `sonnet`, `opus`, `fable` and *Custom…*. Custom shows a text input for a full id such as `claude-opus-5-5`. A saved value that isn't an alias opens with Custom already selected. Saving with a blank custom field shows "Enter a model name for While you type" (or Project brief) and doesn't save.
- **`EffortSelect`** sits next to each tier's model in both `ClaudeCliFields` and `ApiFields`. Options are Default, Low, Medium, High, Extra high and Max.
- **`DEFAULTS`** gains `fastEffort: null, deepEffort: null` for both Claude providers, and `auth: "login"` for `claudeCli`. It stays in sync with `Settings::default()`.
- **`providerLabel`** in `App.tsx` shows "Claude Code (API key), haiku" when the key is in use.

`ipc.ts` mirrors the new fields:
- `type Effort = "low" | "medium" | "high" | "xhigh" | "max"`
- `fastEffort: Effort | null`, `deepEffort: Effort | null`
- `auth: "login" | "apiKey"`

---

## Part B: Session instructions

### Prompt

- `prompt::analysis_system(sections, instructions)` gains an `instructions: &str` argument.
- When `instructions.trim()` is non-empty, the block below goes between the section guide (including the "written separately" line for lanes) and `ANALYSIS_RULES`:

  ```
  <session_instructions>
  {instructions}
  </session_instructions>
  These instructions come from the person drafting. Follow them, but the headings above, their order, and the rules below always win.
  ```

- Blank instructions add nothing, so `analysis_system(&SECTIONS, "")` stays byte-identical to today's prompt (`all_four_sections_is_todays_prompt`).
- `analysis_request(…, sections, instructions)` clips the instructions to `min(MAX_INSTRUCTION_CHARS, budget / 10)` on a char boundary, where `MAX_INSTRUCTION_CHARS = 4_000`. That leaves about 1,200 characters for the smallest local model (8,192 tokens). The clipped text goes to every lane.
- The brief, explorer, merge and Jev prompts don't change.

### Core module: `instructions`

New `crates/livebrainstorm-core/src/instructions.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Template { pub id: String, pub name: String, pub text: String }

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectInstructions { pub default: String, pub current: String, pub template_id: Option<String> }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct InstructionsFile { pub templates: Vec<Template>, pub projects: BTreeMap<String, ProjectInstructions> }
// Default: templates = starters(), projects = empty.

pub struct InstructionStore { path: PathBuf, lock: std::sync::Mutex<()> }
```

`InstructionStore` operations, each a synchronous load-modify-write under `lock`, written atomically (tmp file + rename, as `Settings::save` does):

| Operation | Behaviour |
|---|---|
| `load()` | Missing file: `InstructionsFile::default()`. Unreadable or invalid JSON: rename to `instructions.json.bad` (replacing an older `.bad`), then return the default. A present `"templates": []` stays empty. |
| `project(root)` | That root's entry, or `ProjectInstructions::default()` |
| `save_project(root, p)` | Stores `p`, or removes the entry when `p` equals the default, so the file only holds projects with something set |
| `save_templates(t)` | Replaces the template list |
| `restore_starters()` | Appends each starter whose id is missing, in starter order, and returns the merged list. Never overwrites an edited starter. |

- **Keys.** Projects are keyed by the root path string (`ProjectSummary.root`). Sessions with no project use the key `""`, matching `livebrainstorm.draft.none`. Moving a project folder loses its instructions, as it already loses its draft.
- **Location.** The file lives at `<app_config>/instructions.json`, next to `settings.json`.
- **Ids.** New templates get ids from `crypto.randomUUID()` in the webview. Starters use the fixed ids below.

### Starter templates

Each one works within the four sections and the format rules. The texts are a first draft for the user to edit.

**`feature-design`: Feature design**
```
I'm designing a new feature.
- Faults: look for features or code that already do part of this, integration points I haven't mentioned, and data or state the feature needs that the project doesn't have yet.
- Directions: favour the smallest version that would prove the idea works.
- Questions: ask about scope, who the feature is for, and what success looks like.
```

**`bug-hunt`: Bug hunt**
```
I'm investigating a bug. Treat my draft as a hypothesis, not a fact.
- Faults: point to code paths that explain or contradict the symptoms I describe, especially error handling, concurrency, caching, and state that outlives a request.
- Directions: suggest the cheapest experiment, log line, or test that would confirm or rule out each cause.
- Questions: ask for the evidence that would narrow it down fastest.
```

**`refactor`: Refactor or architecture**
```
I'm planning a refactor or an architectural change.
- Faults: name the callers, contracts, and invariants the change would break, including anything the project's docs say to preserve.
- Directions: propose an order of steps that keeps the code working after each one.
- Questions: ask what must not change, and how we'd know nothing regressed.
```

**`product-ux`: Product and UX**
```
I'm thinking about the user's experience, not the implementation.
- Faults: point out confusing states, missing feedback, empty and error states, and accessibility gaps in what I describe.
- Directions: suggest changes to what the user sees and does. Mention code only to say what exists today.
- Questions: ask who the user is and what they're trying to get done.
```

**`devils-advocate`: Devil's advocate**
```
Be a skeptical reviewer.
- Faults: give the strongest objection to the plan rather than a list of small issues.
- Directions: include one alternative that rejects the premise of my draft.
- Questions: ask what evidence would change my mind.
```

### IPC

All of these follow the usual steps: `#[tauri::command]` in `commands.rs`, registered in `generate_handler!`, and wrapped in `api` in `ipc.ts`. The first three are synchronous commands, so the store's `std` mutex is never held across an `.await`.

| Command | Signature | Notes |
|---|---|---|
| `get_instructions` | `(root: String) -> InstructionsView { templates, project }` | |
| `save_project_instructions` | `(root: String, instructions: ProjectInstructions) -> Result<()>` | |
| `save_templates` | `(templates: Vec<Template>) -> Result<()>` | |
| `restore_starter_templates` | `() -> Result<Vec<Template>>` | |
| `analyze` (changed) | gains `instructions: String` | Passed through to `analysis_request` for every lane |

`AppState` gains `instructions: InstructionStore`, built from `config_dir`.

### Frontend

**Label.** A pure function `instructionsLabel(project, templates)` in `src/lib/instructions.ts` returns:

| State | Label |
|---|---|
| `current` is blank | "No instructions" |
| `templateId` is set and its template's text equals `current` | "Bug hunt" |
| `templateId` is set and the text differs, or the template was deleted | "Bug hunt (edited)", or "Custom" if the template is gone |
| No `templateId` and `current` equals `default` | "Project default" |
| Anything else | "Custom" |

Covered by vitest.

**Top bar.** A ghost button "Instructions: {label}" sits after the brief button. It's visible with or without a project and toggles the drawer.

**`InstructionsDrawer.tsx`** uses the same shell as `BriefDrawer`: right-hand, over the notes, `role="dialog"`. Only one drawer is open at a time, and Esc closes it (handled in `App.tsx`). Contents, top to bottom:
1. **Template `<select>` and *Apply*.** Apply sets `current` to the template's text and `templateId` to its id. An *Undo* link then restores the previous `current` and `templateId`, and disappears on the next edit.
2. **Textarea** (`font-sans`, `inputClass`) bound to `current`, with a counter such as "1,234 / 4,000". Above 4,000 the counter turns `text-fault` and says the excess is ignored. With the Local provider, a muted line adds "Local models read fewer characters, depending on the model", because their cap is `budget / 10`.
3. ***Clear*, *Reset to default* and *Save as default*.** Reset is disabled when `current === default`, and Save as default when they're already equal. A muted line shows the first line of `default`, or "No default for this project".
4. **Templates list.** Each row shows a name with *Edit* and *Delete*.
   - *Edit* expands an inline name field and textarea with Save and Cancel.
   - *Delete* swaps the row for "Delete {name}? Yes / No".
   - *+ New template* opens the same inline editor, pre-filled with `current`.
   - A *Restore starter templates* link sits at the bottom.
   - No `confirm()` or other browser dialogs.

It uses the `ui.tsx` primitives and semantic colour tokens only.

**State in `App.tsx`:**
- `instructions: ProjectInstructions` and `templates: Template[]` live alongside the draft.
- `loadProject` awaits `openProject(path)`, then `getInstructions(summary.root)`, before setting state, so the project and its instructions land in the same render. Startup without a project calls `getInstructions("")`.
- Edits update state immediately. `saveProjectInstructions` runs 500 ms after the last edit, and a pending save is flushed before switching projects and on unmount. Template changes save immediately.
- A save that fails shows an inline error in the drawer ("Couldn't save instructions: …"). The text stays in state.

**`useLiveAnalysis`** takes `instructions: string`:
- It keeps the value in a ref read at call time, as it does for `contextKey`, and passes it to `api.analyze`.
- `App.tsx` adds the instruction text to `contextKey`. Changing the instructions therefore resets the gate and re-reads the same draft after the usual pause, with no gate, the same way switching projects or briefs does.

---

## Testing

| Where | Test |
|---|---|
| `prompt.rs` | Blank or whitespace-only instructions give a byte-identical prompt. The block sits before the rules and after the "written separately" line. Every lane's prompt carries it. Clipping respects `min(4_000, budget / 10)` and char boundaries. |
| `instructions.rs` | Missing file returns the starters. A corrupt file is renamed to `.bad` and the starters are returned. `"templates": []` stays empty. `restore_starters` adds missing starters without overwriting edited ones. `save_project` round-trips and drops entries that equal the default. Concurrent saves don't lose updates (two threads, one store). |
| `settings.rs` | A pre-change `claudeCli` / `anthropicApi` JSON loads with `None` efforts and `Login`. The new fields serialize as `fastEffort`, `deepEffort` and `auth` with the expected values. |
| `claude_cli.rs` | `--effort` appears only when set, with the right value. The key never appears in args. The old-CLI message no longer names `--restricted`. |
| `anthropic_api.rs` | The body builder includes `output_config.effort` only when set. This needs the body construction extracted into a testable `fn body(…) -> Value`. |
| `llm` | `format!("{:?}", Secret(..))` doesn't contain the key. |
| `src/lib/instructions.test.ts` | Every row of the label table. |
| `src/lib/…` | The context key changes when the instructions change. |

Manual checks: the pre-implementation CLI check. In the running app (`npm run tauri dev`, which the user runs, because the agent shell's Node is too old for Vite 7): apply a template, see the notes refresh, restart, and confirm the instructions persist.

## Implementation order

**Phase A: Claude options**
1. Pre-implementation CLI check.
2. Core: `Effort`, `ClaudeAuth`, the new settings fields and their tests.
3. Core: `Secret`, the `ClaudeCli` args and environment, the `AnthropicApi` body, the `cli_error` wording, and their tests.
4. Shell: `provider::resolve` passes effort and key.
5. UI: `ipc.ts` types, `ApiKeyField` extraction, auth radio, `ModelPicker`, `EffortSelect`, `DEFAULTS`, `providerLabel`.
6. Docs.

**Phase B: Session instructions**
1. Core: the instructions argument in `analysis_system` and `analysis_request`, with tests.
2. Core: the `instructions` module (types, starters, store), with tests.
3. Shell: `AppState.instructions`, the four commands, the `analyze` parameter, registration.
4. Frontend: `ipc.ts`, `instructionsLabel` with tests, `useLiveAnalysis`, `App.tsx` state, loading and saving, the top-bar button, `InstructionsDrawer`.
5. Docs.

Each step leaves `cargo test -p livebrainstorm-core`, `cargo clippy --workspace --all-targets`, `npm run typecheck` and `npm test` passing.

**Branching.** The work goes on a new branch, `feat/instructions-and-claude-options`, cut from `feat/parallel-notes` once that branch's phase work settles. It needs the lane-aware `analysis_system(sections)`. This spec is committed on that branch.

## Risks

- **`ANTHROPIC_API_KEY` precedence.** If print mode under `--restricted` doesn't use it, part A's key option fails. The pre-implementation check finds this before any code is written.
- **Effort on models that don't support it.** A custom older model id can reject `--effort` or `output_config.effort`. The error surfaces as "Claude Code failed: …" or the API's message, and choosing Default fixes it. No model-capability table is maintained.
- **Instructions that fight the format.** For example, "write long paragraphs". The closing sentence says the rules win. A Faults section with fewer backticked names makes the fact-check find less, but nothing breaks.
- **Cost while editing instructions.** Each pause while editing re-runs the analysis, just as typing in the draft does.

## Documentation updates

- **AGENTS.md:**
  - Cross-language contracts:
    - `Template` / `ProjectInstructions` / `InstructionsView` ↔ `ipc.ts`
    - `Effort`, `ClaudeAuth` and the new provider fields ↔ `ipc.ts` and `DEFAULTS`
    - `analysis_system(&SECTIONS, "")` must stay byte-identical
  - Persistence: `instructions.json` and its `.bad` rule.
  - Providers: `ClaudeCli` API-key auth through the environment, never argv.
- **README:**
  - The Claude Code API-key option, as the fix for setups that relied on `apiKeyHelper`.
  - Effort settings.
  - Session instructions and templates.
