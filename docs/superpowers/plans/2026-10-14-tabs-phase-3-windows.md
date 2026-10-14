# Phase 3: Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** LiveBrainstorm can have several windows, each its own workspace with its own tabs. Windows come back at launch where they were, ⌘N opens one, closing follows fixed rules, and a tab can move between windows. An app menu owns the shortcuts, and settings and instructions saved in one window refresh the others.

**Architecture:**
- **Core (`livebrainstorm-core`):**
  - New module `menu`: the app menu's own items (id, label, accelerator) as data. The ids are the frontend's `ActionId`s, and a test reads `src/lib/actions.ts` to keep the two equal.
  - New file `sessions/windows.rs`: three `SessionStore` methods for the shell (`windows_to_open`, `close_window_unless_last`, `windows`), built only on Phase 1's public API.
- **Shell (`src-tauri`):**
  - `windows.rs` builds one window per workspace window, labelled with its id (`w-<n>`), and applies the close rules in `on_window_event`.
  - `menu.rs` replaces Tauri's default menu and sends each custom item to the focused window as `menu { id }`.
  - New commands: `open_window`, `move_tab`, `list_windows`. `save_settings` and the instruction saves emit `settings-changed` and `instructions-changed`.
  - `tauri-plugin-window-state` restores positions and sizes. The capability glob `w-*` gives runtime windows IPC.
- **Frontend:**
  - `useActions` takes `menu` events instead of keydown.
  - `moveTab.ts`, `tabs.move` and the *Move to* items in `TabRow`'s context menu move a tab. A tab that arrives in a window is shown there.
  - `crossWindow.ts` refetches after other windows' saves, through Phase 2's `projects` store.
  - `windowTitle.ts` sets the native title from Phase 2's `windowTitle` text.

**Tech Stack:** Rust 2021, Tauri 2.12.1 (menus through muda 0.20), `tauri-plugin-window-state` 2.x (2.5.0 when this was written; it needs tauri 2.12), React 19 + Tailwind v4, vitest 3 (node environment, no DOM).

**Spec:** `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md`. This plan covers:
- Section 3 ("Windows") in full.
- From Section 2, the `settings-changed` and `instructions-changed` rows of "Cross-window events".
- From Section 6, pre-implementation checks 1 and 2, and the manual-checklist items about windows.
- Phase 3's share of "Documentation updates".

**Prerequisites:** Phases 1 and 2 are complete on branch `feat/tabs-windows-usage` (`docs/superpowers/plans/2026-10-14-tabs-phase-1-sessions-backend.md` and `docs/superpowers/plans/2026-10-14-tabs-phase-2-tabs-ui.md`). All work in this plan happens on that branch, in its worktree. Phase 4 (the usage ledger) builds on this plan.

## What Phases 1 and 2 leave behind

This plan changes code those phases wrote. It names that code by contract name, never by line number. Each task copies the definitions it uses into its own **Interfaces** block.

**Core (Phase 1).** `crates/livebrainstorm-core/src/sessions/` holds `mod.rs` (types, `HISTORY_LEN`, re-exports), `ids.rs` and `store.rs`. Every struct uses `#[serde(rename_all = "camelCase")]`.

```rust
pub struct Tab { pub id: String, pub root: Option<String>, pub title: Option<String>, pub created_at: u64,
                 pub updated_at: u64, pub closed_at: Option<u64>, pub draft: String, pub next_snapshot: u64,
                 pub history: Vec<Snapshot> }
/// What lists show; no draft body or history.
pub struct TabSummary { pub id: String, pub root: Option<String>, pub title: Option<String>,
                        pub first_line: String, pub updated_at: u64, pub closed_at: Option<u64> }
pub struct WindowView { pub window: String, pub tabs: Vec<TabSummary>, pub active: Option<Tab>,
                        pub closed: Vec<TabSummary>, pub fresh: bool, pub unreadable: Vec<String> }
pub fn valid_id(id: &str) -> bool;   // ^[tw]-[0-9a-f-]{1,40}$
impl SessionStore {
    pub fn new(dir: PathBuf) -> Self;   // dir = <app_data>/sessions
    pub fn window_ids(&self) -> Vec<String>;                       // creates w-1 if no workspace
    pub fn window_view(&self, window: &str) -> Result<WindowView>;
    pub fn add_window(&self) -> Result<String>;                     // allocates w-<next_window>
    pub fn create_tab(&self, window: &str, root: Option<String>) -> Result<Tab>;  // appended, made active
    pub fn tab(&self, id: &str) -> Result<Tab>;                     // any tab file, open or closed
    pub fn save_draft(&self, id: &str, draft: String) -> Result<bool>;  // true when the first line changed
    pub fn rename_tab(&self, id: &str, title: Option<String>) -> Result<()>;
    pub fn activate(&self, window: &str, id: &str) -> Result<()>;
    pub fn close_window(&self, window: &str) -> Result<Vec<String>>; // ids of the tabs it closed
    pub fn move_tab(&self, id: &str, to: Option<&str>) -> Result<(String, String)>;  // (from, to); None creates a window
    pub fn closed(&self) -> Vec<TabSummary>;
}
```

Any id that fails `valid_id` is `Error::Precondition("Unknown tab.")`. Phase 1's store also behaves as follows, and this plan relies on it:
- `window_ids()` creates `w-1` (unsaved until the first change) only when `workspace.json` is missing. A workspace that lists no windows gives an empty list, and so does one that can't be read (a permissions problem), which `add_window` then refuses with an `Err`.
- A window id that passes `valid_id` but isn't in the workspace is `Precondition("This window was closed.")` in `window_view`, `create_tab`, `close_window`, `reopen_tab` and `move_tab`.
- `save_draft` updates closed tabs too (a save already on its way when the tab closed).
- `move_tab` removes the tab from its window (whose `active` moves to a neighbour, as `close_tab` does), appends it to the target, and makes it the target's active tab only if the target had none; `None` creates a window that shows it. Moving a tab to the window it's already in returns `(from, from)` and changes nothing.
- `TabSummary` derives `Debug, Clone, PartialEq, Serialize`.

**Shell (Phase 1):**
- `src-tauri/src/state.rs`:
  - `AppState` has `sessions: SessionStore`, `analyses: Mutex<HashMap<String, JoinHandle<()>>>` (keyed by tab id), `projects` and `brief_tasks`.
  - `pub fn evict_unused_projects(&self)` on `AppState`, and the free function `pub fn cancel_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str)`. Both are `pub`.
- `src-tauri/src/events.rs` imports `serde::Serialize` and `tauri::{AppHandle, Emitter}`, and defines:
  - `pub const WORKSPACE_CHANGED: &str = "workspace-changed";` and `#[derive(Clone, Serialize)] #[serde(rename_all = "camelCase")] pub struct WorkspaceChanged { pub windows: Vec<String> }`.
  - The emit helper `pub fn workspace_changed(app: &AppHandle, windows: Vec<String>)` (and `brief_status` for briefs). This plan emits `workspace-changed` only through `workspace_changed`.
  - It declares none of Phase 3's events (`SETTINGS_CHANGED`, `INSTRUCTIONS_CHANGED`, `MENU`, `InstructionsChanged`, `MenuEvent`). Tasks 5 and 7 add them.
- `src-tauri/src/commands.rs`:
  - Imports, among others: `use livebrainstorm_core::sessions::{valid_id, LegacyDraft, Recorder, Tab, WindowView};`, `use tauri::{AppHandle, Manager, State};` (no `Emitter`), `use crate::events::{self, BriefState};` and `use crate::state::{cancel_task, replace_task, AppState, ProjectEntry};`.
  - The private helper `fn check_id(id: &str) -> Result<()>`, which returns `Precondition("Unknown tab.")` for an id that fails `valid_id`. Every new command here checks its ids with it.
  - The session commands (`list_workspace`, `close_tab`, …), registered in `lib.rs`.
- `src-tauri/src/lib.rs`: `mod commands; mod events; mod provider; mod secrets; mod state;`, `use tauri::{Manager, RunEvent};`, `.plugin(tauri_plugin_dialog::init())`, a `.setup(|app| { … app.manage(AppState::new(&data_dir, &config_dir)); Ok(()) })`, and a `.run(|app, event| …)` closure whose `RunEvent::Exit` arm stops the local server.
- `tauri.conf.json` still declares one static window (label `main`), and `capabilities/default.json` still says `"windows": ["main"]`.

**Frontend (Phases 1 and 2):**
- `src/lib/ipc.ts`: TS mirrors of `Tab`, `WindowView` and `TabSummary`, plus Phase 1's `api` wrappers (the session ones end with `importLegacyDrafts`). `TabSummary` is `{ id: string; root: string | null; title: string | null; firstLine: string; updatedAt: number; closedAt: number | null }`. `errorMessage(e: unknown): string` is exported too.
- `src/lib/windowId.ts`: `export function currentWindowId(): string`. It returns the window's label when the label matches `/^w-/`, else `"w-1"`.
- `src/lib/events.ts`: `import { listen, type UnlistenFn } from "@tauri-apps/api/event";`, then `onBriefStatus(fn)` and `onWorkspaceChanged(fn)`, each returning `Promise<UnlistenFn>`.
- `src/lib/tabTitle.ts`: `export function tabTitle(t: { title: string | null; firstLine: string }): string`. It returns the title, else `firstLine`, else `"Untitled"`.
- `src/lib/tabList.ts` (Phase 2): `rowTitle(t: Pick<TabSummary, "title" | "firstLine">, liveDraft: string | null): string`, `windowTitle(title: string | null): string` (`"LiveBrainstorm — <title>"`, or `"LiveBrainstorm"` for `null`) and `afterClose(order: string[], id: string): string | null`.
- `src/lib/tabs.ts` (Phase 2), the per-window store:
  - Module state: `state: WorkspaceState` changed only through `set(patch)` (`set({ error })` reports a failed tab action), `sessions: Map<string, TabSession>` (read by `sessionFor(tabId)`), and the draft saver `saver = createSaver<string>(…)` (500 ms, keyed by tab id).
  - Private helpers: `drop(id)` disposes a tab's `TabSession` and forgets it; `show(s: TabSession | null)` shows a tab; `sessionOf(tab: Tab)` creates or finds a session; `order()` is the sidebar's visual order; `refresh()` refetches `list_workspace`, and on every `workspace-changed` (whichever windows it lists).
  - `close(tabId)` first shows `afterClose(order(), tabId)` with `switchTo` (so the backend's stored-order pick never flashes on screen), then flushes, calls `api.closeTab`, `drop`s the session, shows nothing if there was no next tab, and refreshes.
  - It exports:
  ```ts
  export const tabs: {
    load(): Promise<void>; switchTo(tabId: string): Promise<void>; newTab(root: string | null): Promise<void>;
    close(tabId: string): Promise<void>; reopen(tabId: string): Promise<void>; remove(tabId: string): Promise<void>;
    rename(tabId: string, title: string | null): Promise<void>; next(dir: 1 | -1): void;
    refresh(): Promise<void>; setContext(root: string | null, key: string | null, instructions: string): void;
    setDebounce(ms: number): void;
    // Phase 2 ends the object with the comment line "// Phase 3 (Task 4) adds flush() here, …"; Task 4 replaces it with flush()
  };
  export function sessionFor(tabId: string): TabSession | undefined;
  export function useWorkspace(): WorkspaceState;   // { windowId, view, activeId, error, saveError }
  ```
  Its test, `src/lib/tabs.test.ts`, mocks `./ipc` with a `vi.hoisted` `api` object and loads a fresh store per test with `loadStore(first: WindowView)`, using the helpers `summary(id, root)`, `full(id, root, draft?)`, `view(tabs, active, over?)` and the fixture `stored` (stored order a1, b1, a2; the sidebar shows a1, a2 | b1).
- `src/lib/projects.ts` (Phase 2), the per-window project store. It holds each root's instructions (`""` is no project), saves them with its own `createSaver<ProjectInstructions>`, and exports:
  ```ts
  export const projects: { …; reloadInstructions(root: string | null): Promise<void>; flush(): Promise<void>; … };
  export function projectsState(): ProjectsState;   // { …; instructions: Record<string, ProjectInstructions>; templates: Template[]; … }
  ```
  `reloadInstructions` saves the pending edit, fetches `get_instructions`, stores the project's instructions and the templates, and pushes the context to that root's tabs. `""` and `null` both mean no project.
- `src/lib/actions.ts` (Phase 2) exports the following. `src/App.tsx` calls `useActions` once, with handlers for every action except `new-window` and `close-window`. Phase 2's `useActions` ignores a key while a modal `<dialog>` is open and calls `preventDefault()` only when it runs a handler.
  ```ts
  export type ActionId =
    | "new-tab" | "new-window" | "open-project" | "reopen-tab" | "close-tab" | "close-window"
    | "next-tab" | "prev-tab" | "toggle-sidebar" | "settings";      // one member per line in the file, ending with `;`
  export type KeyPress = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"> & { isComposing?: boolean };
  export function keyToActionFor(e: KeyPress, mac: boolean): ActionId | null;
  export function keyToAction(e: KeyboardEvent): ActionId | null;   // ⌘/Ctrl bindings from the spec
  export function useActions(handlers: Partial<Record<ActionId, () => void>>): void;  // window keydown listener
  ```
- `src/components/ui.tsx` (Phase 2): `export interface MenuItem { label: string; onSelect: () => void; disabled?: boolean }` and `Menu({ at, items, label, onClose })`, a popup with no submenus, keyed by item label.
- `src/components/TabRow.tsx` (Phase 2): `TabRow({ tab, active })` imports `import type { TabSummary } from "../lib/ipc";`, keeps `const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);`, opens it in `openMenu` (which ends with `setMenu(…)`), and renders `<Menu … items={[{ label: "Rename", onSelect: startRename }, { label: "Close", onSelect: () => void tabs.close(tab.id) }]} />`.
- `src/App.tsx` (Phase 2):
  - Holds `settings` / `setSettings` (`useState<Settings | null>`) and on load calls `setSettings(s); tabs.setDebounce(s.debounceMs);`.
  - Imports `api, errorMessage` from `./lib/ipc`, `IDLE_BRIEF, projects, useProjects` from `./lib/projects`, `projectName, readCollapsed, rowTitle, windowTitle, writeCollapsed` from `./lib/tabList` and `sessionFor, tabs, useWorkspace` from `./lib/tabs`.
  - Sets the title with `useEffect(() => { document.title = windowTitle(title); }, [title]);`, where `title` is the shown tab's `rowTitle` (it follows the live draft), or `null`.
  - Shows one error banner under the top bar from `const alert = openError ?? (root !== null ? (pj.summaryErrors[root] ?? null) : null);`.
- Docs: Phase 2 added a `- **Shortcuts**` bullet to AGENTS.md's "### UI and styling" saying one `keydown` listener per window handles them, and a README `### Tabs` subsection (with a "Shortcuts:" line) after `### Session instructions`. Phase 1 added `- **Loaded projects.**` under "### Project understanding" and a `- **Brief and workspace events:**` line under "## Cross-language contracts".

## Global Constraints

- **Windows are their workspace ids.** Labels are `w-<n>`, the ids in `workspace.json`. `tauri.conf.json` has `"windows": []`.
- **Building a window:** `WebviewWindowBuilder::new(app, id, WebviewUrl::App("index.html".into()))`, with the title "LiveBrainstorm", a size of 1280×820 and a minimum of 860×560.
- **Only in `setup` or an `async` command.** On Windows, `WebviewWindowBuilder::build` deadlocks in a synchronous command or an event handler.
- **Window state:** `tauri-plugin-window-state` (`"2"`), restoring `StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED`.
- **Capability (`src-tauri/capabilities/default.json`):** `"windows": ["w-*"]`. The permissions are `core:default`, `dialog:allow-open`, `core:window:allow-set-title` and `core:window:allow-close`. Window creation stays in Rust, so the webview never gets `create-webview-window`.
- **Ids over IPC:** every tab or window id a command receives passes `valid_id` (`^[tw]-[0-9a-f-]{1,40}$`) before use. Anything else is `Error::Precondition("Unknown tab.")`.
- **Closing a window:**
  - If other windows are open, `close_window(id)` runs: its tabs go to *Recently closed*, their runs are cancelled, and `workspace-changed` is emitted.
  - The last window is left untouched, which is the same as quitting.
  - ⌘Q exits without touching the workspace.
  - Closing a window's last tab leaves the window open and empty.
- **App menu.** It replaces Tauri's default, which binds ⌘W to *Close Window*.
  - **App:** About, Settings… (⌘,), Quit (⌘Q).
  - **Edit:** the predefined Undo, Redo, Cut, Copy, Paste and Select All.
  - **File:** New Tab ⌘T (on the active tab's project), New Window ⌘N, Open Project… ⌘O, Reopen Closed Tab ⌘⇧T, Close Tab ⌘W, Close Window ⌘⇧W.
  - **Window:** Next Tab ⌃Tab, Previous Tab ⌃⇧Tab, Toggle Sidebar ⌘\\.
  - **Dispatch:** each custom item goes to the focused window as `menu { id }`, and the frontend dispatches it. On Windows and Linux the same items use Ctrl.
  - **Kept from today's default menu** so ⌘H and ⌘M keep working: macOS's Services, Hide, Hide Others and Show All, and the predefined Minimize and Zoom.
- **Moving a tab:**
  - The tab row's context menu offers *Move to new window*, then one *Move to “\<window\>”* item per other window, in place of the spec's *Move to ▸ \<window\>* submenu: Phase 2's `Menu` has no submenus. Windows are named by their active tab's title.
  - The frontend flushes the tab's pending draft save first. `move_tab` then cancels the tab's running analysis, and the tab becomes the target window's active tab. The target window shows it, and the window it left shows the next tab in the sidebar's order, as closing does.
- **Window title:** `LiveBrainstorm — <shown tab's title>` (Phase 2's `windowTitle`, which follows the live draft), through `getCurrentWindow().setTitle`.
- **Events:**
  - `settings-changed` after `save_settings`.
  - `instructions-changed { root, source }` after saving instructions or templates, with `root: ""` for templates. `source` (the saving window's label) is an extra field the contract allows.
  - `menu { id }`.
  - Settings, instructions and briefs are shared by all windows, and the last save wins.
- **Process:**
  - Work on branch `feat/tabs-windows-usage`. Every commit ends with the two trailer lines shown in each task.
  - Never run `cargo fmt` across the repo; match the hand-wrapped style of about 120 columns.
  - `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests: shell changes are verified with `cargo check -p livebrainstorm` and clippy.
  - `std` locks are never held across `.await`. Spawned tasks hold no closures over borrowed arguments.
  - The UI uses semantic colour tokens and the `ui.tsx` primitives, and no browser dialogs.
  - `Precondition` messages are written as instructions to the user.
  - The agent shell has Node 20.11, so `npm run tauri dev` can't run there: the user runs it.

## Review Focus

1. **A shortcut pressed with two windows open.** Expected: ⌘W closes a tab in the focused window only, and ⌘T makes exactly one tab. Two things pin this, both in Task 5:
   - `menu` is heard with a window-scoped listener (`windowEvents.test.ts`, "menu items are heard by this window only, never through a global listener"). A JS listener with target `Any` receives every emit, `emit_to` included.
   - Keydown never runs an action whose shortcut the menu owns (`menuActions.test.ts`, "keydown never runs an action whose shortcut the menu owns…" and "runs one handler, once").
2. **Typing in the Instructions drawer while saves echo back.** Expected: the text never jumps back to an older copy, and another window shows the edit a moment later. A window with its own pending edit saves it before refetching. Pinned in Task 7 (`crossWindow.test.ts`: "never refetches this window's own save" and "saves this window's pending edit before refetching").
3. **Moving a tab right after typing, or a move that fails.** Expected: the receiving window shows the moved tab with the last edit, the window it left shows the next tab in sidebar order, and a failed move leaves the tab where it was. Pinned in Task 6 (`moveTab.test.ts`: "saves the pending draft first…", "waits for a slow save…" and "a failed move keeps the tab in this window"; `tabs.test.ts`: "moving the shown tab saves its draft first, then shows the next tab in the sidebar's order", "a failed move keeps the tab here and says why" and "a tab moved here from another window is shown") and Task 3 (`a_moved_tab_leaves_its_window_and_shows_in_the_new_one`).
4. **Closing windows in quick succession, and closing the last one.** Expected:
   - Closing two of three windows sends both windows' tabs to *Recently closed*.
   - Closing the last window keeps it and its tabs for the next launch.
   - A draft saved just after its window closed is kept.

   Pinned in Task 3: `closing_one_of_two_windows_sends_its_tabs_to_recently_closed`, `the_last_window_is_never_closed` and `a_draft_saved_after_its_window_closed_is_kept`. The decision reads the workspace, not Tauri's window list, which still lists a window that is closing.
5. **A shortcut pressed while Settings is open.** Expected: nothing happens behind the dialog. Pinned in Task 5 (`menuActions.test.ts`, "does nothing while a modal dialog is open…").

## File Structure

| File | Responsibility |
|---|---|
| `crates/livebrainstorm-core/src/menu.rs` (new) | The app menu's own items as data, and the test that ties their ids to `ActionId` |
| `crates/livebrainstorm-core/src/sessions/windows.rs` (new) | `WindowSummary` and the window-level `SessionStore` methods the shell needs |
| `src-tauri/src/windows.rs` (new) | Building a window for a workspace id, and the close rules |
| `src-tauri/src/menu.rs` (new) | The native menu bar, and sending custom items to the focused window |
| `src-tauri/src/{lib.rs, commands.rs, events.rs}` | Plugin, menu and window-event registration, `setup`, the new commands, the new events |
| `src-tauri/{tauri.conf.json, Cargo.toml, capabilities/default.json}` | No static window, the window-state plugin, the `w-*` capability |
| `src/lib/windowTitle.ts` (new) | `setTitle` for the native title (the text is Phase 2's `windowTitle`) |
| `src/lib/moveTab.ts` (new) | Move order (flush, move, forget), naming the target windows, and the context-menu items |
| `src/lib/crossWindow.ts` (new) | Which roots to refetch after another window's save, and the listener hook |
| `src/lib/{ipc.ts, events.ts, actions.ts, tabs.ts}`, `src/App.tsx`, `src/components/TabRow.tsx` | Wrappers, event listeners, menu routing, `tabs.flush` / `tabs.move` and showing an arriving tab, the *Move to* items, wiring |
| `AGENTS.md`, `README.md`, the spec | Docs, and the recorded results of the pre-implementation checks |

---

### Task 1: Pre-implementation checks 1 and 2

The spec asks for two measurements before any Phase 3 code:
1. On macOS, does ⌘Q fire `CloseRequested` for each window before `ExitRequested`?
2. Does a capability with `"windows": ["w-*"]` apply to windows created at runtime?

This task builds a throwaway logging build, has the user run it, records the results in the spec, then removes the probe. Only the spec change is committed.

Reading Tauri 2.12.1 and tao 0.37.1 predicts the results, but the spec requires a measurement:
- The predefined Quit sends AppKit's `terminate:`. That reaches tao's `applicationWillTerminate`, then `RunEvent::Exit`, with no `CloseRequested` and no `ExitRequested`.
- The ACL matches window patterns with `glob::Pattern::matches(label)` at each invoke.

The same run also measures something this phase owns. Drafts save 500 ms after typing stops, and a flush on `pagehide` may never reach Rust when a window closes or the app quits. So the run checks whether an edit typed just before closing a window, or just before ⌘Q, survives.

**Files:**
- Temporary (reverted in Step 7): `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json`, `public/probe.html`, `public/probe.js`
- Modify: `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md` (section "Pre-implementation checks")

**Interfaces:**
- Consumes: the Phase 1–2 shell as it stands, with the static `main` window from `tauri.conf.json` and the `.run(|app, event| …)` closure in `lib.rs`.
- Produces: the recorded outcome of each measurement, and from it the list of branches later tasks apply. The branches are:
  - **Quit guard**, Task 4 Step 8.
  - **Flush on close request**, Task 4 Step 9.
  - **Runtime capability**, Task 4 Step 10.
  - **Custom Quit**, Task 5 Step 10.

- [ ] **Step 1: Add the probe to the shell**

In `src-tauri/src/lib.rs`, add this block above `pub fn run()`:

```rust
// PROBE (Phase 3 pre-implementation checks 1 and 2). Removed in Task 1, Step 7.
static PROBE_T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
fn probe(what: &str) {
    let ms = PROBE_T0.get_or_init(std::time::Instant::now).elapsed().as_millis();
    eprintln!("[probe +{ms}ms] {what}");
}
```

Right after `.plugin(tauri_plugin_dialog::init())`, add:

```rust
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { .. } => probe(&format!("CloseRequested {}", window.label())),
            tauri::WindowEvent::Destroyed => probe(&format!("Destroyed {}", window.label())),
            _ => {}
        })
```

In `.setup(|app| { … })`, just before its final `Ok(())`, add the following. `w-90` matches the glob and `x-90` is the control that must not match.

```rust
            for label in ["w-90", "x-90"] {
                tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("probe.html".into()))
                    .title(format!("probe {label}"))
                    .inner_size(460.0, 220.0)
                    .build()?;
            }
            probe("setup done");
```

Make this the first statement inside the `.run(|app, event| { … })` closure:

```rust
            match &event {
                RunEvent::ExitRequested { code, .. } => probe(&format!("ExitRequested code={code:?}")),
                RunEvent::Exit => probe("Exit"),
                _ => {}
            }
```

- [ ] **Step 2: Add the probe page and the probe capability**

Create `public/probe.html`. Vite serves `public/` at the dev server's root. The script is a file, not inline, because the CSP allows only `'self'` scripts.

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>probe</title>
  </head>
  <body style="font: 13px ui-monospace, monospace; padding: 12px">
    <pre id="out">probing…</pre>
    <script type="module" src="/probe.js"></script>
  </body>
</html>
```

Create `public/probe.js`. Both probes are *plugin* commands, because app commands from the app's own origin are allowed without any capability. Only plugin commands such as events, window and dialog show whether a capability applies.

```js
// Pre-implementation check 2: does this window's label get the capability's permissions?
const internals = window.__TAURI_INTERNALS__;
const label = internals.metadata.currentWindow.label;

async function probe(name, cmd, args) {
  try {
    return `${name}: allowed (${JSON.stringify(await internals.invoke(cmd, args))})`;
  } catch (e) {
    return `${name}: BLOCKED (${e})`;
  }
}

const lines = [
  `window ${label}`,
  await probe("core:default (plugin:app|version)", "plugin:app|version", {}),
  await probe("core:window:allow-set-title", "plugin:window|set_title", { label, value: `probe ${label} ok` }),
];
document.getElementById("out").textContent = lines.join("\n");
```

Replace `src-tauri/capabilities/default.json` with:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "PROBE for pre-implementation check 2; restored in Task 1, Step 7.",
  "windows": ["main", "w-*"],
  "permissions": ["core:default", "dialog:allow-open", "core:window:allow-set-title"]
}
```

- [ ] **Step 3: Check that the probe build compiles**

Run: `cargo check -p livebrainstorm`
Expected: compiles. A warning that names only `probe` code is acceptable for this throwaway build.

- [ ] **Step 4: Have the user run the probe (the agent shell can't run Vite 7)**

Hand the user the script below and wait for their notes and the `[probe …]` lines from the terminal. Each "start" means `npm run tauri dev` in the worktree. Three windows open: the app, "probe w-90" and "probe x-90".

1. **Check 2.** Read the text in both probe windows.
2. **⌘Q.** Click into the app window and press ⌘Q. Copy every `[probe …]` line printed after the key press.
3. **Dock Quit.** Start again. Right-click the app's Dock icon and choose Quit. Copy the lines.
4. **One by one.** Start again. Close "probe x-90" with its red button, then "probe w-90", then the app window. Copy the lines.
5. **Edit, then close a window.** Start again. In any tab of the app window, type a new word and, without pausing, click the app window's red close button. Close both probe windows. Start again. Is the word in the draft?
6. **Edit, then ⌘Q.** In the same tab, type another new word and press ⌘Q without pausing. Start again. Is it there? Then remove both test words.

- [ ] **Step 5: Classify the results**

For each ⌘Q and Dock Quit log:
- **A:** there's no `CloseRequested` line before `Exit`. This is the predicted outcome.
- **B:** `ExitRequested` comes before every `CloseRequested`.
- **C:** a `CloseRequested` comes before any `ExitRequested`, or there's a `CloseRequested` with no `ExitRequested` at all.

For check 2:
- **Pass:** `w-90` shows "allowed" for both probes and `x-90` shows "BLOCKED" for both.
- **Fail:** `w-90` shows "BLOCKED".
- **Inconclusive:** `x-90` shows "allowed". Something other than this capability grants the permission, so stop and report it.

- [ ] **Step 6: Record the results and the branches in the spec**

In `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md`, under `### Pre-implementation checks`, insert this block after list item 3 and before `### Phases`. Fill each `<…>` from the user's notes. Paste the `[probe …]` lines verbatim, or summarise a long run as "CloseRequested x-90, Destroyed x-90, …".

```markdown
**Result for checks 1 and 2 (<YYYY-MM-DD>, Tauri 2.12.1, macOS <version>):**
- Check 1, ⌘Q with three windows open: `<lines>`. Outcome <A, B or C>.
- Check 1, Dock → Quit: `<lines>`. Outcome <A, B or C>.
- Check 1, closing the windows one at a time: `<lines>`. After the last close, `ExitRequested code=None` and then `Exit`: <yes or no>.
- Check 1, drafts: a word typed just before closing a window <survived or was lost>. A word typed just before ⌘Q <survived or was lost>.
- Check 2: `w-90` <allowed or BLOCKED> both probes, and `x-90` <allowed or BLOCKED> both. The `w-*` glob <applies or doesn't apply> to windows created at runtime.
- Phase 3 therefore applies: <the branch names from the table in the plan's Task 1, Step 6, or "no branches">.
```

Choose the branches with this table:

| Result | Branch to apply |
|---|---|
| ⌘Q outcome A, Dock Quit outcome A | none for quitting |
| ⌘Q or Dock Quit outcome B | **Quit guard** (Task 4, Step 8) |
| ⌘Q outcome C | **Quit guard** and **Custom Quit** (Task 5, Step 10) |
| Dock Quit outcome C | **Stop and ask the user.** No hook runs before those close requests, so a Dock quit would send every window's tabs to *Recently closed*. The design needs revisiting. |
| A word typed just before ⌘Q was lost | **Custom Quit** (Task 5, Step 10), which needs the **Quit guard** too |
| A word typed just before closing a window was lost | **Flush on close request** (Task 4, Step 9) |
| Check 2 fail | **Runtime capability** (Task 4, Step 10) |
| Check 2 inconclusive | **Stop and ask the user** |

- [ ] **Step 7: Remove the probe**

Run:
```bash
git checkout -- src-tauri/src/lib.rs src-tauri/capabilities/default.json
rm -f public/probe.html public/probe.js
rmdir public 2>/dev/null || true
git status --short
```
Expected: only `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md` is modified. If `public/` held files before this task, `rmdir` leaves it in place, which is correct.

- [ ] **Step 8: Commit**

```bash
git add docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md
git commit -m "docs(spec): record the quit event order, draft survival and the w-* capability check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 2: Core: the app menu's items as data, tied to the frontend's action ids

**Files:**
- Create: `crates/livebrainstorm-core/src/menu.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (`pub mod menu;`)

**Interfaces:**
- Consumes: `ActionId` in `src/lib/actions.ts` (Phase 2). The test reads the file, so the declaration must keep this shape: the keyword line `export type ActionId =`, double-quoted members, and a closing `;` before any other `;`. Phase 2 writes it one member per line:
  ```ts
  export type ActionId =
    | "new-tab"
    | "new-window"
    | "open-project"
    | "reopen-tab"
    | "close-tab"
    | "close-window"
    | "next-tab"
    | "prev-tab"
    | "toggle-sidebar"
    | "settings";
  ```
- Produces, in `livebrainstorm_core::menu`:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct MenuItemSpec { pub id: &'static str, pub label: &'static str, pub accelerator: &'static str }`
  - `pub static SETTINGS: MenuItemSpec`
  - `pub static FILE: [&[MenuItemSpec]; 3]` and `pub static WINDOW: [&[MenuItemSpec]; 2]`, each a list of groups with a separator between them
  - `pub fn items() -> impl Iterator<Item = &'static MenuItemSpec>`, every custom item in menu order
  - `pub fn is_action(id: &str) -> bool`

- [ ] **Step 1: Write the module with its tests, and empty data**

Create `crates/livebrainstorm-core/src/menu.rs`:

```rust
//! The app menu's own items, as data. Each id is one of the frontend's `ActionId`s
//! (`src/lib/actions.ts`): the shell builds the native menu from these
//! (`src-tauri/src/menu.rs`) and sends a chosen item's id to the focused window as
//! `menu { id }`. Predefined items (About, Quit, Hide, the Edit menu, Minimize) aren't
//! listed; the OS runs them. Accelerators use Tauri's syntax: `CmdOrCtrl` is ⌘ on macOS
//! and Ctrl elsewhere, and `Ctrl` is the Control key (⌃) everywhere.

/// One custom menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItemSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub accelerator: &'static str,
}

/// In the app menu, after About.
pub static SETTINGS: MenuItemSpec = MenuItemSpec { id: "", label: "", accelerator: "" };

/// The File menu, as groups with a separator between them.
pub static FILE: [&[MenuItemSpec]; 3] = [&[], &[], &[]];

/// The Window menu's own items; the shell puts the predefined Minimize and Zoom above them.
pub static WINDOW: [&[MenuItemSpec]; 2] = [&[], &[]];

/// Every custom item, in menu order.
pub fn items() -> impl Iterator<Item = &'static MenuItemSpec> {
    std::iter::once(&SETTINGS).chain(FILE.iter().chain(WINDOW.iter()).flat_map(|group| group.iter()))
}

/// Whether a menu event's id is one of ours, which the frontend handles. Predefined
/// items have ids of their own and are never forwarded.
pub fn is_action(id: &str) -> bool {
    items().any(|item| item.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_spec_items_and_shortcuts() {
        let got: Vec<(&str, &str)> = items().map(|i| (i.id, i.accelerator)).collect();
        assert_eq!(
            got,
            [
                ("settings", "CmdOrCtrl+Comma"),
                ("new-tab", "CmdOrCtrl+T"),
                ("new-window", "CmdOrCtrl+N"),
                ("open-project", "CmdOrCtrl+O"),
                ("reopen-tab", "CmdOrCtrl+Shift+T"),
                ("close-tab", "CmdOrCtrl+W"),
                ("close-window", "CmdOrCtrl+Shift+W"),
                ("next-tab", "Ctrl+Tab"),
                ("prev-tab", "Ctrl+Shift+Tab"),
                ("toggle-sidebar", "CmdOrCtrl+Backslash"),
            ]
        );
    }

    #[test]
    fn ids_and_shortcuts_are_unique() {
        let n = items().count();
        assert_eq!(items().map(|i| i.id).collect::<HashSet<_>>().len(), n);
        assert_eq!(items().map(|i| i.accelerator).collect::<HashSet<_>>().len(), n);
    }

    #[test]
    fn predefined_and_unknown_ids_are_not_forwarded() {
        assert!(is_action("close-tab"));
        for id in ["quit", "about", "hide", "copy", "", "Close-Tab"] {
            assert!(!is_action(id), "{id}");
        }
    }

    /// Cross-language contract: the menu's ids are exactly the frontend's `ActionId`s.
    #[test]
    fn every_item_is_a_frontend_action_and_every_action_has_an_item() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/lib/actions.ts");
        let ts = std::fs::read_to_string(path).expect("src/lib/actions.ts (Phase 2)");
        let union = ts.split("export type ActionId =").nth(1).expect("`export type ActionId =` in actions.ts");
        let union = union.split(';').next().unwrap_or_default();
        let mut actions: Vec<&str> = union.split('"').skip(1).step_by(2).collect();
        let mut ids: Vec<&str> = items().map(|i| i.id).collect();
        actions.sort_unstable();
        ids.sort_unstable();
        assert_eq!(ids, actions, "menu.rs ids vs ActionId in src/lib/actions.ts");
    }
}
```

In `crates/livebrainstorm-core/src/lib.rs`, add `pub mod menu;` after `pub mod local;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core menu::tests`
Expected: FAIL. `the_spec_items_and_shortcuts` and the `ActionId` test fail on their `assert_eq!`, and `predefined_and_unknown_ids_are_not_forwarded` fails on `is_action("close-tab")`.

- [ ] **Step 3: Fill in the items**

Replace the three placeholder statics with:

```rust
/// In the app menu, after About.
pub static SETTINGS: MenuItemSpec = MenuItemSpec { id: "settings", label: "Settings…", accelerator: "CmdOrCtrl+Comma" };

/// The File menu, as groups with a separator between them.
pub static FILE: [&[MenuItemSpec]; 3] = [
    &[
        MenuItemSpec { id: "new-tab", label: "New Tab", accelerator: "CmdOrCtrl+T" },
        MenuItemSpec { id: "new-window", label: "New Window", accelerator: "CmdOrCtrl+N" },
        MenuItemSpec { id: "open-project", label: "Open Project…", accelerator: "CmdOrCtrl+O" },
    ],
    &[MenuItemSpec { id: "reopen-tab", label: "Reopen Closed Tab", accelerator: "CmdOrCtrl+Shift+T" }],
    &[
        MenuItemSpec { id: "close-tab", label: "Close Tab", accelerator: "CmdOrCtrl+W" },
        MenuItemSpec { id: "close-window", label: "Close Window", accelerator: "CmdOrCtrl+Shift+W" },
    ],
];

/// The Window menu's own items; the shell puts the predefined Minimize and Zoom above them.
pub static WINDOW: [&[MenuItemSpec]; 2] = [
    &[
        MenuItemSpec { id: "next-tab", label: "Next Tab", accelerator: "Ctrl+Tab" },
        MenuItemSpec { id: "prev-tab", label: "Previous Tab", accelerator: "Ctrl+Shift+Tab" },
    ],
    &[MenuItemSpec { id: "toggle-sidebar", label: "Toggle Sidebar", accelerator: "CmdOrCtrl+Backslash" }],
];
```

- [ ] **Step 4: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core menu::tests && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: the four menu tests PASS, the core suite passes, and clippy prints no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/menu.rs crates/livebrainstorm-core/src/lib.rs
git commit -m "feat(core): app menu items as data, checked against the frontend's ActionId

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 3: Core: window helpers on `SessionStore`

Rust allows an `impl` block for a type anywhere in its own crate, so these methods live in a new file and use only Phase 1's public API. The tests also pin two Phase 1 behaviours the shell relies on:
- A draft saved after its window closed is kept.
- A moved tab leaves its old window's `active`.

**Files:**
- Create: `crates/livebrainstorm-core/src/sessions/windows.rs`
- Modify: `crates/livebrainstorm-core/src/sessions/mod.rs` (Phase 1): add `mod windows;` next to the other `mod` lines, and `pub use windows::WindowSummary;` next to the other re-exports.

**Interfaces:**
- Consumes (Phase 1, `livebrainstorm_core::sessions`):
  ```rust
  pub struct TabSummary { pub id: String, pub root: Option<String>, pub title: Option<String>,
                          pub first_line: String, pub updated_at: u64, pub closed_at: Option<u64> }
  pub struct WindowView { pub window: String, pub tabs: Vec<TabSummary>, pub active: Option<Tab>,
                          pub closed: Vec<TabSummary>, pub fresh: bool, pub unreadable: Vec<String> }
  impl SessionStore {
      pub fn new(dir: PathBuf) -> Self;
      pub fn window_ids(&self) -> Vec<String>;                       // creates w-1 if no workspace
      pub fn window_view(&self, window: &str) -> Result<WindowView>;
      pub fn add_window(&self) -> Result<String>;                     // allocates w-<next_window>
      pub fn create_tab(&self, window: &str, root: Option<String>) -> Result<Tab>;  // appended, made active
      pub fn tab(&self, id: &str) -> Result<Tab>;                     // any tab file, open or closed
      pub fn save_draft(&self, id: &str, draft: String) -> Result<bool>;  // true when the first line changed
      pub fn rename_tab(&self, id: &str, title: Option<String>) -> Result<()>;
      pub fn activate(&self, window: &str, id: &str) -> Result<()>;
      pub fn close_window(&self, window: &str) -> Result<Vec<String>>; // ids of the tabs it closed
      pub fn move_tab(&self, id: &str, to: Option<&str>) -> Result<(String, String)>;  // (from, to)
      pub fn closed(&self) -> Vec<TabSummary>;
  }
  ```
  `TabSummary` must derive `Debug` and `Clone`. It crosses IPC inside `WindowView`, so it already derives `Serialize`. If `Debug` or `Clone` is missing, add it in `sessions/mod.rs`.
- Produces (re-exported from `livebrainstorm_core::sessions`):
  - `#[derive(Debug, Clone, Serialize)] #[serde(rename_all = "camelCase")] pub struct WindowSummary { pub id: String, pub active: Option<TabSummary>, pub tabs: usize }`
  - `SessionStore::windows_to_open(&self) -> Result<Vec<String>>`: every workspace window in order, or one new empty window if the workspace lists none.
  - `SessionStore::close_window_unless_last(&self, window: &str) -> Result<Option<Vec<String>>>`: `Some(closed tab ids)` when another window is open, `None` for the last window.
  - `SessionStore::windows(&self) -> Result<Vec<WindowSummary>>`: every window in workspace order, with its active tab's summary.

- [ ] **Step 1: Write the file with its tests, and stub bodies**

Create `crates/livebrainstorm-core/src/sessions/windows.rs`:

```rust
//! Window-level operations for the shell's runtime windows (Phase 3), built on
//! `SessionStore`'s public API.

use serde::Serialize;

use super::{SessionStore, TabSummary};
use crate::Result;

/// One window for the tab menu's "Move to …" items, named there by its active tab.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummary {
    pub id: String,
    pub active: Option<TabSummary>,
    pub tabs: usize,
}

impl SessionStore {
    /// The windows to build at launch: every workspace window, in order, or one new
    /// empty window if the workspace lists none, so the app never starts without one.
    pub fn windows_to_open(&self) -> Result<Vec<String>> {
        unimplemented!()
    }

    /// A window's close request. With another window open, its tabs go to Recently
    /// closed and it leaves the workspace: returns the closed tab ids. The last window is
    /// left as it is, because closing it quits and the next launch restores it: `None`.
    /// Decided from the workspace, not Tauri's window list, which still lists a window
    /// that is closing.
    pub fn close_window_unless_last(&self, _window: &str) -> Result<Option<Vec<String>>> {
        unimplemented!()
    }

    /// Every window with its active tab, in workspace order.
    pub fn windows(&self) -> Result<Vec<WindowSummary>> {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        (dir, store)
    }

    fn ids(tabs: &[TabSummary]) -> Vec<&str> {
        tabs.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn the_last_window_is_never_closed() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let tab = s.create_tab(&w1, Some("/p".into())).unwrap();
        assert_eq!(s.close_window_unless_last(&w1).unwrap(), None);
        assert_eq!(s.window_ids(), [w1.as_str()]);
        assert_eq!(ids(&s.window_view(&w1).unwrap().tabs), [tab.id.as_str()]);
        assert!(s.closed().is_empty(), "nothing goes to Recently closed");
    }

    #[test]
    fn closing_one_of_two_windows_sends_its_tabs_to_recently_closed() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let w2 = s.add_window().unwrap();
        let a = s.create_tab(&w2, None).unwrap();
        let b = s.create_tab(&w2, Some("/p".into())).unwrap();
        let mut closed = s.close_window_unless_last(&w2).unwrap().expect("another window is open");
        closed.sort();
        let mut both = vec![a.id.clone(), b.id.clone()];
        both.sort();
        assert_eq!(closed, both);
        assert_eq!(s.window_ids(), [w1.as_str()]);
        let recently: Vec<String> = s.closed().into_iter().map(|t| t.id).collect();
        assert!(recently.contains(&a.id) && recently.contains(&b.id), "{recently:?}");
        // A second close request right after (two windows closed in a row) finds w1 alone.
        assert_eq!(s.close_window_unless_last(&w1).unwrap(), None);
    }

    #[test]
    fn a_draft_saved_after_its_window_closed_is_kept() {
        // The webview's pending draft save can land after the shell closed its window.
        let (_dir, s) = store();
        let w2 = s.add_window().unwrap();
        let tab = s.create_tab(&w2, None).unwrap();
        s.close_window_unless_last(&w2).unwrap();
        s.save_draft(&tab.id, "typed just before closing".into()).unwrap();
        assert_eq!(s.tab(&tab.id).unwrap().draft, "typed just before closing");
        assert!(s.closed().iter().any(|t| t.id == tab.id), "and the tab stays closed");
    }

    #[test]
    fn launch_opens_every_workspace_window_in_order() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let w2 = s.add_window().unwrap();
        assert_eq!(s.windows_to_open().unwrap(), [w1, w2]);
    }

    #[test]
    fn launch_never_opens_zero_windows() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("workspace.json"), r#"{"version":1,"nextWindow":3,"windows":[]}"#).unwrap();
        let s = SessionStore::new(sessions);
        let opened = s.windows_to_open().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(s.window_ids(), opened, "and it's saved in the workspace");
    }

    #[test]
    fn windows_are_named_by_their_active_tab() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let w2 = s.add_window().unwrap();
        s.create_tab(&w1, None).unwrap();
        let b = s.create_tab(&w1, Some("/p".into())).unwrap(); // made active
        s.rename_tab(&b.id, Some("Checkout retry".into())).unwrap();
        let list = s.windows().unwrap();
        assert_eq!(list.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), [w1.as_str(), w2.as_str()]);
        assert_eq!(list[0].tabs, 2);
        let active = list[0].active.as_ref().expect("w1 has an active tab");
        assert_eq!((active.id.as_str(), active.title.as_deref()), (b.id.as_str(), Some("Checkout retry")));
        assert_eq!((list[1].tabs, list[1].active.is_none()), (0, true));
    }

    #[test]
    fn a_moved_tab_leaves_its_window_and_shows_in_the_new_one() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let a = s.create_tab(&w1, None).unwrap();
        let b = s.create_tab(&w1, None).unwrap(); // active
        let (from, to) = s.move_tab(&b.id, None).unwrap();
        assert_eq!(from, w1);
        assert!(s.window_ids().contains(&to), "None makes a new window");
        s.activate(&to, &b.id).unwrap(); // what the move_tab command does next
        let there = s.window_view(&to).unwrap();
        assert_eq!(ids(&there.tabs), [b.id.as_str()]);
        assert_eq!(there.active.map(|t| t.id), Some(b.id.clone()));
        let here = s.window_view(&w1).unwrap();
        assert_eq!(ids(&here.tabs), [a.id.as_str()]);
        assert_ne!(here.active.map(|t| t.id), Some(b.id.clone()), "the old window no longer shows it");
        // Into an existing window, the tab goes last.
        assert_eq!(s.move_tab(&a.id, Some(to.as_str())).unwrap(), (w1.clone(), to.clone()));
        assert_eq!(ids(&s.window_view(&to).unwrap().tabs), [b.id.as_str(), a.id.as_str()]);
    }
}
```

In `crates/livebrainstorm-core/src/sessions/mod.rs`, add `mod windows;` next to the existing `mod` lines and `pub use windows::WindowSummary;` next to the existing re-exports.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core sessions::windows::tests`
Expected: FAIL. Six of the seven tests panic with `not implemented`. `a_moved_tab_leaves_its_window_and_shows_in_the_new_one` already passes, because it uses only Phase 1's API.

- [ ] **Step 3: Implement**

Replace the three stub bodies:

```rust
    pub fn windows_to_open(&self) -> Result<Vec<String>> {
        let ids = self.window_ids();
        if !ids.is_empty() {
            return Ok(ids);
        }
        Ok(vec![self.add_window()?])
    }

    pub fn close_window_unless_last(&self, window: &str) -> Result<Option<Vec<String>>> {
        if !self.window_ids().iter().any(|w| w != window) {
            return Ok(None);
        }
        self.close_window(window).map(Some)
    }

    pub fn windows(&self) -> Result<Vec<WindowSummary>> {
        let mut out = Vec::new();
        for id in self.window_ids() {
            let view = self.window_view(&id)?;
            let active_id = view.active.map(|tab| tab.id);
            let active = view.tabs.iter().find(|t| Some(&t.id) == active_id.as_ref()).cloned();
            out.push(WindowSummary { tabs: view.tabs.len(), active, id });
        }
        Ok(out)
    }
```

(Keep each method's doc comment from Step 1. Rename `_window` back to `window`.)

- [ ] **Step 4: Run the tests and lint**

Run: `cargo test -p livebrainstorm-core sessions:: && cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets`
Expected: the seven new tests and every Phase 1 `sessions` test PASS, and clippy prints no warnings.

Phase 1 pins both behaviours these two tests rely on (`a_closed_tab_still_takes_a_late_draft_save` and `moving_a_tab` in `sessions/store.rs`). If one of them fails here anyway, Phase 1's store is incomplete: fix it in `sessions/store.rs`, and don't loosen the test:
- **`a_draft_saved_after_its_window_closed_is_kept`:** `SessionStore::save_draft` refuses closed tabs. Make it update any existing tab file that passes `valid_id`, open or closed. Only `analyze` needs an open tab, through `open_tab_root`.
- **`a_moved_tab_leaves_its_window_and_shows_in_the_new_one`, at the `assert_ne!`:** `SessionStore::move_tab` leaves the source window's `active` on the moved tab. Remove it with `WindowEntry::remove_tab`, which moves `active` the way `close_tab` does.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/sessions
git commit -m "feat(core): window helpers on SessionStore: windows to open, close unless last, window list

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 4: Runtime windows: built from the workspace, with window state, the capability glob, the close rules and titles

**Files:**
- Modify: `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `Cargo.lock`, `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/windows.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/commands.rs`
- Modify: `src/lib/ipc.ts`, `src/lib/tabs.ts` (`flush`), `src/lib/tabs.test.ts`, `src/App.tsx` (the title effect)
- Create: `src/lib/windowTitle.ts`, `src/lib/windowTitle.test.ts`
- Branch-only, per Task 1: `src-tauri/src/state.rs` (**Quit guard**), `src/App.tsx` (**Flush on close request**)

**Interfaces:**
- Consumes:
  - From Task 3: `SessionStore::windows_to_open(&self) -> Result<Vec<String>>`, `SessionStore::close_window_unless_last(&self, window: &str) -> Result<Option<Vec<String>>>`, and `SessionStore::add_window(&self) -> Result<String>` / `close_window(&self, window: &str) -> Result<Vec<String>>` (Phase 1).
  - From Phase 1:
    - `AppState { sessions: SessionStore, analyses: Mutex<HashMap<String, JoinHandle<()>>>, … }`, `pub fn AppState::evict_unused_projects(&self)` and `pub fn cancel_task(slots: &Mutex<HashMap<String, JoinHandle<()>>>, key: &str)` (`src-tauri/src/state.rs`).
    - `pub fn events::workspace_changed(app: &AppHandle, windows: Vec<String>)`, which emits `workspace-changed` (`WorkspaceChanged { windows }`) to every window.
    - `src-tauri/src/commands.rs` imports `use tauri::{AppHandle, Manager, State};` and `use crate::events::{self, BriefState};`.
  - From Phase 2:
    - `windowTitle(title: string | null): string` and `rowTitle(t, liveDraft): string` in `src/lib/tabList.ts`.
    - In `src/App.tsx`: `const title = activeSummary ? rowTitle(activeSummary, view?.draft ?? null) : null;` and the effect `useEffect(() => { document.title = windowTitle(title); }, [title]);`.
    - In `src/lib/tabs.ts`: the draft saver `saver` (`createSaver<string>`, 500 ms, keyed by tab id) and the exported `tabs` object, whose last line is the comment ``// Phase 3 (Task 4) adds `flush()` here, returning `saver.flush()`; this phase flushes inside the store.``. In `src/lib/tabs.test.ts`: the hoisted `api` mock, `loadStore`, `view`, `full` and `stored`.
    - `projects.flush(): Promise<void>` (the pending instructions save; branch "Flush on close request").
- Produces:
  - Rust `windows::build(app: &AppHandle, id: &str) -> livebrainstorm_core::Result<WebviewWindow>`
  - Rust `windows::on_window_event(window: &Window, event: &WindowEvent)`
  - Command `open_window() -> Result<String>` (async)
  - TS `api.openWindow(): Promise<string>`
  - `src/lib/windowTitle.ts`: `setWindowTitle(title: string): Promise<void>` (never rejects) and `useWindowTitle(title: string): void`
  - `tabs.flush(): Promise<void>` in `src/lib/tabs.ts`: saves the pending draft edit now
  - With the Quit guard: `AppState.quitting: AtomicBool`

- [ ] **Step 1: Write the failing tests for the window title and `tabs.flush`**

Phase 2's `windowTitle` (in `src/lib/tabList.ts`) already builds the text, and its tests pin it. This task only sends that text to the native window. Create `src/lib/windowTitle.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

const win = vi.hoisted(() => ({ setTitle: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));

import { setWindowTitle } from "./windowTitle";

beforeEach(() => {
  win.setTitle.mockReset();
  vi.stubGlobal("document", { title: "" }); // vitest runs in node: there is no DOM
});

describe("setWindowTitle", () => {
  it("sets the native title, which Tauri doesn't copy from document.title", async () => {
    win.setTitle.mockResolvedValue(undefined);
    await setWindowTitle("LiveBrainstorm — Checkout retry");
    expect(win.setTitle).toHaveBeenCalledWith("LiveBrainstorm — Checkout retry");
    expect(document.title).toBe("LiveBrainstorm — Checkout retry");
  });

  it("a refused title is logged, never thrown", async () => {
    win.setTitle.mockRejectedValue("core:window:allow-set-title not allowed");
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(setWindowTitle("LiveBrainstorm")).resolves.toBeUndefined();
    expect(log).toHaveBeenCalledTimes(1);
    log.mockRestore();
  });
});
```

In `src/lib/tabs.test.ts` (Phase 2), add this test at the end of the `describe("tabs store", …)` block. Closing a window and moving a tab call `tabs.flush()` so the last edit isn't left in the 500 ms timer:

```ts
  it("flush saves the pending draft now", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    expect(api.saveDraft).not.toHaveBeenCalled();
    await tabs.flush();
    expect(api.saveDraft).toHaveBeenCalledWith("a1", "typed in a1");
  });
```

- [ ] **Step 2: Run them to verify they fail**

Run: `npx vitest run src/lib/windowTitle.test.ts src/lib/tabs.test.ts`
Expected: FAIL. `windowTitle.test.ts` fails with `Failed to resolve import "./windowTitle"`, and "flush saves the pending draft now" fails with `tabs.flush is not a function`. Phase 2's `tabs.test.ts` tests still pass.

- [ ] **Step 3: Create `src/lib/windowTitle.ts`, and add `tabs.flush`**

```ts
// The native window title follows the shown tab. Tauri doesn't copy document.title to
// the window, so it's set with setTitle (core:window:allow-set-title in the capability).
// The text comes from windowTitle in tabList.ts.
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Sets this window's title. Cosmetic: a failure is logged and leaves the previous title. */
export function setWindowTitle(title: string): Promise<void> {
  document.title = title;
  return getCurrentWindow()
    .setTitle(title)
    .catch((e: unknown) => console.error("setTitle failed", e));
}

/** Keeps this window's native title at `title`. */
export function useWindowTitle(title: string): void {
  useEffect(() => {
    void setWindowTitle(title);
  }, [title]);
}
```

In `src/lib/tabs.ts`, replace the comment line Phase 2 left at the end of the `tabs` object (``// Phase 3 (Task 4) adds `flush()` here, …``) with:

```ts
  /** Saves the pending draft edit now: before closing this window or moving a tab out of it. */
  flush: () => saver.flush(),
```

If Phase 2's `tabs` object already has a `flush` entry, leave it and skip this edit: a second `flush` in the object literal is a duplicate-property type error.

Run: `npx vitest run src/lib/windowTitle.test.ts src/lib/tabs.test.ts`
Expected: PASS: 2 tests in `windowTitle.test.ts`, and every `tabs.test.ts` test, Phase 2's and "flush saves the pending draft now".

- [ ] **Step 4: Configuration: no static window, the window-state plugin, the capability glob**

In `src-tauri/tauri.conf.json`, replace the whole `"windows": [ { … } ]` entry under `"app"` with:

```json
    "windows": [],
```

In `src-tauri/Cargo.toml`, add after `tauri-plugin-dialog = "2"`:

```toml
tauri-plugin-window-state = "2"
```

Replace `src-tauri/capabilities/default.json` with:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Every app window (labels w-<n>, one per workspace window): core IPC, the folder picker, setting its own title, and closing itself. Windows are created in Rust only, so the webview never gets create-webview-window.",
  "windows": ["w-*"],
  "permissions": ["core:default", "dialog:allow-open", "core:window:allow-set-title", "core:window:allow-close"]
}
```

- [ ] **Step 5: Create `src-tauri/src/windows.rs`**

```rust
//! Runtime windows. Each is one workspace window, labelled with its id (`w-<n>`), so
//! `tauri-plugin-window-state` (which keys on labels) restores its position and size,
//! and the capability glob `"windows": ["w-*"]` gives it IPC.

use livebrainstorm_core::{Error, Result};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent};

use crate::events;
use crate::state::{cancel_task, AppState};

/// Builds the window for workspace window `id`. Call it from `setup` or an `async`
/// command only: on Windows, building a window in a synchronous command or an event
/// handler deadlocks (WebView2).
pub fn build(app: &AppHandle, id: &str) -> Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, id, WebviewUrl::App("index.html".into()))
        .title("LiveBrainstorm")
        .inner_size(1280.0, 820.0)
        .min_inner_size(860.0, 560.0)
        .build()
        // `Provider` is the catch-all failure; its message is shown verbatim.
        .map_err(|e| Error::Provider(format!("Couldn't open a window: {e}")))
}

/// Registered with `Builder::on_window_event`.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { .. } = event {
        close_requested(window.app_handle(), window.label());
    }
}

/// With another window open, this window's tabs go to Recently closed and their runs stop.
/// The last window is left as it is: closing it quits, and the next launch restores it.
fn close_requested(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    match state.sessions.close_window_unless_last(id) {
        Ok(Some(closed)) => {
            for tab in &closed {
                cancel_task(&state.analyses, tab); // drops the claude children too (kill_on_drop)
            }
            state.evict_unused_projects();
            events::workspace_changed(app, vec![id.to_owned()]);
        }
        Ok(None) => {}
        // The window still closes; its tabs stay listed and come back at the next launch.
        Err(e) => eprintln!("LiveBrainstorm: couldn't move window {id}'s tabs to Recently closed: {e}"),
    }
}
```

- [ ] **Step 6: Wire it into `src-tauri/src/lib.rs`, and add `open_window`**

In `src-tauri/src/lib.rs`:
- Add `mod windows;` to the module list, after `mod state;` (the list stays alphabetical: `commands`, `events`, `provider`, `secrets`, `state`, `windows`).
- Right after `.plugin(tauri_plugin_dialog::init())`, add:

```rust
        // Restores each window's position and size by label (= workspace window id).
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .on_window_event(windows::on_window_event)
```

- In `.setup(|app| { … })`, after the `app.manage(AppState::new(…));` line and before `Ok(())`, add the block below. A workspace that can't be read (a permissions problem; Phase 1 never overwrites it) still gets one window, whose `list_workspace` then shows the error, rather than an app that silently fails to start.

```rust
            // One window per workspace window; tauri.conf.json declares none.
            let ids = app.state::<AppState>().sessions.windows_to_open().unwrap_or_else(|e| {
                eprintln!("LiveBrainstorm: couldn't read the saved windows: {e}");
                vec!["w-1".to_owned()]
            });
            for id in ids {
                windows::build(app.handle(), &id)?;
            }
```

- Add `commands::open_window,` to `generate_handler![…]`, right after Phase 1's `commands::import_legacy_drafts,` (the last session command).

In `src-tauri/src/commands.rs`, add `use crate::windows;` after `use crate::state::{…};`. Phase 1 already imports `crate::events` as a module (`use crate::events::{self, BriefState};`), and the event goes out through its `events::workspace_changed` helper, so no `Emitter` import is needed. Then add this section above `// ---------- live analysis ----------`:

```rust
// ---------- windows ----------

/// A new, empty window ("Open a project" shows in it). `async`: building a window in a
/// synchronous command deadlocks on Windows.
#[tauri::command]
pub async fn open_window(app: AppHandle) -> Result<String> {
    let state = app.state::<AppState>();
    let id = state.sessions.add_window()?;
    if let Err(e) = windows::build(&app, &id) {
        let _ = state.sessions.close_window(&id); // it has no tabs: this only drops the entry
        return Err(e);
    }
    events::workspace_changed(&app, vec![id.clone()]);
    Ok(id)
}
```

In `src/lib/ipc.ts`, add to `api`, right after Phase 1's `importLegacyDrafts` wrapper (the last session wrapper):

```ts
  openWindow: () => invoke<string>("open_window"),
```

- [ ] **Step 7: Frontend: the native title**

In `src/App.tsx`, replace Phase 2's title effect:

```ts
  useEffect(() => {
    document.title = windowTitle(title);
  }, [title]);
```

with this call, at the same place:

```ts
  useWindowTitle(windowTitle(title));
```

and add `import { useWindowTitle } from "./lib/windowTitle";` after the `./lib/useTabSession` import. Keep importing `windowTitle` from `./lib/tabList`: `windowTitle.ts` exports no `windowTitle`, so the two never clash.

- [ ] **Step 8: Branch "Quit guard" (only if Task 1 recorded ⌘Q or Dock Quit as outcome B or C)**

Skip this step for outcome A.

In `src-tauri/src/state.rs`, add `use std::sync::atomic::AtomicBool;` and this field to `AppState`:

```rust
    /// Set once the app starts quitting. Quitting fires window close requests
    /// (pre-implementation check 1), and those must leave the workspace untouched.
    pub quitting: AtomicBool,
```

In `AppState::new`, add `quitting: AtomicBool::new(false),`.

In `src-tauri/src/lib.rs`, make this the first statement of the `.run(|app, event| { … })` closure:

```rust
            if matches!(event, RunEvent::ExitRequested { .. }) {
                app.state::<AppState>().quitting.store(true, std::sync::atomic::Ordering::SeqCst);
            }
```

In `src-tauri/src/windows.rs`, make these the first lines of `close_requested`, after `let state = …;`:

```rust
    if state.quitting.load(std::sync::atomic::Ordering::SeqCst) {
        return; // quitting keeps every window (pre-implementation check 1)
    }
```

- [ ] **Step 9: Branch "Flush on close request" (only if Task 1 found a word typed just before closing a window was lost)**

Add `"core:window:allow-destroy"` to the `permissions` in `src-tauri/capabilities/default.json`.

In `src/App.tsx`, add this effect. Import `getCurrentWindow` from `@tauri-apps/api/window` if it isn't imported. With a close-requested listener, Tauri waits for the handler and then destroys the window. The Rust close rules still run first, and the draft save then lands in the closed tab, which Task 3's `a_draft_saved_after_its_window_closed_is_kept` covers.

```ts
  // Pre-implementation check 1: an edit typed just before closing a window was lost, so
  // this window saves its draft and instructions before it goes. Tauri waits for this
  // handler, then destroys the window.
  useEffect(() => {
    const off = getCurrentWindow().onCloseRequested(async () => {
      await Promise.all([tabs.flush(), projects.flush()]);
    });
    return () => void off.then((unlisten) => unlisten());
  }, []);
```

- [ ] **Step 10: Branch "Runtime capability" (only if Task 1's check 2 failed)**

In `src-tauri/src/windows.rs`, add `use tauri::ipc::CapabilityBuilder;`. In `build`, before `WebviewWindowBuilder::new(…)`, add:

```rust
    // Pre-implementation check 2: the w-* glob doesn't reach runtime windows, so each
    // window gets its own capability, granting exactly what default.json lists.
    app.add_capability(
        CapabilityBuilder::new(format!("window-{id}"))
            .window(id)
            .permission("core:default")
            .permission("dialog:allow-open")
            .permission("core:window:allow-set-title")
            .permission("core:window:allow-close"),
    )
    .map_err(|e| Error::Provider(format!("Couldn't open a window: {e}")))?;
```

If Step 9 also applies, add `.permission("core:window:allow-destroy")` as well.

- [ ] **Step 11: Check, lint and test**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm run typecheck && npm test`
Expected:
- `tauri-plugin-window-state` 2.x is added to `Cargo.lock`. Version 2.5.0 needs tauri 2.12, and the lockfile has 2.12.1.
- `git diff Cargo.lock | grep -A1 '^-name = "tauri"'` prints nothing: `tauri` stays at 2.12.1. If the resolver picked a newer plugin that moved `tauri` (`@tauri-apps/api` 2.12 would then no longer match it, which `tauri dev` refuses), run `cargo update -p tauri-plugin-window-state --precise 2.5.0 && cargo update -p tauri --precise 2.12.1`, then check again.
- The shell compiles and clippy prints no warnings.
- Every test passes, including `windowTitle.test.ts` and the new `tabs.test.ts` test.

- [ ] **Step 12: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/Cargo.toml Cargo.lock src-tauri/capabilities/default.json src-tauri/src src/lib/windowTitle.ts src/lib/windowTitle.test.ts src/lib/ipc.ts src/lib/tabs.ts src/lib/tabs.test.ts src/App.tsx
git commit -m "feat: runtime windows from the workspace, window state, w-* capability, close rules, titles

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 5: The app menu, routed into the Phase 2 actions

**Files:**
- Create: `src-tauri/src/menu.rs`
- Modify: `src-tauri/src/events.rs`, `src-tauri/src/lib.rs`
- Modify: `src/lib/events.ts`, `src/lib/actions.ts`, `src/App.tsx`
- Create: `src/lib/menuActions.test.ts`, `src/lib/windowEvents.test.ts`

**Interfaces:**
- Consumes:
  - From Task 2: `livebrainstorm_core::menu::{MenuItemSpec, SETTINGS, FILE, WINDOW, is_action}`, where `MenuItemSpec { id: &'static str, label: &'static str, accelerator: &'static str }`, `FILE: [&[MenuItemSpec]; 3]` and `WINDOW: [&[MenuItemSpec]; 2]`.
  - From Task 4: `api.openWindow(): Promise<string>`.
  - From Phase 2, `src/lib/actions.ts` (it already imports `useEffect` and `useRef` from `react`):
    ```ts
    export type ActionId =
      | "new-tab" | "new-window" | "open-project" | "reopen-tab" | "close-tab" | "close-window"
      | "next-tab" | "prev-tab" | "toggle-sidebar" | "settings";
    export function keyToAction(e: KeyboardEvent): ActionId | null;
    export function useActions(handlers: Partial<Record<ActionId, () => void>>): void;
    ```
  - From Phase 2, `src/App.tsx`: one `useActions({ … })` call with handlers for every action except `new-window` and `close-window`; `api`, `errorMessage`, `projects` and `tabs` imported; `useState` and `useEffect` imported from `react`; the banner `const alert = openError ?? (root !== null ? (pj.summaryErrors[root] ?? null) : null);`.
  - From Task 4: `tabs.flush(): Promise<void>`. From Phase 2: `projects.flush(): Promise<void>`.
  - From Phase 1, `src/lib/events.ts`: `import { listen, type UnlistenFn } from "@tauri-apps/api/event";`.
- Produces:
  - Rust `events::MENU: &str = "menu"` and `#[derive(Clone, Serialize)] events::MenuEvent { pub id: String }`
  - Rust `menu::build(app: &AppHandle) -> tauri::Result<Menu<Wry>>` and `menu::on_event(app: &AppHandle, event: tauri::menu::MenuEvent)`
  - TS in `events.ts`: `interface MenuEvent { id: string }` and `onMenu(fn: (e: MenuEvent) => void): Promise<UnlistenFn>`. It listens on this window only.
  - TS in `actions.ts`:
    - `ownsShortcut(action: ActionId): boolean`
    - `menuAction(id: string): ActionId | null`
    - `keydownAction(e: KeyboardEvent): ActionId | null`
    - `dispatchMenu(id: string, handlers: Partial<Record<ActionId, () => void>>, modalOpen: boolean): ActionId | null`
    - `useActions(handlers)`, same signature, new body.
  - `App.tsx` handlers for `new-window` and `close-window`.

- [ ] **Step 1: Write the failing tests**

Create `src/lib/menuActions.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { dispatchMenu, keydownAction, menuAction, ownsShortcut, type ActionId } from "./actions";

const ALL: ActionId[] = [
  "new-tab",
  "new-window",
  "open-project",
  "reopen-tab",
  "close-tab",
  "close-window",
  "next-tab",
  "prev-tab",
  "toggle-sidebar",
  "settings",
];

describe("menu actions", () => {
  it("accepts every action id and nothing else", () => {
    for (const id of ALL) expect(menuAction(id)).toBe(id);
    for (const id of ["", "quit", "about", "copy", "NEW-TAB", "constructor", "toString", "__proto__"]) {
      expect(menuAction(id)).toBeNull();
    }
  });

  it("runs one handler, once", () => {
    const close = vi.fn();
    const next = vi.fn();
    expect(dispatchMenu("close-tab", { "close-tab": close, "next-tab": next }, false)).toBe("close-tab");
    expect(close).toHaveBeenCalledTimes(1);
    expect(next).not.toHaveBeenCalled();
  });

  it("does nothing while a modal dialog is open, or for an id that isn't an action", () => {
    const close = vi.fn();
    expect(dispatchMenu("close-tab", { "close-tab": close }, true)).toBeNull();
    expect(dispatchMenu("quit", { "close-tab": close }, false)).toBeNull();
    expect(close).not.toHaveBeenCalled();
  });

  it("an action with no handler is a no-op", () => {
    expect(dispatchMenu("new-window", {}, false)).toBe("new-window");
  });

  it("the menu owns every action's shortcut", () => {
    for (const id of ALL) expect(ownsShortcut(id)).toBe(true);
  });

  it("keydown never runs an action whose shortcut the menu owns, so one keystroke is one action", () => {
    for (const key of ["t", "T", "n", "o", "w", "W", "\\", ",", "Tab"]) {
      for (const metaKey of [false, true]) {
        for (const ctrlKey of [false, true]) {
          for (const shiftKey of [false, true]) {
            const e = { key, code: "", metaKey, ctrlKey, shiftKey, altKey: false, repeat: false, target: null } as unknown as KeyboardEvent;
            const action = keydownAction(e);
            expect(action === null || !ownsShortcut(action)).toBe(true);
          }
        }
      }
    }
  });
});
```

Phase 2's `keyToAction` reads only `key`, the modifier flags and `isComposing`, so a plain object stands in for the event in vitest's node environment (no DOM).

Create `src/lib/windowEvents.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (e: { payload: unknown }) => void;
const tauri = vi.hoisted(() => ({
  globalListen: vi.fn(async (_event: string, _handler: Handler) => () => {}),
  windowListen: vi.fn(async (_event: string, _handler: Handler) => () => {}),
}));
vi.mock("@tauri-apps/api/event", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/event")>()),
  listen: tauri.globalListen,
}));
vi.mock("@tauri-apps/api/window", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/window")>()),
  getCurrentWindow: () => ({ label: "w-2", listen: tauri.windowListen }),
}));

import { onMenu } from "./events";

const names = (listen: typeof tauri.globalListen) => listen.mock.calls.map(([name]) => name);

beforeEach(() => {
  tauri.globalListen.mockClear();
  tauri.windowListen.mockClear();
});

describe("cross-window events", () => {
  it("menu items are heard by this window only, never through a global listener", async () => {
    const seen: string[] = [];
    await onMenu((e) => seen.push(e.id));
    expect(names(tauri.windowListen)).toEqual(["menu"]);
    expect(names(tauri.globalListen)).not.toContain("menu");
    tauri.windowListen.mock.calls[0][1]({ payload: { id: "close-tab" } });
    expect(seen).toEqual(["close-tab"]);
  });
});
```

- [ ] **Step 2: Run them to verify they fail**

Run: `npx vitest run src/lib/menuActions.test.ts src/lib/windowEvents.test.ts`
Expected: FAIL. The `actions` imports aren't functions yet (`menuAction is not a function`), and `onMenu` isn't exported from `./events`.

- [ ] **Step 3: `onMenu` in `src/lib/events.ts`**

Add `import { getCurrentWindow } from "@tauri-apps/api/window";` and, at the end of the file:

```ts
// ---- Phase 3 ----

/** Mirrors `events::MenuEvent`: an app-menu item, by its `ActionId`. */
export interface MenuEvent {
  id: string;
}

/** This window's app-menu items. Window-scoped on purpose: the shell sends each item to the
 *  focused window only, and a global `listen` would also receive the other windows' items. */
export const onMenu = (fn: (e: MenuEvent) => void): Promise<UnlistenFn> =>
  getCurrentWindow().listen<MenuEvent>("menu", (e) => fn(e.payload));
```

- [ ] **Step 4: Menu routing in `src/lib/actions.ts`**

Phase 2 already imports `useEffect` and `useRef` from `react`. Add `import { onMenu } from "./events";` after that import. Keep `ActionId`, `KeyPress`, `keyToActionFor` and `keyToAction` as they are. Add the block below, and replace Phase 2's `useActions` (its doc comment included) with the one in it.

```ts
// ---- Phase 3: the app menu ----

/**
 * Whether the native app menu (src-tauri/src/menu.rs) owns an action's shortcut. The menu
 * carries every action; owning the shortcut means keydown never runs it, because in
 * WKWebView a key the page doesn't preventDefault goes on to the menu, and binding both
 * would run the action twice. A Record, so a new ActionId fails the typecheck until it's
 * decided here.
 */
const MENU_SHORTCUT: Record<ActionId, boolean> = {
  "new-tab": true,
  "new-window": true,
  "open-project": true,
  "reopen-tab": true,
  "close-tab": true,
  "close-window": true,
  "next-tab": true,
  "prev-tab": true,
  "toggle-sidebar": true,
  settings: true,
};

export const ownsShortcut = (action: ActionId): boolean => MENU_SHORTCUT[action];

/** The action a `menu { id }` event names, or null for any other id. */
export function menuAction(id: string): ActionId | null {
  return Object.hasOwn(MENU_SHORTCUT, id) ? (id as ActionId) : null;
}

/** The action a keydown still runs itself: never one whose shortcut the menu owns. */
export function keydownAction(e: KeyboardEvent): ActionId | null {
  const action = keyToAction(e);
  return action !== null && !ownsShortcut(action) ? action : null;
}

/**
 * Runs the handler for one menu item, once. Nothing runs while a modal dialog is open
 * (Settings): the native menu doesn't know about it, and ⌘W shouldn't close a tab behind it.
 */
export function dispatchMenu(id: string, handlers: Partial<Record<ActionId, () => void>>, modalOpen: boolean): ActionId | null {
  const action = menuAction(id);
  if (action === null || modalOpen) return null;
  handlers[action]?.();
  return action;
}

/**
 * Binds the actions: this window's app-menu items, plus any keydown the menu doesn't own.
 * Neither runs while a modal dialog (Settings) is open, and a key calls preventDefault()
 * only when it runs a handler.
 */
export function useActions(handlers: Partial<Record<ActionId, () => void>>): void {
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = keydownAction(e);
      const run = action ? latest.current[action] : undefined;
      if (!run || document.querySelector("dialog[open]")) return;
      e.preventDefault();
      run();
    };
    window.addEventListener("keydown", onKey);
    const menu = onMenu(({ id }) => {
      dispatchMenu(id, latest.current, document.querySelector("dialog[open]") !== null);
    });
    return () => {
      window.removeEventListener("keydown", onKey);
      void menu.then((unlisten) => unlisten());
    };
  }, []);
}
```

- [ ] **Step 5: Run the tests**

Run: `npx vitest run src/lib/menuActions.test.ts src/lib/windowEvents.test.ts`
Expected: PASS: 6 tests in `menuActions.test.ts` and 1 in `windowEvents.test.ts`.

- [ ] **Step 6: The `menu` event in `src-tauri/src/events.rs`**

Phase 1 declared neither. Add the constant next to `WORKSPACE_CHANGED`, and the struct after `WorkspaceChanged`:

```rust
pub const MENU: &str = "menu";
```

```rust
/// An app-menu item, sent to the focused window only (`emit_to` its label). `id` is an `ActionId`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEvent {
    pub id: String,
}
```

- [ ] **Step 7: Create `src-tauri/src/menu.rs`**

```rust
//! The app menu. Tauri's default menu binds ⌘W to Close Window, so the whole menu is
//! replaced. Our own items (`livebrainstorm_core::menu`) carry the frontend's action ids
//! and go to the focused window as `menu { id }`; that window's `useActions` runs them.
//! Predefined items stay native. The Edit menu keeps Undo, Redo, Cut, Copy, Paste and
//! Select All: without them the draft `<textarea>` loses those shortcuts on macOS.
//! Services, Hide, Hide Others and Show All (macOS), and Minimize and Zoom, are kept from
//! Tauri's default menu, so ⌘H and ⌘M keep working.

use livebrainstorm_core::menu::{self as spec, MenuItemSpec};
use tauri::menu::{AboutMetadata, Menu, MenuBuilder, MenuEvent, MenuItem, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::events;

/// The menu bar. `Builder::menu` calls this once at startup.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let info = app.package_info();
    let about =
        AboutMetadata { name: Some(info.name.clone()), version: Some(info.version.to_string()), ..Default::default() };
    let settings = item(app, &spec::SETTINGS)?;
    #[cfg(target_os = "macos")]
    let app_menu = SubmenuBuilder::new(app, &info.name)
        .about(Some(about))
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    #[cfg(not(target_os = "macos"))]
    let app_menu =
        SubmenuBuilder::new(app, &info.name).about(Some(about)).separator().item(&settings).separator().quit().build()?;
    let file = groups(app, SubmenuBuilder::new(app, "File"), &spec::FILE)?;
    let edit = SubmenuBuilder::new(app, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let window = groups(app, SubmenuBuilder::new(app, "Window").minimize().maximize().separator(), &spec::WINDOW)?;
    MenuBuilder::new(app).items(&[&app_menu, &file, &edit, &window]).build()
}

fn item(app: &AppHandle, spec: &MenuItemSpec) -> tauri::Result<MenuItem<Wry>> {
    MenuItemBuilder::with_id(spec.id, spec.label).accelerator(spec.accelerator).build(app)
}

/// Appends `groups` to `menu`, with a separator between groups.
fn groups<'m>(
    app: &'m AppHandle,
    mut menu: SubmenuBuilder<'m, Wry, AppHandle>,
    groups: &[&[MenuItemSpec]],
) -> tauri::Result<Submenu<Wry>> {
    for (k, group) in groups.iter().enumerate() {
        if k > 0 {
            menu = menu.separator();
        }
        for spec in group.iter() {
            menu = menu.item(&item(app, spec)?);
        }
    }
    menu.build()
}

/// Registered with `Builder::on_menu_event`. A custom item goes to the focused window
/// only (`emit_to` a label reaches that window's window-scoped listener, `onMenu`).
/// `get_focused_window` needs Tauri's `unstable` feature, so the focused window is found
/// by asking each one.
pub fn on_event(app: &AppHandle, event: MenuEvent) {
    let id: &str = event.id().as_ref();
    if !spec::is_action(id) {
        return; // a predefined item; the OS already ran it
    }
    let windows = app.webview_windows();
    let focused = windows.iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(label, _)| label.clone());
    // ⌘N still works with every window minimized; the other items need the window they act on.
    let target = focused.or_else(|| if id == "new-window" { windows.into_keys().min() } else { None });
    if let Some(label) = target {
        let _ = app.emit_to(label.as_str(), events::MENU, events::MenuEvent { id: id.to_owned() });
    }
}
```

- [ ] **Step 8: Register the menu in `src-tauri/src/lib.rs`**

Add `mod menu;` to the module list, after `mod events;` (the list stays alphabetical: `commands`, `events`, `menu`, `provider`, `secrets`, `state`, `windows`). After `.on_window_event(windows::on_window_event)`, add:

```rust
        .menu(menu::build)
        .on_menu_event(menu::on_event)
```

- [ ] **Step 9: Handlers for New Window and Close Window in `src/App.tsx`**

Phase 2 calls `useActions` once, in `src/App.tsx`. In that component:
- Add `import { getCurrentWindow } from "@tauri-apps/api/window";` after the `@tauri-apps/plugin-dialog` import. `api`, `errorMessage`, `projects` and `tabs` are already imported there.
- Add this state and helper right above the `useActions` call:

```ts
  const [windowError, setWindowError] = useState<string | null>(null);
  // ⌘⇧W. Saves the pending draft and instructions first; the shell then applies the close
  // rules (on_window_event): with other windows open, this window's tabs go to Recently closed.
  const closeWindow = () => {
    setWindowError(null);
    Promise.all([tabs.flush(), projects.flush()])
      .then(() => getCurrentWindow().close())
      .catch((e: unknown) => setWindowError(`Couldn't close the window: ${errorMessage(e)}`));
  };
```

Add these two entries to the handlers object passed to `useActions`. The shell's message already reads "Couldn't open a window: …", so it's shown as it is:

```ts
    "new-window": () => {
      setWindowError(null);
      api.openWindow().catch((e: unknown) => setWindowError(errorMessage(e)));
    },
    "close-window": closeWindow,
```

Show the error in Phase 2's banner under the top bar: change the `alert` line to

```ts
  const alert = windowError ?? openError ?? (root !== null ? (pj.summaryErrors[root] ?? null) : null);
```

- [ ] **Step 10: Branch "Custom Quit" (only if Task 1 recorded ⌘Q as outcome C, or a word typed just before ⌘Q was lost)**

Skip this step otherwise. It needs `AppState.quitting`, so apply Task 4's Step 8 first if Task 4 skipped it.

In `src-tauri/src/events.rs`, add:

```rust
/// Quit is about to exit: every window saves its pending edits now.
pub const FLUSH_REQUESTED: &str = "flush-requested";
```

In `src-tauri/src/menu.rs`:
- Add `use std::sync::atomic::Ordering;`, `use std::time::Duration;` and `use crate::state::AppState;`.
- In both `app_menu` builders, replace `.quit()` with `.item(&quit_item(app)?)`.
- Make this the first statement of `on_event`, after `let id = …;`:

```rust
    if id == QUIT {
        quit(app);
        return;
    }
```

Add these items:

```rust
/// Our own Quit (pre-implementation check 1): the predefined one closes windows first,
/// or loses the last edit. This one marks the app as quitting, asks every window to save,
/// and exits 400 ms later.
const QUIT: &str = "quit";

fn quit_item(app: &AppHandle) -> tauri::Result<MenuItem<Wry>> {
    MenuItemBuilder::with_id(QUIT, "Quit LiveBrainstorm").accelerator("CmdOrCtrl+Q").build(app)
}

fn quit(app: &AppHandle) {
    app.state::<AppState>().quitting.store(true, Ordering::SeqCst);
    let _ = app.emit(events::FLUSH_REQUESTED, ());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        app.exit(0);
    });
}
```

In `src/lib/events.ts`, add:

```ts
/** Quit is about to exit (pre-implementation check 1): save pending edits now. */
export const onFlushRequested = (fn: () => void): Promise<UnlistenFn> => listen("flush-requested", () => fn());
```

In `src/App.tsx`, add the effect below, after the title call. Add `import { onFlushRequested } from "./lib/events";`. The instructions saver lives in Phase 2's `projects` store, so `projects.flush()` saves it.

```ts
  // Quit waits 400 ms for these saves (pre-implementation check 1).
  useEffect(() => {
    const off = onFlushRequested(() => {
      void tabs.flush();
      void projects.flush();
    });
    return () => void off.then((unlisten) => unlisten());
  }, []);
```

- [ ] **Step 11: Check, lint and test**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm run typecheck && npm test`
Expected: the shell compiles with no clippy warnings, and every test passes. That includes `menu::tests::every_item_is_a_frontend_action_and_every_action_has_an_item` and the 7 new vitest tests.

If manual checklist item 9 (Task 8) later shows ⌃Tab never reaches the menu on macOS, apply this fallback. The webview then handles those keys and `preventDefault` stops them reaching the menu, so they still fire once:
- Set `"next-tab": false` and `"prev-tab": false` in `MENU_SHORTCUT`.
- In `menuActions.test.ts`, change "the menu owns every action's shortcut" to expect `false` for those two.

- [ ] **Step 12: Commit**

```bash
git add src-tauri/src src/lib/events.ts src/lib/actions.ts src/lib/menuActions.test.ts src/lib/windowEvents.test.ts src/App.tsx
git commit -m "feat: app menu with tab and window shortcuts, sent to the focused window

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 6: Moving a tab between windows

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`
- Modify: `src/lib/ipc.ts`, `src/lib/tabs.ts`, `src/lib/tabs.test.ts`, `src/components/TabRow.tsx`
- Create: `src/lib/moveTab.ts`, `src/lib/moveTab.test.ts`

**Interfaces:**
- Consumes:
  - From Task 3: `#[derive(Debug, Clone, Serialize)] #[serde(rename_all = "camelCase")] pub struct WindowSummary { pub id: String, pub active: Option<TabSummary>, pub tabs: usize }` (re-exported from `livebrainstorm_core::sessions`) and `SessionStore::windows(&self) -> Result<Vec<WindowSummary>>`.
  - From Phase 1, core:
    - `SessionStore::move_tab(&self, id: &str, to: Option<&str>) -> Result<(String, String)>`: `(from, to)`. `None` creates a window that shows the tab. An existing target gets the tab at the end, and makes it active only if it had none. A tab already in `to` returns `(from, from)` and changes nothing. An unknown target is `Precondition("This window was closed.")`, a closed tab `Precondition("This tab was closed.")`.
    - `SessionStore::activate(&self, window: &str, id: &str) -> Result<()>` and `SessionStore::close_window(&self, window: &str) -> Result<Vec<String>>`.
  - From Phase 1, shell (`src-tauri/src/commands.rs`): the private `fn check_id(id: &str) -> Result<()>`; the imports `use livebrainstorm_core::sessions::{valid_id, LegacyDraft, Recorder, Tab, WindowView};`, `use crate::events::{self, BriefState};` and `use crate::state::{cancel_task, replace_task, AppState, ProjectEntry};`; `pub fn cancel_task(slots, key)`, `pub fn AppState::evict_unused_projects(&self)` and `pub fn events::workspace_changed(app: &AppHandle, windows: Vec<String>)`.
  - From Task 4: `windows::build(app: &AppHandle, id: &str) -> Result<WebviewWindow>`, `use crate::windows;` and the `// ---------- windows ----------` section in `commands.rs`, and `openWindow` in `api`.
  - TS from Phase 1:
    - `TabSummary` (`{ id: string; root: string | null; title: string | null; firstLine: string; updatedAt: number; closedAt: number | null }`)
    - `tabTitle(t: { title: string | null; firstLine: string }): string`
    - `currentWindowId(): string`
  - From Phase 2, `src/lib/tabs.ts` (module internals, used by the code added to the same file):
    - `state: WorkspaceState`, `set(patch: Partial<WorkspaceState>)`, `saver` (the draft `createSaver<string>`), `drop(id: string)` (disposes and forgets a session), `show(s: TabSession | null)`, `sessionOf(tab: Tab): TabSession`, `order(): string[]` (visual order), `switchTo(tabId)`, `refresh()`, and the imports of `api`, `errorMessage` (`./ipc`) and `afterClose` (`./tabList`).
    - `close(tabId)` shows the next tab in visual order with `switchTo` *before* it closes the tab, so the backend's stored-order pick never flashes; `move` does the same.
    - `refresh()` contains these lines, which Step 5 extends:
      ```ts
      for (const id of [...sessions.keys()]) if (!view.tabs.some((t) => t.id === id)) drop(id);
      set({ view });
      // Nothing shown yet, or the shown tab left this window: show what the backend says is active.
      const shown = state.activeId;
      ```
    - The `tabs` object lists `load, switchTo, newTab, close, reopen, remove, rename, next, refresh, setContext, setDebounce` and Task 4's `flush`.
    - `src/lib/tabs.test.ts`: the hoisted `api` mock object, `loadStore(first)`, `summary`, `full`, `view` and `stored` (stored order a1, b1, a2; the sidebar shows a1, a2 | b1), and `beforeEach` with `vi.resetAllMocks()`.
  - From Phase 2, components: `export interface MenuItem { label: string; onSelect: () => void; disabled?: boolean }` and `Menu` (`ui.tsx`; no submenus, items keyed by label), and `TabRow` as described in "What Phases 1 and 2 leave behind".
- Produces:
  - Commands `move_tab(tab_id: String, to: Option<String>) -> Result<String>` (the target window id, async) and `list_windows() -> Result<Vec<WindowSummary>>`
  - TS `interface WindowSummary { id: string; active: TabSummary | null; tabs: number }`, `api.moveTab(tabId: string, to: string | null): Promise<string>` and `api.listWindows(): Promise<WindowSummary[]>`
  - `src/lib/moveTab.ts`:
    - `interface MoveDeps { flush: () => Promise<void>; move: (tabId: string, to: string | null) => Promise<string>; forget: (tabId: string) => void }`
    - `moveTabTo(tabId: string, to: string | null, deps: MoveDeps): Promise<string>`
    - `interface MoveTarget { id: string; name: string }`
    - `moveTargets(windows: WindowSummary[], self: string): MoveTarget[]`
    - `moveMenuItems(targets: MoveTarget[], move: (to: string | null) => void): { label: string; onSelect: () => void }[]`
  - `tabs.move(tabId: string, to: string | null): Promise<void>`
  - `tabs.ts` behaviour: a tab that newly appears in this window's list as the backend's active tab (one moved here) is shown.
  - `TabRow`'s context menu: Rename, *Move to new window*, one *Move to “\<window\>”* per other window, Close.

- [ ] **Step 1: Write the failing tests**

Create `src/lib/moveTab.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import type { TabSummary, WindowSummary } from "./ipc";
import { moveMenuItems, moveTabTo, moveTargets } from "./moveTab";

describe("moveTabTo", () => {
  it("saves the pending draft first, and drops the tab here only after the move", async () => {
    const order: string[] = [];
    const deps = {
      flush: vi.fn(async () => void order.push("flush")),
      move: vi.fn(async (_tabId: string, _to: string | null) => {
        order.push("move");
        return "w-3";
      }),
      forget: vi.fn((_tabId: string) => void order.push("forget")),
    };
    await expect(moveTabTo("t-1", null, deps)).resolves.toBe("w-3");
    expect(order).toEqual(["flush", "move", "forget"]);
    expect(deps.move).toHaveBeenCalledWith("t-1", null);
    expect(deps.forget).toHaveBeenCalledWith("t-1");
  });

  it("waits for a slow save before moving", async () => {
    let saved!: () => void;
    const flush = () => new Promise<void>((resolve) => (saved = resolve));
    const move = vi.fn(async () => "w-2");
    const moving = moveTabTo("t-1", "w-2", { flush, move, forget: () => {} });
    await Promise.resolve();
    expect(move).not.toHaveBeenCalled();
    saved();
    await moving;
    expect(move).toHaveBeenCalledTimes(1);
  });

  it("a failed move keeps the tab in this window", async () => {
    const forget = vi.fn();
    const move = async (): Promise<string> => {
      throw "This tab was closed.";
    };
    await expect(moveTabTo("t-1", "w-2", { flush: async () => {}, move, forget })).rejects.toBe("This tab was closed.");
    expect(forget).not.toHaveBeenCalled();
  });
});

describe("moveTargets", () => {
  const tab = (id: string, title: string | null, firstLine: string): TabSummary => ({
    id,
    root: null,
    title,
    firstLine,
    updatedAt: 0,
    closedAt: null,
  });
  const win = (id: string, active: TabSummary | null, tabs = active ? 1 : 0): WindowSummary => ({ id, active, tabs });

  it("lists the other windows, named by their active tab", () => {
    const windows = [win("w-1", tab("t-1", "Sidebar tabs", "")), win("w-2", tab("t-2", null, "Checkout retry")), win("w-3", null)];
    expect(moveTargets(windows, "w-1")).toEqual([
      { id: "w-2", name: "Checkout retry" },
      { id: "w-3", name: "Empty window" },
    ]);
  });

  it("numbers repeated names, and offers nothing with one window", () => {
    const windows = [win("w-1", null), win("w-4", tab("t-4", null, "")), win("w-6", tab("t-6", null, "")), win("w-7", null)];
    expect(moveTargets(windows, "w-1").map((t) => t.name)).toEqual(["Untitled", "Untitled (2)", "Empty window"]);
    expect(moveTargets([win("w-1", null)], "w-1")).toEqual([]);
  });
});

describe("moveMenuItems", () => {
  it("offers a new window first, then each other window by name", () => {
    const moves: (string | null)[] = [];
    const items = moveMenuItems(
      [
        { id: "w-2", name: "Checkout retry" },
        { id: "w-3", name: "Empty window" },
      ],
      (to) => void moves.push(to),
    );
    expect(items.map((i) => i.label)).toEqual(["Move to new window", "Move to “Checkout retry”", "Move to “Empty window”"]);
    items.forEach((i) => i.onSelect());
    expect(moves).toEqual([null, "w-2", "w-3"]);
  });
});
```

In `src/lib/tabs.test.ts` (Phase 2), add `moveTab: vi.fn(),` to the hoisted `api` object, after `importLegacyDrafts: vi.fn(),`. Then add these three tests at the end of the `describe("tabs store", …)` block:

```ts
  it("moving the shown tab saves its draft first, then shows the next tab in the sidebar's order", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    api.moveTab.mockResolvedValue("w-2");
    api.getTab.mockResolvedValue(full("a2", "/a"));
    api.listWorkspace.mockResolvedValue(view(stored.slice(1), full("b1", "/b")));
    await tabs.move("a1", null);
    expect(api.saveDraft).toHaveBeenCalledWith("a1", "typed in a1");
    expect(api.saveDraft.mock.invocationCallOrder[0]).toBeLessThan(api.moveTab.mock.invocationCallOrder[0]);
    expect(api.moveTab).toHaveBeenCalledWith("a1", null);
    expect(sessionFor("a1")).toBeUndefined();
    expect(api.getTab).toHaveBeenCalledWith("a2"); // the backend's pick, b1, follows the stored order
    expect(workspaceState().activeId).toBe("a2");
  });

  it("a failed move keeps the tab here and says why", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    api.getTab.mockResolvedValue(full("a2", "/a"));
    api.moveTab.mockRejectedValue("This window was closed.");
    await tabs.move("a1", "w-9");
    expect(sessionFor("a1")).toBeDefined();
    expect(workspaceState().activeId).toBe("a1");
    expect(workspaceState().error).toBe("This window was closed.");
  });

  it("a tab moved here from another window is shown", async () => {
    const { tabs, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    api.listWorkspace.mockResolvedValue(view([...stored, summary("m1", "/m")], full("m1", "/m")));
    await tabs.refresh();
    expect(workspaceState().activeId).toBe("m1");
    // The backend's active tab alone is never followed: only a tab that just arrived.
    api.listWorkspace.mockResolvedValue(view([...stored, summary("m1", "/m")], full("a2", "/a")));
    await tabs.refresh();
    expect(workspaceState().activeId).toBe("m1");
  });
```

- [ ] **Step 2: Run them to verify they fail**

Run: `npx vitest run src/lib/moveTab.test.ts src/lib/tabs.test.ts`
Expected: FAIL. `moveTab.test.ts` fails with `Failed to resolve import "./moveTab"`. In `tabs.test.ts`, the two move tests fail with `tabs.move is not a function`, and "a tab moved here from another window is shown" fails because `activeId` stays `"a1"`. Phase 2's tests still pass.

- [ ] **Step 3: Types and wrappers in `src/lib/ipc.ts`, and `src/lib/moveTab.ts`**

In `src/lib/ipc.ts`, add after the `TabSummary` interface:

```ts
/** Mirrors `sessions::WindowSummary`: one window, named in menus by its active tab. */
export interface WindowSummary {
  id: string;
  active: TabSummary | null;
  tabs: number;
}
```

and to `api`, after `openWindow`:

```ts
  moveTab: (tabId: string, to: string | null) => invoke<string>("move_tab", { tabId, to }),
  listWindows: () => invoke<WindowSummary[]>("list_windows"),
```

Create `src/lib/moveTab.ts`:

```ts
// Moving a tab to another window. The order matters: the pending draft is saved first,
// so the receiving window loads the last edit; then the backend moves the tab (and stops
// its run, whose Channel belongs to this window); only then does this window drop it.
import type { WindowSummary } from "./ipc";
import { tabTitle } from "./tabTitle";

export interface MoveDeps {
  /** Saves the pending draft edit now. */
  flush: () => Promise<void>;
  /** `api.moveTab`: resolves to the window the tab is now in. */
  move: (tabId: string, to: string | null) => Promise<string>;
  /** Disposes this window's TabSession for the tab and forgets it. */
  forget: (tabId: string) => void;
}

/** `to` null moves the tab to a new window. Rejects, keeping the tab here, if the move fails. */
export async function moveTabTo(tabId: string, to: string | null, deps: MoveDeps): Promise<string> {
  await deps.flush();
  const target = await deps.move(tabId, to);
  deps.forget(tabId);
  return target;
}

export interface MoveTarget {
  id: string;
  name: string;
}

/** The other windows, named by their active tab ("Empty window" without one). A repeated
 *  name gets " (2)", " (3)"… in workspace order, so every entry can be told apart. */
export function moveTargets(windows: WindowSummary[], self: string): MoveTarget[] {
  const seen = new Map<string, number>();
  return windows
    .filter((w) => w.id !== self)
    .map((w) => {
      const name = w.active ? tabTitle(w.active) : "Empty window";
      const n = (seen.get(name) ?? 0) + 1;
      seen.set(name, n);
      return { id: w.id, name: n === 1 ? name : `${name} (${n})` };
    });
}

/** The tab context menu's move items: "Move to new window", then "Move to “<window>”" for each
 *  other window. Phase 2's `Menu` has no submenus and keys its items by label, which the
 *  numbering in `moveTargets` keeps unique. */
export function moveMenuItems(targets: MoveTarget[], move: (to: string | null) => void): { label: string; onSelect: () => void }[] {
  return [
    { label: "Move to new window", onSelect: () => move(null) },
    ...targets.map((t) => ({ label: `Move to “${t.name}”`, onSelect: () => move(t.id) })),
  ];
}
```

Run: `npx vitest run src/lib/moveTab.test.ts`
Expected: PASS (6 tests).

- [ ] **Step 4: `tabs.move`, and showing a tab that arrives, in `src/lib/tabs.ts`**

Add `import { moveTabTo } from "./moveTab";` after the `./ipc` import.

In `refresh()`, replace these two lines:

```ts
  set({ view });
  // Nothing shown yet, or the shown tab left this window: show what the backend says is active.
```

with:

```ts
  const before = state.view?.tabs ?? null;
  set({ view });
  // A tab moved here from another window arrives as this window's active tab (move_tab
  // activates it there): show it. Only a tab that just arrived; the backend's active tab
  // alone is never followed, because this window's own switches set it.
  const arrived = view.active;
  if (before !== null && arrived !== null && !before.some((t) => t.id === arrived.id)) {
    if (arrived.id !== state.activeId) show(sessionOf(arrived));
    return;
  }
  // Nothing shown yet, or the shown tab left this window: show what the backend says is active.
```

Add this function after `close`:

```ts
/**
 * Moves a tab to window `to`, or to a new window (null). The pending draft is saved first,
 * and a failed move leaves the tab here. Moving the shown tab shows the next one in the
 * sidebar's order, as closing does. The shell stops the tab's run; the other window shows it.
 */
async function move(tabId: string, to: string | null): Promise<void> {
  const wasShown = tabId === state.activeId;
  const next = wasShown ? afterClose(order(), tabId) : null;
  set({ error: null });
  // As closing does: show the next tab first, so the backend's stored-order pick never flashes.
  if (next) await switchTo(next);
  try {
    await moveTabTo(tabId, to, {
      flush: () => saver.flush(),
      move: (id, target) => api.moveTab(id, target),
      forget: drop,
    });
  } catch (e) {
    if (next) await switchTo(tabId); // the tab stays here, shown again
    set({ error: errorMessage(e) });
    return;
  }
  if (wasShown && !next) show(null); // the window stays open and empty
  await refresh().catch((e) => set({ error: errorMessage(e) }));
}
```

In the exported `tabs` object, add `move,` after `close,`.

Run: `npx vitest run src/lib/moveTab.test.ts src/lib/tabs.test.ts`
Expected: PASS: 6 tests in `moveTab.test.ts`, and every `tabs.test.ts` test, Phase 2's and the three new ones.

- [ ] **Step 5: The `move_tab` and `list_windows` commands**

In `src-tauri/src/commands.rs`, add `WindowSummary` to Phase 1's sessions import, which becomes:

```rust
use livebrainstorm_core::sessions::{valid_id, LegacyDraft, Recorder, Tab, WindowSummary, WindowView};
```

(`cancel_task`, `events` and `windows` are imported already.) Add to the `// ---------- windows ----------` section, after `open_window`:

```rust
/// Moves a tab to another window, or to a new one (`to: None`), and makes it active there.
/// Its running analysis stops: the run's Channel belongs to the old window, and the new
/// window re-runs at the next pause. Returns the target window's id. `async`: it may build
/// a window, which deadlocks in a synchronous command on Windows.
#[tauri::command]
pub async fn move_tab(app: AppHandle, tab_id: String, to: Option<String>) -> Result<String> {
    check_id(&tab_id)?;
    if let Some(window) = &to {
        check_id(window)?;
    }
    let state = app.state::<AppState>();
    let (from, to) = state.sessions.move_tab(&tab_id, to.as_deref())?;
    if from == to {
        return Ok(to); // already there: nothing moved, and its run goes on
    }
    cancel_task(&state.analyses, &tab_id); // drops the claude children too (kill_on_drop)
    if let Err(e) = state.sessions.activate(&to, &tab_id) {
        eprintln!("LiveBrainstorm: moved tab {tab_id} to {to} but couldn't show it there: {e}");
    }
    match app.get_webview_window(&to) {
        Some(window) => {
            let _ = window.set_focus();
        }
        None => {
            if let Err(e) = windows::build(&app, &to) {
                // Put it back rather than strand it in a window that isn't on screen.
                let _ = state.sessions.move_tab(&tab_id, Some(from.as_str()));
                let _ = state.sessions.activate(&from, &tab_id);
                let _ = state.sessions.close_window(&to);
                events::workspace_changed(&app, vec![from]);
                return Err(e);
            }
        }
    }
    state.evict_unused_projects();
    events::workspace_changed(&app, vec![from, to.clone()]);
    Ok(to)
}

/// Every window, named by its active tab, for the *Move to* items.
#[tauri::command]
pub fn list_windows(state: State<'_, AppState>) -> Result<Vec<WindowSummary>> {
    state.sessions.windows()
}
```

In `src-tauri/src/lib.rs`, add `commands::move_tab,` and `commands::list_windows,` to `generate_handler![…]` after `commands::open_window,`.

- [ ] **Step 6: The *Move to* items in `src/components/TabRow.tsx`**

Phase 2's `Menu` takes a flat `items` list, so the move items join `TabRow`'s list between Rename and Close (the spec's order: Rename, Move to new window, Move to ▸, Close). The other windows are fetched each time the menu opens.

- Change `import type { TabSummary } from "../lib/ipc";` to:
  ```ts
  import { api, type TabSummary } from "../lib/ipc";
  import { moveMenuItems, moveTargets, type MoveTarget } from "../lib/moveTab";
  ```
- Add `import { currentWindowId } from "../lib/windowId";` after the `../lib/useTabSession` import.
- After `const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);`, add:
  ```ts
  const [targets, setTargets] = useState<MoveTarget[]>([]);
  ```
- In `openMenu`, after its `setMenu(…)` line, add:
  ```ts
    // The other windows, for "Move to “…”". Without them, "Move to new window" still works.
    setTargets([]);
    api
      .listWindows()
      .then((list) => setTargets(moveTargets(list, currentWindowId())))
      .catch((err: unknown) => console.error("list_windows failed", err));
  ```
- Replace the `items` prop of `<Menu>` with:
  ```tsx
          items={[
            { label: "Rename", onSelect: startRename },
            ...moveMenuItems(targets, (to) => void tabs.move(tab.id, to)),
            { label: "Close", onSelect: () => void tabs.close(tab.id) },
          ]}
  ```

- [ ] **Step 7: Check, lint and test**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm run typecheck && npm test`
Expected: the shell compiles with no clippy warnings, there are no type errors, and every test passes.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src src/lib/ipc.ts src/lib/moveTab.ts src/lib/moveTab.test.ts src/lib/tabs.ts src/lib/tabs.test.ts src/components/TabRow.tsx
git commit -m "feat: move a tab to a new window or another one

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 7: Settings and instructions shared across windows

**Files:**
- Modify: `src-tauri/src/events.rs`, `src-tauri/src/commands.rs`
- Modify: `src/lib/events.ts`, `src/lib/windowEvents.test.ts`, `src/App.tsx`
- Create: `src/lib/crossWindow.ts`, `src/lib/crossWindow.test.ts`

**Interfaces:**
- Consumes:
  - Today's commands in `src-tauri/src/commands.rs` (Phases 1 and 2 leave them as they are):
    - `save_settings(app: AppHandle, settings: Settings) -> Result<()>` (async; it writes the settings with `*state.settings.write().expect("settings lock") = settings;`, then may stop the local server and calls `warm_up(app).await`)
    - `save_project_instructions(state: State<'_, AppState>, root: String, instructions: ProjectInstructions) -> Result<()>`
    - `save_templates(state: State<'_, AppState>, templates: Vec<Template>) -> Result<()>`
    - `restore_starter_templates(state: State<'_, AppState>) -> Result<Vec<Template>>`
  - From Phase 1: `src-tauri/src/events.rs` imports `serde::Serialize` and `tauri::{AppHandle, Emitter}`; `commands.rs` imports `use tauri::{AppHandle, Manager, State};` and `use crate::events::{self, BriefState};`.
  - `currentWindowId(): string` (Phase 1).
  - `api.getSettings(): Promise<Settings>`.
  - From Phase 2, `src/lib/projects.ts`: `projects.reloadInstructions(root: string | null): Promise<void>` (saves this window's pending instructions edit, refetches `get_instructions(root)`, stores the project's instructions and the templates, and pushes the context to that root's tabs; `""` and `null` both mean no project), `projects.flush(): Promise<void>`, and `projectsState(): ProjectsState`, whose `instructions: Record<string, ProjectInstructions>` holds every root this window has loaded (`""` is no project).
  - From Phase 2, `src/App.tsx`: `const [settings, setSettings] = useState<Settings | null>(null);`, `tabs.setDebounce(ms)`, and the import `import { IDLE_BRIEF, projects, useProjects } from "./lib/projects";`.
- Produces:
  - Rust `events::SETTINGS_CHANGED: &str = "settings-changed"` and `events::INSTRUCTIONS_CHANGED: &str = "instructions-changed"`
  - Rust `#[derive(Clone, Serialize)] events::InstructionsChanged { pub root: String, pub source: String }`. `source` is an extra field the contract allows: the label of the window that saved.
  - Rust helpers `events::settings_changed(app: &AppHandle)` and `events::instructions_changed(from: &Webview, root: String)`.
  - TS in `events.ts`:
    - `interface InstructionsChangedEvent { root: string; source: string }`
    - `onSettingsChanged(fn: () => void): Promise<UnlistenFn>`
    - `onInstructionsChanged(fn: (e: InstructionsChangedEvent) => void): Promise<UnlistenFn>`
  - `src/lib/crossWindow.ts`:
    - `interface CrossWindowDeps { windowId: string; cachedRoots: () => Iterable<string>; flushInstructions: () => Promise<void>; refetchInstructions: (root: string) => Promise<void>; refetchSettings: () => Promise<void> }`
    - `rootsToRefetch(e: InstructionsChangedEvent, self: string, cached: Iterable<string>): string[]`
    - `refreshInstructions(e: InstructionsChangedEvent, d: CrossWindowDeps): Promise<void>`
    - `useCrossWindowRefresh(deps: CrossWindowDeps): void`

- [ ] **Step 1: Write the failing tests**

Create `src/lib/crossWindow.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { refreshInstructions, rootsToRefetch, type CrossWindowDeps } from "./crossWindow";

describe("rootsToRefetch", () => {
  const cached = ["/a", "/b", ""];

  it("never refetches this window's own save", () => {
    expect(rootsToRefetch({ root: "/a", source: "w-1" }, "w-1", cached)).toEqual([]);
    expect(rootsToRefetch({ root: "", source: "w-1" }, "w-1", cached)).toEqual([]);
  });

  it("refetches the saved root only if this window has it", () => {
    expect(rootsToRefetch({ root: "/a", source: "w-2" }, "w-1", cached)).toEqual(["/a"]);
    expect(rootsToRefetch({ root: "/c", source: "w-2" }, "w-1", cached)).toEqual([]);
  });

  it("a templates change refetches every cached root", () => {
    expect(rootsToRefetch({ root: "", source: "w-2" }, "w-1", cached)).toEqual(["/a", "/b", ""]);
  });
});

describe("refreshInstructions", () => {
  const deps = (log: string[], cached: string[]): CrossWindowDeps => ({
    windowId: "w-1",
    cachedRoots: () => cached,
    flushInstructions: async () => void log.push("flush"),
    refetchInstructions: async (root) => void log.push(`refetch ${root}`),
    refetchSettings: async () => void log.push("settings"),
  });

  it("saves this window's pending edit before refetching", async () => {
    const log: string[] = [];
    await refreshInstructions({ root: "/a", source: "w-2" }, deps(log, ["/a", "/b"]));
    expect(log).toEqual(["flush", "refetch /a"]);
  });

  it("does nothing for its own save, or a root it doesn't show", async () => {
    const log: string[] = [];
    await refreshInstructions({ root: "/a", source: "w-1" }, deps(log, ["/a"]));
    await refreshInstructions({ root: "/z", source: "w-2" }, deps(log, ["/a"]));
    expect(log).toEqual([]);
  });
});
```

In `src/lib/windowEvents.test.ts`, change the import to `import { onInstructionsChanged, onMenu, onSettingsChanged } from "./events";` and add this test inside the `describe`:

```ts
  it("settings and instructions changes reach every window", async () => {
    const settings = vi.fn();
    const instructions = vi.fn();
    await onSettingsChanged(settings);
    await onInstructionsChanged(instructions);
    expect(names(tauri.globalListen)).toEqual(["settings-changed", "instructions-changed"]);
    tauri.globalListen.mock.calls[0][1]({ payload: null });
    tauri.globalListen.mock.calls[1][1]({ payload: { root: "/p", source: "w-1" } });
    expect(settings).toHaveBeenCalledTimes(1);
    expect(instructions).toHaveBeenCalledWith({ root: "/p", source: "w-1" });
  });
```

- [ ] **Step 2: Run them to verify they fail**

Run: `npx vitest run src/lib/crossWindow.test.ts src/lib/windowEvents.test.ts`
Expected: FAIL. `./crossWindow` doesn't resolve, and `onSettingsChanged` isn't a function.

- [ ] **Step 3: The listeners in `src/lib/events.ts`**

Add to the `// ---- Phase 3 ----` part:

```ts
/** Mirrors `events::InstructionsChanged`. `root` "" is the templates (or the no-project
 *  instructions); `source` is the label of the window that saved. */
export interface InstructionsChangedEvent {
  root: string;
  source: string;
}

/** After any window saves settings. Global: every window refetches. */
export const onSettingsChanged = (fn: () => void): Promise<UnlistenFn> => listen("settings-changed", () => fn());

/** After any window saves instructions or templates. Global: each window decides what to refetch. */
export const onInstructionsChanged = (fn: (e: InstructionsChangedEvent) => void): Promise<UnlistenFn> =>
  listen<InstructionsChangedEvent>("instructions-changed", (e) => fn(e.payload));
```

- [ ] **Step 4: Create `src/lib/crossWindow.ts`**

```ts
// Settings, instructions and templates are shared by every window, and the last save
// wins. Another window's save arrives as an event, and this window refetches what it shows.
import { useEffect, useRef } from "react";
import { onInstructionsChanged, onSettingsChanged, type InstructionsChangedEvent } from "./events";

export interface CrossWindowDeps {
  /** This window's id (`currentWindowId()`), to recognise its own saves. */
  windowId: string;
  /** The roots whose instructions this window has loaded ("" is the no-project instructions). */
  cachedRoots: () => Iterable<string>;
  /** Saves this window's pending instructions edit now. */
  flushInstructions: () => Promise<void>;
  /** Fetches one root's `InstructionsView` (templates included) into this window's cache. */
  refetchInstructions: (root: string) => Promise<void>;
  refetchSettings: () => Promise<void>;
}

/**
 * The cached roots to refetch after `e`. A window never refetches its own save: the copy
 * it would get back can be older than what was typed since. Root "" (templates, or the
 * no-project instructions) refetches every cached root, because each view carries the templates.
 */
export function rootsToRefetch(e: InstructionsChangedEvent, self: string, cached: Iterable<string>): string[] {
  if (e.source === self) return [];
  const roots = [...cached];
  return e.root === "" ? roots : roots.filter((root) => root === e.root);
}

/** Saves this window's own pending edit before refetching, so that edit can't land half a
 *  second later on top of the refetched copy, and the last save really is the last. */
export async function refreshInstructions(e: InstructionsChangedEvent, d: CrossWindowDeps): Promise<void> {
  const roots = rootsToRefetch(e, d.windowId, d.cachedRoots());
  if (roots.length === 0) return;
  await d.flushInstructions();
  await Promise.all(roots.map((root) => d.refetchInstructions(root)));
}

/** Keeps this window's settings and instructions in step with saves made in other windows. */
export function useCrossWindowRefresh(deps: CrossWindowDeps): void {
  const latest = useRef(deps);
  useEffect(() => {
    latest.current = deps;
  });
  useEffect(() => {
    // Neither command can fail; a dropped refresh is caught up by the next save's event.
    const report = (e: unknown) => console.error("cross-window refresh failed", e);
    const subscriptions = [
      onSettingsChanged(() => void latest.current.refetchSettings().catch(report)),
      onInstructionsChanged((e) => void refreshInstructions(e, latest.current).catch(report)),
    ];
    return () => subscriptions.forEach((s) => void s.then((unlisten) => unlisten()));
  }, []);
}
```

- [ ] **Step 5: Run the tests**

Run: `npx vitest run src/lib/crossWindow.test.ts src/lib/windowEvents.test.ts`
Expected: PASS: 5 tests in `crossWindow.test.ts` and 2 in `windowEvents.test.ts`.

- [ ] **Step 6: The events in `src-tauri/src/events.rs`**

Phase 1 declared none of these. Change the `use tauri::{AppHandle, Emitter};` line to `use tauri::{AppHandle, Emitter, Webview};`, add the two constants next to `WORKSPACE_CHANGED`, and add the struct and the helpers at the end of the file. The helpers follow Phase 1's `brief_status` and `workspace_changed`:

```rust
pub const SETTINGS_CHANGED: &str = "settings-changed";
pub const INSTRUCTIONS_CHANGED: &str = "instructions-changed";
```

```rust
/// After saving instructions (`root` is that project, "" for no project) or templates
/// (`root` ""). `source` is the saving window's label: it skips its own event.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionsChanged {
    pub root: String,
    pub source: String,
}

/// Every window calls `get_settings`. No payload.
pub fn settings_changed(app: &AppHandle) {
    let _ = app.emit(SETTINGS_CHANGED, ()); // a window may be gone
}

/// Other windows refetch; `source` lets the saving window skip its own event, whose copy
/// can be older than what was typed since. `Webview::emit` reaches every window, like `app.emit`.
pub fn instructions_changed(from: &Webview, root: String) {
    let _ = from.emit(INSTRUCTIONS_CHANGED, InstructionsChanged { root, source: from.label().to_owned() });
}
```

- [ ] **Step 7: Emit them from `src-tauri/src/commands.rs`**

Change Phase 1's `use tauri::{AppHandle, Manager, State};` to `use tauri::{AppHandle, Manager, State, Webview};`. (`events` is imported already, as a module.)

In `save_settings`, right after `*state.settings.write().expect("settings lock") = settings;`, add the line below. The lock guard is a temporary of the line above it, so nothing is held across the `.await`s that follow.

```rust
    events::settings_changed(&app); // other windows call get_settings
```

Replace the three instruction commands with:

```rust
#[tauri::command]
pub fn save_project_instructions(
    state: State<'_, AppState>,
    webview: Webview,
    root: String,
    instructions: ProjectInstructions,
) -> Result<()> {
    state.instructions.save_project(&root, instructions)?;
    events::instructions_changed(&webview, root);
    Ok(())
}

#[tauri::command]
pub fn save_templates(state: State<'_, AppState>, webview: Webview, templates: Vec<Template>) -> Result<()> {
    state.instructions.save_templates(templates)?;
    events::instructions_changed(&webview, String::new());
    Ok(())
}

#[tauri::command]
pub fn restore_starter_templates(state: State<'_, AppState>, webview: Webview) -> Result<Vec<Template>> {
    let templates = state.instructions.restore_starters()?;
    events::instructions_changed(&webview, String::new());
    Ok(templates)
}
```

Tauri injects `webview: Webview`, so it isn't an IPC argument and the `api` wrappers don't change.

- [ ] **Step 8: Wire the refresh into `src/App.tsx`**

The per-root instructions and their saver live in Phase 2's `projects` store, and `projects.reloadInstructions` already saves a pending edit before it refetches. Settings live in `App.tsx`. In `src/App.tsx`:
- Change `import { IDLE_BRIEF, projects, useProjects } from "./lib/projects";` to `import { IDLE_BRIEF, projects, projectsState, useProjects } from "./lib/projects";`.
- Add `import { useCrossWindowRefresh } from "./lib/crossWindow";` after the `./lib/actions` import. `currentWindowId` is imported already.
- Add this call right after the `useWindowTitle(…)` line (Task 4):

```tsx
  // Another window saved settings or instructions: refetch what this window shows.
  useCrossWindowRefresh({
    windowId: currentWindowId(),
    cachedRoots: () => Object.keys(projectsState().instructions),
    flushInstructions: () => projects.flush(),
    refetchInstructions: (root) => projects.reloadInstructions(root),
    refetchSettings: async () => {
      const s = await api.getSettings();
      setSettings(s);
      tabs.setDebounce(s.debounceMs);
    },
  });
```

- [ ] **Step 9: Check, lint and test**

Run: `cargo check -p livebrainstorm && cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm run typecheck && npm test`
Expected: the shell compiles with no clippy warnings, there are no type errors, and every test passes.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src src/lib/events.ts src/lib/windowEvents.test.ts src/lib/crossWindow.ts src/lib/crossWindow.test.ts src/App.tsx
git commit -m "feat: settings and instructions saved in one window refresh the others

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 8: Docs, and the manual checklist for windows

**Files:**
- Modify: `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes: the names every earlier task produced, and Task 1's recorded results and branches.
- Produces: documentation only.

- [ ] **Step 1: AGENTS.md: UI and styling**

Phase 2 rewrote parts of AGENTS.md, so find each passage by its text, not by line number.

In "### UI and styling", replace Phase 2's `- **Shortcuts**` bullet (the one that begins "- **Shortcuts** go through `useActions`") with:

```markdown
- **Shortcuts** belong to the app menu (`src-tauri/src/menu.rs`, see "Windows and the app menu"): ⌘T new tab on the shown tab's project, ⌘N new window, ⌘O open a project, ⌘⇧T reopen the last closed tab, ⌘W close the tab, ⌘⇧W close the window, ⌃Tab / ⌃⇧Tab next and previous tab, ⌘\\ sidebar, ⌘, settings. Ctrl replaces ⌘ on Windows and Linux. Each item reaches the focused window as a `menu` event, and `useActions` (`src/lib/actions.ts`) runs the handler `App.tsx` passes for it, except while a modal `<dialog>` is open. Keydown runs no action whose shortcut the menu owns (`ownsShortcut`), so one keystroke is one action; on macOS Ctrl+letter keeps its text-editing meaning (⌃T transposes).
```

Then add after the `- **Settings**` bullet:

```markdown
- **Windows:** every window runs the same app (`index.html`) and learns its workspace id from its label (`currentWindowId()` in `src/lib/windowId.ts`). Its title follows the shown tab (`windowTitle` in `src/lib/tabList.ts`) through `getCurrentWindow().setTitle` (`useWindowTitle` in `src/lib/windowTitle.ts`), because Tauri doesn't copy `document.title` to the window. A tab's context menu offers *Move to new window* and one *Move to “<window>”* per other window (`moveMenuItems` in `src/lib/moveTab.ts`; `Menu` has no submenus), each named by its active tab.
```

- [ ] **Step 2: AGENTS.md: a section on windows and the app menu**

Insert this subsection after "### Live analysis flow", before "### Project understanding: two layers". Replace the `<quit>` sentence with the line that matches Task 1's recorded result:
- **No branch:** "⌘Q exits without any close request (pre-implementation check 1), so it never touches the workspace."
- **Quit guard:** "Quitting fires close requests after `ExitRequested`, which sets `AppState.quitting`; the close handler then does nothing (pre-implementation check 1)."
- **Custom Quit:** "Quit is our own menu item: it sets `AppState.quitting`, emits `flush-requested` so every window saves its pending edits, and exits 400 ms later (pre-implementation check 1)."

If Task 5's ⌃Tab fallback was applied, also replace "The menu owns every action's shortcut, so `useActions` binds no keydown for them:" with "The menu owns every action's shortcut except ⌃Tab and ⌃⇧Tab, which never reach it from the webview on macOS, so `useActions` handles only those two on keydown (with `preventDefault`) and binds no keydown for the rest:".

```markdown
### Windows and the app menu

- **Labels are workspace ids.** `tauri.conf.json` declares no windows (`"windows": []`). `setup` builds one per `SessionStore::windows_to_open()` with `windows::build` (`src-tauri/src/windows.rs`), labelled `w-<n>`. `tauri-plugin-window-state` (which keys on labels) restores each one's position and size, and the capability glob `"windows": ["w-*"]` gives it IPC. `open_window` and `move_tab` to a new window build windows the same way.
- **Build windows only in `setup` or an `async` command.** On Windows, `WebviewWindowBuilder::build` deadlocks in a synchronous command or an event handler, so the menu handler never builds one: ⌘N goes to the frontend, which calls `open_window`.
- **Closing** (`windows::on_window_event`, `CloseRequested`): with another window open, `SessionStore::close_window_unless_last` sends this window's tabs to *Recently closed*; the shell cancels their runs, evicts unused projects and emits `workspace-changed`. The last window is left untouched, which is the same as quitting. The decision reads the workspace, not Tauri's window list, which still lists a window that is closing. <quit>
- **Moving a tab** (`move_tab`): the frontend saves the pending draft first (`moveTabTo` in `src/lib/moveTab.ts`, through `tabs.move`), then the shell moves the tab, cancels its run (the run's Channel belongs to the old window), makes it active in the target, focuses that window, and emits `workspace-changed` for both windows. If a new window can't be built, the tab moves back. The window it left shows the next tab in sidebar order, as closing does; the target window shows the tab because it arrived in its list as the active tab (`refresh` in `src/lib/tabs.ts`, which otherwise never follows the backend's active tab).
- **The app menu** (`src-tauri/src/menu.rs`) replaces Tauri's default, which binds ⌘W to Close Window. Its own items are data in `livebrainstorm_core::menu`, and their ids are the frontend's `ActionId`s. A chosen item goes to the focused window only, as `menu { id }`, and `useActions` dispatches it, except while a modal `<dialog>` is open. The menu owns every action's shortcut, so `useActions` binds no keydown for them: in WKWebView a key the page doesn't `preventDefault` goes on to the menu, and binding both would run the action twice. The Edit menu keeps the predefined Undo, Redo, Cut, Copy, Paste and Select All; without them the draft `<textarea>` loses those shortcuts on macOS.
- **Shared settings and instructions:** `save_settings` emits `settings-changed`; instruction and template saves emit `instructions-changed { root, source }` (`root` "" for templates). Other windows refetch (`useCrossWindowRefresh` in `src/lib/crossWindow.ts`, through `projects.reloadInstructions`), saving their own pending edit first. The saving window skips its own event, because the copy it would get back can be older than what was typed since. The last save wins.
```

- [ ] **Step 3: AGENTS.md: persistence, contracts and decisions**

In "### Persistence", add:

```markdown
- Window positions and sizes: `<app_config>/.window-state.json`, written by `tauri-plugin-window-state` on exit and keyed by window label. Entries for closed windows stay, which is harmless because window ids are never reused.
```

In "### Project understanding: two layers", in Phase 1's `- **Loaded projects.**` bullet, replace "after `close_tab` and `delete_tab`" with "after `close_tab`, `delete_tab`, `move_tab` and closing a window".

In "## Cross-language contracts", add after Phase 1's `- **Brief and workspace events:**` line:

```markdown
- **Window ids ↔ the capability glob:** windows are labelled with their workspace ids (`w-<n>`), and `src-tauri/capabilities/default.json` grants IPC to `"windows": ["w-*"]`. A window with any other label gets no plugin commands: no events, no folder picker, no title.
- **Menu ids ↔ `ActionId`:** `livebrainstorm_core::menu` ↔ `ActionId` in `src/lib/actions.ts`. `menu::tests::every_item_is_a_frontend_action_and_every_action_has_an_item` reads the TS file and fails on drift.
- **Window events:** `settings-changed` (no payload), `instructions-changed` (`events::InstructionsChanged { root, source }` ↔ `InstructionsChangedEvent`) and `menu` (`events::MenuEvent { id }` ↔ `MenuEvent`), in `src-tauri/src/events.rs` ↔ `src/lib/events.ts`. `menu` is emitted to one window, so `onMenu` must listen with `getCurrentWindow().listen`: a global `listen` has target `Any`, which receives every emit, `emit_to` included.
```

In the `- **New IPC command:**` line, replace "Plugin permissions do: `src-tauri/capabilities/default.json` grants only `core:default` and `dialog:allow-open`." with the sentence below. Add `core:window:allow-destroy` to the list if Task 4's Step 9 applied.

```markdown
Plugin permissions do: `src-tauri/capabilities/default.json` grants `core:default`, `dialog:allow-open`, `core:window:allow-set-title` and `core:window:allow-close` to the windows matching `w-*`.
```

In "## Deliberate decisions to preserve", add:

```markdown
- Window creation stays in Rust (`open_window`, `move_tab`), so the webview never gets `create-webview-window`.
```

- [ ] **Step 4: README: windows**

In `README.md`, add this subsection after Phase 2's `### Tabs` subsection (it ends with its "Shortcuts:" paragraph), before `### Requirements for the Claude subscription option`:

```markdown
### Windows

**File → New Window** (⌘N) opens another window with its own tabs. To move a tab, right-click it in the sidebar and choose *Move to new window*, or *Move to* and the window you want, named by the tab it shows. The tab opens there, and its notes analyse again at the next pause. Closing a window (⌘⇧W, or its close button) while others are open puts its tabs under *Recently closed*, where any window can reopen them. Closing the last window, or quitting with ⌘Q, keeps everything: the next launch reopens each window where it was, with its tabs. Settings, instructions and project briefs are shared by all windows, and a change in one shows in the others. On Windows and Linux, use Ctrl instead of ⌘.
```

- [ ] **Step 5: Final automated checks**

Run: `cargo test -p livebrainstorm-core && cargo clippy --workspace --all-targets && cargo check -p livebrainstorm && npm run typecheck && npm test`
Expected: everything passes, with no clippy warnings.

- [ ] **Step 6: Manual checklist (the user runs `npm run tauri dev`)**

Hand this list to the user. Each failure goes back to the task named in brackets.

1. **Quit and relaunch** [Task 4]: with two windows open, each moved and resized and holding different tabs, quit with ⌘Q. Relaunch.
   - Both windows come back where they were, with their tabs, drafts, notes and history.
   - No tab shows "Analysing…" at launch. (The usage footer's "no new runs" check comes with Phase 4.)
2. **Titles** [Task 4]: each title bar reads "LiveBrainstorm — \<shown tab\>", and it follows tab switches, renames and the first line as you type it. An empty window reads "LiveBrainstorm".
3. **New Window** [Tasks 4, 5]: ⌘N opens an empty window showing "Open a project", and its sidebar lists none of the other window's tabs. Minimize every window and press ⌘N: a window still opens.
4. **Two windows, two projects** [spec]: type in a tab in each window. Both rules animate, and neither run aborts the other.
5. **Brief** [spec]: build the brief in window 1. The same project's tab in window 2 turns current.
6. **Close a non-last window** [Task 4]: close one with the red button, and another with ⌘⇧W.
   - Their tabs appear under *Recently closed* in the remaining window. Reopen one there.
   - Close the last window: the app quits. Relaunch: that window comes back with its tabs.
7. **Close right after typing** [Task 4]: type a word and immediately close a non-last window. Reopen that tab from *Recently closed*: the word is there.
8. **Move a tab mid-run** [Task 6]:
   - While a tab shows "Analysing…", right-click it and choose *Move to new window*. The old window's rule stops and it shows the next tab in its sidebar; a new window opens showing the moved tab with its saved notes. Typing there runs again at the next pause.
   - Type a word in a tab and, without pausing, move it to another window: the word is in the moved tab's draft.
   - Right-click a tab and choose *Move to “…”*. The items name each other window by its active tab ("Empty window" for one with none, "(2)" on a repeated name). That window comes to the front showing the moved tab, last in its sidebar group.
9. **Shortcuts** [Task 5]:
   - In the draft, ⌘C, ⌘V, ⌘X, ⌘Z, ⌘⇧Z and ⌘A work.
   - ⌘T, ⌘W, ⌘⇧T, ⌘O, ⌃Tab, ⌃⇧Tab, ⌘\\ and ⌘, each act once, in the focused window only. ⌘W closes a tab, never the window. With two windows open, the other window's sidebar doesn't change.
   - With Settings open, ⌘W does nothing.
   - ⌘M minimizes and ⌘H hides.
   - If ⌃Tab does nothing, apply the fallback at the end of Task 5's Step 11.
10. **Shared settings** [Task 7]: change the model in window 1's Settings. Window 2's provider label follows.
11. **Shared instructions** [Task 7]: with the same project open in both windows, type in its Instructions drawer in window 1 for several seconds. Window 1's text never jumps, and window 2's drawer shows the edit a moment after you pause. Edit a template in window 2: window 1's template list follows.
12. **IPC in a runtime window** [Task 4]: in a second window, *Open a project…* shows the folder picker.
13. **Windows or Linux, if available** [Task 5]: the shortcuts use Ctrl, and each fires once.

- [ ] **Step 7: Commit**

```bash
git add AGENTS.md README.md
git commit -m "docs: windows, the app menu and cross-window events

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```
