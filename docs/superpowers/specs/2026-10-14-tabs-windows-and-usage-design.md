# Tabs, windows and a usage ledger: design

Date: 2026-10-14 · Status: approved in conversation, awaiting written-spec review

## Goal

LiveBrainstorm today shows one project and one draft at a time, and a keystroke anywhere aborts the only analysis. This change lets the user work on several projects and ideas at once:

1. **Tabs.** A sidebar lists projects, and each project has one or more tabs. A tab has its own draft, its own notes and its own analysis. Tabs analyse in parallel, across projects too, and a tab shows that it is analysing.
2. **Sessions.** Each tab keeps its last 5 analyses, which the user can browse. Drafts, notes and history survive restarts. A closed tab is kept and can be reopened.
3. **Windows.** The app can have several windows. Each window is its own workspace with its own tabs.
4. **Usage.** No global cap is placed on parallel runs. Instead, the app shows tokens and estimated cost per day, so the user can see what parallel tabs cost.

## Decisions (from the brainstorm)

| Topic | Decision | Source |
|---|---|---|
| Brief for tabs on one project | One cached brief per project, shared by its tabs in every window | User |
| Drafts across restarts | Persist | User |
| Concurrency cap | None; show daily tokens and estimated cost instead | User |
| What a session holds | Its own draft and notes. The model never sees other tabs or earlier analyses | User |
| "Previous 5 analyses" | A history the user can browse; each run stays stateless | User (option 1) |
| Closed tabs | Persist, reopen at any later launch, kept until deleted | User |
| Model per tab | No: one independent analyser per tab, using the global model setting | User (option 1) |
| Windows | Each window is its own workspace (its sidebar lists only its tabs); moving a tab is explicit; *Recently closed* is shared | User (option 2) |
| Usage view | Today in the sidebar footer, plus a 30-day panel broken down by project and kind | User (option 1) |
| Storage | Core-owned JSON files (`SessionStore`), like `instructions.json` | User (approach A) |
| Cancelled runs in the ledger | Estimated from characters and marked as estimates | User approved |
| Brief or instructions change with many tabs open | Only the visible tab re-runs; background tabs re-run when shown | User approved |
| Closing a window | Its tabs go to *Recently closed* if other windows remain; the last window (or ⌘Q) keeps everything | User approved |
| Closing a window's last tab | The window stays open and empty | User approved |

## Non-goals

- A model or provider per tab.
- Sending earlier analyses or other tabs to the model. Live analysis stays stateless (AGENTS.md).
- Dragging tabs between windows, or showing one tab in two windows.
- Spend limits, budgets or blocking runs.
- Counting Jev/TypeSafe usage. It isn't Claude, and its price isn't published.
- Syncing sessions across machines, or searching across tabs.

## Success criteria

- Two tabs on the same project, or on different projects, analyse at the same time. Typing in one never aborts the other.
- Quitting and relaunching reopens every window with its tabs, drafts, notes and history, and starts no model call. Verified with the usage ledger: no new runs at launch.
- A tab's history holds at most 5 analyses, newest first. Skips and errors never add one.
- A brief built from one tab shows as current in every tab of that project, in every window. One build per project at a time.
- A single tab's analysis prompt and `claude` arguments are byte-identical to today's. Covered by the existing prompt tests.
- Upgrading from today's build loses nothing: every saved draft becomes a tab.
- For a completed Claude Code run, the ledger's cost equals the CLI's `total_cost_usd`. Covered by a fixture test.
- Tab and window ids that arrive over IPC are validated before they're used in a file path.

---

## 1. Data model and persistence

### Files

Everything lives under `<app_data>/sessions/` and belongs to a new core module, `livebrainstorm-core::sessions`:

```
sessions/
  workspace.json        windows, their tab order, active tab
  tabs/<tabId>.json     one tab: draft, title, history
```

`workspace.json`:

```json
{
  "version": 1,
  "nextWindow": 3,
  "windows": [
    { "id": "w-1", "tabs": ["t-…", "t-…"], "active": "t-…" },
    { "id": "w-2", "tabs": [], "active": null }
  ]
}
```

`tabs/<tabId>.json`:

```json
{
  "id": "t-…",
  "root": "/Users/me/code/shop-api",
  "title": null,
  "createdAt": 1760400000,
  "updatedAt": 1760403600,
  "closedAt": null,
  "draft": "…",
  "nextSnapshot": 7,
  "history": [ /* Snapshot, newest first, at most HISTORY_LEN = 5 */ ]
}
```

- `root` is optional. A tab with no project works like today's no-project draft and sits under "No project" in the sidebar.
- `title: null` means the sidebar shows the draft's first non-empty line, trimmed to 48 characters, or "Untitled". Renaming sets `title`.
- **A closed tab is any tab file that no window lists.** There is no separate closed list that could drift. If `workspace.json` is lost, every tab shows up under *Recently closed* instead of vanishing. `closedAt` orders that list. A tab file that no window lists and that has no `closedAt` (the crash case) sorts by `updatedAt`.

### Snapshot

```json
{
  "id": 6,
  "at": 1760403600,
  "draft": "the draft this analysis read",
  "context": "root|briefCreatedAt|instructions",
  "markdown": "## Intent …",
  "partial": false,
  "notices": [],
  "checks": [ /* CheckResult, as in jev/check.rs */ ],
  "checksTotal": 4,
  "checksStopped": false
}
```

- `context` is the frontend's `analysisContextKey`, passed through `analyze` as an opaque string. It lets the gate keep working after a restart.
- `checksTotal` comes from the last `checking` event. On restore, `checks.len() < checksTotal` with `checksStopped: false` means the run was cut off. The UI then shows "Checks stopped when you edited.", as it does live.

### Ids

- `SessionStore` generates tab ids as `t-<unix millis, hex>-<per-process counter, hex>`. Window ids are `w-<nextWindow>`.
- **Every id received over IPC must match `^[tw]-[0-9a-f-]{1,40}$`** before it is used in a path. Anything else is `Error::Precondition("Unknown tab.")`. A tab id becomes a file name, so this check is what stops path traversal.

### Store behaviour

- **Writes:** atomic (tmp file + rename). A file that exists but can't be parsed is renamed to `.bad` (then `.bad.1`, …) before anything is written, using the same rules as `InstructionStore`. A read error that isn't a parse error (a permissions problem, a directory in the way) is reported and never renamed.
- **Concurrency:** one `std::sync::Mutex` guards the workspace and a cache of loaded tabs. It is held only for synchronous file I/O, never across `.await`.
- **Draft saves:** the frontend uses `createSaver` keyed by tab id (500 ms after the last edit). It flushes on tab switch, before moving a tab, and on `pagehide`.

### Operations (`SessionStore`)

| Operation | Effect |
|---|---|
| `workspace()` | Windows and their tabs. On first run, creates `w-1` and reports `fresh: true` |
| `create_tab(window, root)` | New tab at the end of the window's list, made active |
| `save_draft(tab, draft)` | Updates `draft` and `updatedAt` |
| `rename_tab(tab, title)` | `None` returns the tab to an automatic title |
| `record_snapshot(tab, Snapshot) -> id` | Inserts at the front, trims to 5, returns the snapshot id |
| `update_snapshot(tab, id, f)` | Adds checks, a notice or `checksStopped` to one snapshot; a no-op if it has been trimmed |
| `activate(window, tab)` | Sets `active` |
| `close_tab(tab)` | Removes the tab from its window and sets `closedAt` |
| `close_window(window)` | Closes all its tabs (`closedAt` set) and removes the window |
| `reopen_tab(tab, window)` | Clears `closedAt` and appends the tab to `window`. Fails with "That tab is already open in another window." if a window lists it |
| `delete_tab(tab)` | Only for closed tabs: deletes the file |
| `move_tab(tab, to)` | `to: None` creates a new window |
| `import_legacy(drafts, active_root)` | Once, when `fresh`. Every non-empty draft becomes a tab in `w-1`; `active_root`'s tab is active |
| `closed()` | Closed tabs, newest first, with `root`, title and `closedAt` |

### Upgrade from today's build

On its first launch, the frontend finds `fresh: true`. It reads every `livebrainstorm.draft.<root>` key and `livebrainstorm.lastProject` from `localStorage`; the root `none` becomes `null`. It calls `import_legacy_drafts`, and removes those keys only after the call succeeds. If there are no drafts, the window starts empty.

---

## 2. Backend runtime

### `AppState` (`src-tauri/src/state.rs`)

| Today | Becomes |
|---|---|
| `project: RwLock<Option<Arc<ProjectIndex>>>` and `brief: RwLock<Option<StoredBrief>>` | `projects: RwLock<HashMap<String, Arc<ProjectEntry>>>`, keyed by root, where `ProjectEntry { index: Arc<ProjectIndex>, brief: Option<StoredBrief> }` |
| `analysis: Mutex<Option<JoinHandle<()>>>` | `analyses: Mutex<HashMap<String, JoinHandle<()>>>`, keyed by tab id |
| `brief_task: Mutex<Option<JoinHandle<()>>>` | `brief_tasks: Mutex<HashMap<String, JoinHandle<()>>>`, keyed by root |
| none | `sessions: SessionStore`, `usage: Arc<UsageLedger>` |

`replace_task` takes the slot map and a key. A new run for tab A aborts only tab A's previous run.

**Project memory.** A `ProjectIndex` keeps file contents, so the registry holds one index per project that has an *open* tab in any window. After `close_tab`, `close_window`, `move_tab` and `delete_tab`, the shell drops any registry entry that no open tab references.

### Commands

| Command | Behaviour |
|---|---|
| `open_project(path) -> ProjectSummary` | Scans and inserts or replaces the registry entry. Used by *Open a project…* and for explicit refreshes. Other projects stay loaded |
| `project_summary(root) -> ProjectSummary` | The registered summary. Scans only if the root isn't loaded yet. Windows call it when they show a tab |
| `analyze(tabId, draft, previousDraft, instructions, context, onEvent)` | See below |
| `cancel_analysis(tabId)` | Aborts that tab's run. `close_tab` and `move_tab` call it too |
| `build_brief(root, onEvent)` | See below |
| `list_workspace(window) -> WorkspaceView` | This window's tabs (summaries plus the full active tab), the shared closed list, and `fresh` |
| `get_tab(tabId) -> Tab` | The full tab, used when switching to it |
| `create_tab`, `save_draft`, `rename_tab`, `activate_tab`, `close_tab`, `reopen_tab`, `delete_tab`, `move_tab`, `import_legacy_drafts` | Thin wrappers over `SessionStore` that emit `workspace-changed` |
| `open_window() -> String` | Creates a window (Section 3) |
| `usage_report(sinceHour) -> Vec<UsageBucket>` | Section 5 |

### `analyze`

- The pipeline is today's, unchanged: provider at `Tier::Fast`, pre-phase (gate and rerank), lanes, merge, fact-check.
- **Inputs:** the task looks up the tab's `root` in `SessionStore` and takes the index and brief from the registry entry for that root. A tab with no root runs without code context, as today.
- **Precondition:** an unknown or closed tab fails with "This tab was closed."
- **Recording:** checks that arrive before `done` collect in a task-local buffer. On `done`, the task calls `record_snapshot` with the text, `partial`, the notices so far, the buffered checks and `checksTotal`. Any later check, notice or `checksStopped` goes to `update_snapshot` with that id.
  - Skips and errors record nothing.
  - If recording fails, the notes still stream and the user gets one `notice`: "Couldn't save these notes: …".

### `build_brief`

- **One build per root.** If `brief_tasks` already holds a live task for that root, the call returns `Ok(())` without starting another. The caller's drawer shows "Building…" from the broadcast status.
- **Streaming:** the initiating window still gets the live text and activity over its Channel, as today.
- **Every window** receives the Tauri event `brief-status { root, state: "building" | "done" | "error", message? }`.
- **On `done`:** the task rescans the project (`spawn_blocking`, as `open_project` does), replaces the registry entry, then emits `brief-status: done`. Each window then calls `project_summary(root)`.

### Cross-window events (emitted with `app.emit`)

| Event | When | Windows react by |
|---|---|---|
| `brief-status { root, state }` | A brief build starts, finishes or fails | Updating the brief dot. On `done`, refreshing that project's summary |
| `workspace-changed { windows: [id] }` | Any session mutation | Refetching `list_workspace` if their id is listed. Every window refetches the closed list |
| `settings-changed` | After `save_settings` | Calling `get_settings` |
| `instructions-changed { root }` | After saving instructions or templates | Refetching that root's instructions if it shows a tab of that project |
| `usage-changed` | After a ledger record (at most once a second) | Refreshing the footer |

### Concurrency

- There is no global cap. N tabs analysing means up to N `claude -p` processes (× lanes when parallel notes is on).
- With the Local provider, all tabs share one `llama-server` and queue on it. `ensure_local` is unchanged, and aborting one tab's run never stops the server.

---

## 3. Windows

### Creating and restoring

- `tauri.conf.json` gets `"windows": []`. In `setup`, the shell creates one window per `workspace.json` entry, or `w-1` on the first run, with `WebviewWindowBuilder::new(app, id, WebviewUrl::App("index.html".into()))`. Each window has the title "LiveBrainstorm", a size of 1280×820 and a minimum of 860×560.
- Position and size come from `tauri-plugin-window-state`, which keys on window labels. The labels are the stable workspace ids.
- A window finds its workspace id with `getCurrentWindow().label`.
- `open_window()` allocates `w-<nextWindow>`, saves it, and builds the window. The new window is empty and shows "Open a project".

### Capabilities (`src-tauri/capabilities/default.json`)

- `"windows": ["main"]` becomes `"windows": ["w-*"]`. Without that, runtime windows get no IPC at all.
- Two permissions are added: `core:window:allow-set-title` (the title follows the active tab) and `core:window:allow-close` (⌘⇧W).
- Window creation stays in Rust (`open_window`), so the webview never gets `create-webview-window`.

### Closing

- **A window's close request** is handled in `on_window_event(CloseRequested)`:
  - If other windows are open, the shell calls `close_window(id)`. The window's tabs go to *Recently closed* and their runs are cancelled.
  - If it's the last window, the workspace is left untouched, which is the same as quitting.
- **⌘Q** exits without touching the workspace.
- **Closing a window's last tab** leaves the window open and empty.

### App menu

Tauri's default menu binds ⌘W to *Close Window*, so the app menu is replaced:

- **App menu:** About, Settings… (⌘,), Quit (⌘Q).
- **Edit menu:** keeps the predefined Undo, Redo, Cut, Copy, Paste and Select All. Without them, the draft `<textarea>` loses those shortcuts on macOS.
- **File:**

  | Item | Shortcut |
  |---|---|
  | New Tab | ⌘T, on the active tab's project |
  | New Window | ⌘N |
  | Open Project… | ⌘O |
  | Reopen Closed Tab | ⌘⇧T |
  | Close Tab | ⌘W |
  | Close Window | ⌘⇧W |

- **Window:** Next Tab (⌃Tab), Previous Tab (⌃⇧Tab), Toggle Sidebar (⌘\\).
- **Dispatch:** each menu event is emitted to the focused window as `menu { id }`, and the frontend dispatches it. On Windows and Linux the same items use Ctrl.

### Moving a tab

- The tab row's context menu offers *Move to new window* and *Move to ▸ \<window\>*, where windows are named by their active tab's title.
- The frontend flushes the tab's pending draft save, then calls `move_tab`, which cancels any running analysis. That run's Channel belongs to the old window.
- The receiving window shows the saved notes and analyses again at the next pause.

### Shared settings

Settings, instructions and briefs are shared by all windows. The last save wins, and the events in Section 2 refresh the other windows.

---

## 4. Frontend

### Layout

```
┌ sidebar ───────────────┬──────── draft ─────────┬──── notes ──────────┐
│ margin             ⋯ + │                        │ ‹ 2 of 5 › 14:02    │
│ ┃ Sidebar tabs         │                        │ Notes on an earlier │
│ ┇ Analysing…           │                        │ draft · Show it     │
│ │ Usage ledger         │                        │ INTENT …            │
│ shop-api           ⋯ + │                        │ FAULTS …            │
│ ╎ Checkout retry       │                        │                     │
│ Open a project…        │                        │                     │
│ ▸ Recently closed (4)  │                        │                     │
│ Today 182k tok · ~$0.41│                        │                     │
└────────────────────────┴────────────────────────┴─────────────────────┘
```

- `main` becomes a three-column grid: `[sidebar 248px | draft 3fr | notes minmax(320px, 2fr)]`. The draft and notes panes keep their own `min-h-0` and `overflow-y-auto`.
- The sidebar collapses with ⌘\\. It starts collapsed when the window is narrower than 1,100 px. The collapsed state is per window, in `localStorage` (`livebrainstorm.sidebar.<windowId>`), and read and written inside `try`/`catch`.

### Sidebar (`src/components/Sidebar.tsx`)

- **Groups:** tabs are grouped by project, in order of first appearance in the window's tab list.
  - A group header shows the project name, a `⋯` menu (Project brief…, Rebuild brief) and `+` for a new tab on that project.
  - Tabs without a project sit under "No project".
  - **Order:** a new tab joins the end of its project's group. ⌃Tab / ⌃⇧Tab follow this visual order, not the stored list order.
- **Tab row:** the title and a status line.
  - Double-click to rename.
  - The context menu has Rename, Move to new window, Move to ▸, and Close.
  - The active tab uses `bg-subtle`.
- **Other entries:** *Open a project…* creates a new tab for the chosen folder. *Recently closed (n)* is collapsible and grouped by project. Each closed tab offers Reopen and Delete; Delete asks inline ("Delete for good?"), never with `confirm()`.
- **Footer:** the usage line (Section 5) and the provider label, which opens Settings.

### The analysing indicator

The indicator reuses the visual language of the rule between the panes.

- **Rule:** each tab row has a 2 px × 28 px rule at its left, using the same state classes as `Rule` in `NotesPane.tsx`:
  - `animate-fill` while thinking
  - solid when the notes are current
  - dashed when the draft has moved on
  - faint when idle

  Move the `RULE` map into `ui.tsx` and export it, so both rules share it.
- **Status line:** **"Analysing…"** while thinking, "Draft moved on", "Notes current", or the error in `text-fault`.
- **Collapsed group:** if any tab in a collapsed project group is thinking, its header shows the pulsing dot (`animate-pulse-dot`).
- **Reduced motion:** under `prefers-reduced-motion`, the rule is static (`motion-reduce:animate-none`, `opacity-50`) and the text carries the state.

### Per-tab state

`useLiveAnalysis` is replaced by a controller per tab that lives outside React. A timer inside a hook would die when you switch tabs.

**`src/lib/tabSession.ts`: `class TabSession`**
- **State:** `draft`, `AnalysisState` (unchanged transitions from `analysisState.ts`), `lastSent`, `lastAnalyzed`, `context`, `active`, `contextDirty`, `viewing` (history index, 0 = latest).
- **`setDraft(text)`** stores the draft, schedules the save, and restarts the debounce timer. The pause fires whether or not the tab is active.
- **`setContext(key)`:**
  - If the key changes on the active tab, reset `lastSent`/`lastAnalyzed` and re-run, as today.
  - On a background tab, only set `contextDirty`.
- **`activate()`** runs the dirty re-run (if the draft is ≥ 24 normalized characters). **`deactivate()`** leaves any timer or run alone.
- **`restore(tab)`:**
  - State starts from `history[0]` (status `done`), and `context = history[0].context`.
  - `lastSent = normalize(history[0].draft)` when it equals the normalized current draft, so restoring starts no run.
  - `lastAnalyzed = { draft, context }` from `history[0]` unless it is partial.
  - **Wait for both the brief summary and the instructions:** the window calls `setContext` for a tab only after it has loaded that root's brief summary *and* its instructions. A key computed before then (`root|0|`) would differ from the snapshot's and trigger a spurious run at launch.
- **`rerun()`, `dispose()`:** `dispose` clears timers. The backend run is cancelled by `close_tab` / `move_tab`.
- **`subscribe(fn)`:** for `useSyncExternalStore`.
- It calls `api.analyze(tabId, draft, previousDraft, instructions, context, onEvent)` and drops events from superseded requests by request id, as today.

**`src/lib/tabs.ts`:** a per-window module store (`useSyncExternalStore`, like `downloads.ts`).
- It holds the workspace view, the active tab id, and `Map<tabId, TabSession>`. It creates controllers lazily when a tab is first shown or starts a run.
- It exposes `useTabSession(tabId)` and `useWorkspace()`.

**`src/lib/tabTitle.ts`:** `tabTitle(tab)` returns the explicit title, else the first line of the draft, else "Untitled". The window title is `LiveBrainstorm — ${tabTitle(active)}`.

**Context per tab:** `analysisContextKey(root, brief?.createdAt, instructions.current)`.
- Instructions are cached per root in the window: fetched on first use, refreshed on `instructions-changed`.
- Brief summaries are cached per root: from `project_summary`, refreshed on `brief-status: done`.

### History browsing (`NotesPane`)

- **Header:** `‹ 2 of 5 ›` plus the snapshot's time. The arrows are disabled at the ends and hidden when the history holds fewer than two analyses.
- **An older snapshot** shows its notes, checks and notices read-only, under a banner in `text-muted`: "Notes on an earlier draft." A **Show that draft** link expands the snapshot's draft in `font-serif text-muted`, collapsed by default.
- **On `started`:** the view goes back to the latest (`viewing = 0`). Live streaming always renders the latest.

### Top bar

The top bar shows the active tab's project name, the brief status, the Instructions button and the provider label. *Open a project* moves to the sidebar. With no active tab, the top bar shows only the app name.

---

## 5. Usage ledger

### What is counted

- Counted: every model call, meaning each notes lane, each brief explorer and the brief merge.
- Not counted: Jev calls.
- Kinds: `notes` and `brief`.

### Where the numbers come from

| Provider | Tokens | Cost |
|---|---|---|
| Claude Code | `result.modelUsage[model]`: `inputTokens`, `cacheCreationInputTokens`, `cacheReadInputTokens`, `outputTokens`, one record per model | `modelUsage[model].costUSD`. Its sum equals `total_cost_usd`. If `modelUsage` is missing, use the `usage` totals with `total_cost_usd`, under the requested model |
| Anthropic API | `message_start.message.usage` (input and cache fields) + the last `message_delta.usage.output_tokens` | `usage::PRICES` × tokens |
| Local (OpenAI-compatible) | The final chunk's `usage` (request `stream_options: { include_usage: true }`), or an estimate if absent | $0 |

**Fixture.** The CLI `result` event recorded on 2026-10-14 is the parser test fixture. That run used `--model haiku` and resolved to `claude-haiku-5-5`, with `costUSD` 0.00046765 and `costBasis: "list"`.

### Price table (`usage::PRICES`)

List prices in $ per million tokens. Source: Claude API reference, model table cached 2026-10-06.

| Model id | Input | Output |
|---|---|---|
| `claude-fable-5-1`, `claude-fable-5` | 10.00 | 50.00 |
| `claude-opus-5-5` | 4.00 | 20.00 |
| `claude-opus-5`, `claude-opus-4-8`, `claude-opus-4-7`, `claude-opus-4-6` | 5.00 | 25.00 |
| `claude-sonnet-5-5`, `claude-sonnet-5` | 2.00 | 10.00 |
| `claude-sonnet-4-6` | 3.00 | 15.00 |
| `claude-haiku-5-5` | 0.10 | 0.50 |
| `claude-haiku-4-5` | 1.00 | 5.00 |

- **Lookup order:** exact id first. Otherwise the CLI aliases `fable`, `opus`, `sonnet` and `haiku` map to the newest id of that family (`claude-fable-5-1`, `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`). Otherwise the cost is unknown and shown as "—", with the tokens still counted.
- **Haiku 5.5 above 100K prompt tokens:** the price rises to $0.50 / $2.50. Analysis prompts are capped at 120k characters (≈ 30k tokens), so the table uses the lower rate.
- **Cache tokens:** the API provider sets no `cache_control`, so its responses have none today. If any appear, they are priced at the input rate, which errs high.
- **Updating:** prices change rarely, and editing this table is a deliberate one-line change, like `LLAMA_CPP_BUILD`.

### Cancelled runs

- A keystroke aborts a run before its `result` arrives, so those runs would otherwise be invisible.
- `usage::Meter` wraps each call. It counts the characters sent (`system` + `user`) and received (`Chunk::Text`).
- The shell creates the meter immediately before `provider.stream`, after the pre-phase, so a run aborted during the Jev pre-phase records nothing. After `stream` returns, the shell calls `meter.finish(result.is_ok())`.

| How the call ended | What is recorded |
|---|---|
| A `Chunk::Usage` arrived | That usage, exactly |
| `stream` returned `Ok` without usage (for example, a local build that doesn't report it) | An estimate |
| `stream` returned `Err` without usage | Nothing. A failed call (bad key, CLI missing) is assumed unbilled |
| Dropped before `stream` returned (a keystroke aborted it) | An estimate, from `Drop` |

- **An estimate:**
  - tokens = `ceil(chars / 4)`
  - all input priced at the input rate, using `Provider::model()` (a new accessor that returns the model the provider was asked for)
  - `estimated: true`
- `Drop` writes through the ledger's `std` mutex. It never awaits.

### Plumbing

- **`llm::Chunk`** gains `Usage(Usage)`. Each provider emits it once at the end of any call for which the provider reports usage, including a failed call whose CLI `result` carries usage. `Provider::stream` keeps its signature.
  ```rust
  pub struct Usage {
      pub model: String,
      pub input: u64,
      pub cache_write: u64,
      pub cache_read: u64,
      pub output: u64,
      pub cost_usd: Option<f64>,
  }
  ```
- **`usage::Meter::new(ledger, UsageTag { root: Option<String>, kind }, model, sent_chars)`** wraps a sink. It forwards `Text` and `Activity`, and records `Usage`.
- **The shell** wraps the sink of every notes lane, explorer and merge call. Sinks that `match` on `Chunk` gain a `Usage` arm.
- **`UsageLedger`** is in core and has no Tauri dependency. It takes an `on_record` callback, which the shell sets to emit `usage-changed` (throttled to once per second).

### Storage (`<app_data>/usage.json`)

```json
{
  "version": 1,
  "buckets": [
    { "hour": 489000, "root": "/Users/me/code/shop-api", "kind": "notes", "model": "claude-haiku-5-5",
      "input": 1200, "cacheWrite": 22000, "cacheRead": 90000, "output": 3100,
      "costUsd": 0.0123, "runs": 14, "estimatedRuns": 3, "estimatedCostUsd": 0.0011, "unpricedRuns": 0 }
  ]
}
```

- **Buckets:** `hour = unix_secs / 3600`, in UTC. The key is `(hour, root, kind, model)`.
- **Grouping by day happens in the UI,** so "today" follows the user's clock without a time-zone crate in Rust.
- **Writes:** in memory, flushed atomically at most every 5 s and on `RunEvent::Exit`. On load, buckets older than 90 days are dropped. A corrupt file is kept as `.bad` and the ledger starts empty.
- **`usage_report(sinceHour)`** returns the buckets from that hour on.

### UI

- **Footer:** "Today · 182k tokens · ~$0.41".
  - Tokens are input + cache write + cache read + output, summed over today's local hours.
  - The cost is prefixed with "~" when any of it comes from an estimate.
- **Usage panel:** clicking the footer opens a native `<dialog>` (`showModal()`, like Settings).
  - One row per local day for the last 30 days: date, tokens, estimated cost, runs. The first row is a 30-day total.
  - A row expands into lines per project and kind ("shop-api · notes").
  - Estimates are labelled, e.g. "incl. ~$0.03 from 12 cancelled runs". Runs with no price say "price unknown for N runs".
  - A note at the bottom: "On a Claude subscription you aren't billed per token. This is what the same usage would cost on the API."
- **Pure helpers** (`src/lib/usage.ts`): `groupByLocalDay(buckets, now)`, `todayTotals(buckets, now)`, `formatTokens(n)` ("182k", "1.2M") and `formatCost(usd, estimated)`.

---

## 6. Errors, testing and phases

### Error handling

Saving never blocks typing. Persistence never fails an analysis.

| Failure | Behaviour |
|---|---|
| Corrupt `workspace.json` | Kept as `.bad`. The app starts with `w-1` empty. Every tab file shows under *Recently closed* |
| Corrupt tab file | Kept as `.bad` and skipped. The sidebar shows "Couldn't read a tab; kept it as \<file\>.bad" once |
| `record_snapshot` fails | The notes still show, plus one `notice` |
| `save_draft` fails | Shown in the sidebar footer in `text-fault` and retried with the next edit (`createSaver`'s `onError`) |
| Project folder missing or unreadable | The tab opens with its draft and notes. The top bar shows the `Precondition` message. Analysis runs without code context until the folder is readable |
| `analyze` / `get_tab` on a closed or unknown tab | `Precondition`: "This tab was closed." |
| `reopen_tab` on a tab open elsewhere | `Precondition`: "That tab is already open in another window." |
| Ledger write fails | One `eprintln!`, then ignored. The ledger never affects an analysis |

### Testing

**Core (`cargo test -p livebrainstorm-core`):**
- **`sessions`:**
  - Create, save the draft, rename to `None` and back.
  - History keeps the newest 5.
  - `update_snapshot` attaches to the right id and is a no-op after trimming.
  - Close, reopen, delete. Moving to `None` creates a window.
  - Any tab file not listed by a window counts as closed; ordering by `closedAt` and `updatedAt`.
  - Reopening a tab that's already open fails.
  - Corrupt workspace and tab files become `.bad`, and earlier `.bad` copies are never replaced.
  - Legacy import (empty drafts skipped, active root), and the id validation regex.
- **`usage`:**
  - CLI `result` parsing from the 2026-10-14 fixture, and a fixture with two models.
  - API SSE usage parsing.
  - Price lookup: exact id, alias, unknown.
  - The `Meter` estimate on drop, and no record before the request is sent.
  - Hourly bucketing and merging, the 90-day prune, corrupt file → `.bad`.
- **Existing prompt tests stay green unchanged.** This is the byte-identical guard.

**Frontend (`npm test`):**
- **`tabSession`,** with fake timers and a fake `api.analyze`:
  - A background tab's pause fires after you switch away.
  - A context change on a background tab waits for `activate()`.
  - `restore` starts no run when the draft matches `history[0]`.
  - `restore` from a partial snapshot leaves `lastAnalyzed` null.
  - Superseded events are dropped.
  - `started` resets `viewing`.
- **Helpers:** `tabTitle`, and the `usage.ts` helpers (local-day grouping across a time-zone offset, formatting).

**Manual checklist (`npm run tauri dev`):**
- Two windows, two projects: type in both, and both tabs animate without aborting each other.
- Brief: build it in window 1, and the tab of the same project in window 2 turns current.
- Quit and relaunch: windows, tabs, history and positions come back, and the usage footer shows no new runs.
- Close a non-last window: its tabs appear under *Recently closed* in the other window. Reopen one.
- Move a tab mid-run: the old run stops, and the new window re-runs at the next pause.
- ⌘C, ⌘V and ⌘Z still work in the draft after the menu change.
- The footer changes after each run. A burst of typing adds estimated runs.

### Pre-implementation checks

Run each check before the phase that needs it, and record the results in this spec, as the Claude-options spec did:

1. **Phase 3:** on macOS, does ⌘Q fire `CloseRequested` for each window before `ExitRequested`? If it does, the close handler must skip `close_window` once `ExitRequested` has fired. Measure the event order with a logging build before relying on that.
2. **Phase 3:** does a capability with `"windows": ["w-*"]` apply to windows created at runtime with those labels?
3. **Phase 4:** does the pinned `llama-server` build (`LLAMA_CPP_BUILD`) return `usage` in its final streamed chunk when asked with `stream_options.include_usage`? If not, local runs use the `Meter` estimate (cost $0 either way).

### Phases

Each phase gets its own plan, as Phases A and B did.

1. **Sessions backend.** Build the `sessions` core module, the `AppState` registry and per-tab and per-root task slots, the new commands, recorded snapshots, `brief-status`, and the legacy import. The current single-pane UI runs on the window's active tab. *Open a project* switches to this window's tab for that project, or creates one if none exists, so behaviour matches today's per-project drafts. The only visible change is that notes survive restarts.
2. **Tabs UI.** Add the sidebar, `TabSession` and `tabs.ts`, the analysing indicator, *Recently closed*, history browsing, and the tab shortcuts: ⌘T, ⌘W, ⌘⇧T and ⌃Tab, handled in the webview for now.
3. **Windows.** Add runtime windows, the window-state plugin, the capability glob and permissions, `move_tab`, the close semantics, the app menu (which takes over the Phase 2 shortcuts), and the `settings-changed` and `instructions-changed` events.
4. **Usage ledger.** Add `Chunk::Usage`, the provider parsing, `Meter`, `UsageLedger`, the footer and the panel.

## Documentation updates

- **AGENTS.md:**
  - Rewrite "Live analysis flow" step 2 for per-tab slots, and the brief paragraph for builds keyed by root plus `brief-status`.
  - Add `sessions` and `usage` to the architecture and persistence sections, and replace the "drafts in `localStorage`" line.
  - Add `tabs/<id>.json` id validation to the invariants.
  - Note `Chunk::Usage` under Providers.
  - Add the new events to the cross-language contracts: `brief-status`, `workspace-changed`, `settings-changed`, `instructions-changed`, `usage-changed`, `menu`.
  - Add `usage::PRICES` to the deliberate one-line changes.
- **Cross-language contracts** to add:
  - `sessions` types ↔ `ipc.ts`
  - `UsageBucket` ↔ `ipc.ts`
  - window ids ↔ the capability glob
- **README:** describe tabs, windows and the usage panel, including the note that subscription users see an API-equivalent estimate.
