# Tabs, windows and usage, Phase 2: Tabs UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A window shows a sidebar of tabs grouped by project. Each tab has its own draft, notes and analysis, analyses in the background, shows that it is analysing, keeps a browsable history of its last 5 analyses, and can be renamed, closed, reopened or deleted. Tab shortcuts are handled in the webview.

**Architecture:**
- **Pure rules, tested with vitest:**
  - `src/lib/tabStatus.ts`: a tab's rule state and status line.
  - `src/lib/tabList.ts`: grouping, visual order, titles, rename and sidebar-flag rules.
  - `src/lib/history.ts`: a snapshot as notes, and the `‹ 2 of 5 ›` position.
  - `src/lib/actions.ts`: key presses to `ActionId`.
- **Controllers outside React:**
  - `TabSession` (Phase 1) gains history browsing (`view`, `viewing`), a history refetch after `done`, and the background-tab context rule.
  - `src/lib/tabs.ts` is this window's tab store: the workspace view, the active tab, and one `TabSession` per shown tab.
  - `src/lib/projects.ts` caches each project's summary, instructions and brief build, and pushes each project's analysis context into `tabs.ts` once both the summary and the instructions have loaded.
- **Components:** `Sidebar`, `TabRow`, `ClosedTabs`, a shared `Menu` and `RULE` in `ui.tsx`, the `NotesPane` history header, and a new top bar. `App.tsx` becomes a three-column window wired to the two stores.
- There are no Rust changes in this phase.

**Tech Stack:** React 19 + Tailwind v4, vitest 3 (node environment, fake timers, `vi.mock`), Tauri v2 JS API (`@tauri-apps/api`, `@tauri-apps/plugin-dialog`).

**Spec:** `docs/superpowers/specs/2026-10-14-tabs-windows-and-usage-design.md`, Section 4 ("Frontend") in full, plus the frontend rows of Section 6 ("Error handling", "Testing") and "Documentation updates".

**Prerequisites:**
- Phase 1 (`docs/superpowers/plans/2026-10-14-tabs-phase-1-sessions-backend.md`) is complete on branch `feat/tabs-windows-usage`. All work in this plan happens on that branch, in the same worktree.
- Phases 3 and 4 come after this plan and build on its names: `tabs` (its draft saver is the module-level `saver`; Phase 3's Task 4 adds `tabs.flush()`, so this plan doesn't), `sessionFor`, `useActions` and `ActionId`, `TabRow`'s menu items, `Sidebar`'s `footer` prop, and `projects` in `src/lib/projects.ts`, which holds the per-project instructions (`projects.reloadInstructions(root)`, `projects.flush()`, `projectsState().instructions`) that today live in `App.tsx`.

### What Phase 1 leaves behind (this plan relies on it)

These exist when this plan starts. Task 1, Step 1 checks them.

- **`src/lib/ipc.ts`** has these types and `api` wrappers (names exact; Rust returns shown as TS):
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

  api.listWorkspace(window: string): Promise<WindowView>
  api.getTab(tabId: string): Promise<Tab>
  api.createTab(window: string, root: string | null): Promise<Tab>
  api.saveDraft(tabId: string, draft: string): Promise<void>
  api.renameTab(tabId: string, title: string | null): Promise<void>
  api.activateTab(window: string, tabId: string): Promise<void>
  api.closeTab(tabId: string): Promise<void>        // cancels the tab's run; emits workspace-changed
  api.reopenTab(tabId: string, window: string): Promise<void>   // Precondition "That tab is already open in another window."
  api.deleteTab(tabId: string): Promise<void>       // closed tabs only
  api.importLegacyDrafts(window: string, drafts: LegacyDraft[], activeRoot: string | null): Promise<void>
  api.openProject(path: string): Promise<ProjectSummary>     // scans and replaces the registry entry
  api.projectSummary(root: string): Promise<ProjectSummary>  // scans only if not loaded
  api.analyze(tabId: string, draft: string, previousDraft: string | null, instructions: string, context: string,
              onEvent: (e: UiEvent) => void): Promise<void>
  api.cancelAnalysis(tabId: string): Promise<void>
  api.buildBrief(root: string, onEvent: (e: UiEvent) => void): Promise<void>
  ```
  `getInstructions`, `saveProjectInstructions`, `saveTemplates`, `restoreStarterTemplates`, `getSettings`, `saveSettings` and `warmUp` are unchanged from today.
- **`src/lib/windowId.ts`:** `export function currentWindowId(): string` (the window label when it matches `/^w-/`, else `"w-1"`).
- **`src/lib/events.ts`:** `onBriefStatus(fn: (e: BriefStatusEvent) => void)` and `onWorkspaceChanged(fn: (e: WorkspaceChangedEvent) => void)`, each returning `Promise<UnlistenFn>` and built on the global `listen`. `BriefStatusEvent` is `{ root: string; state: "building" | "done" | "error"; message: string | null }` and `WorkspaceChangedEvent` is `{ windows: string[] }`. This plan never names the payload types; it reads the fields.
- **`src/lib/tabTitle.ts`:** `TITLE_CHARS = 48`, `firstLine(draft: string): string` (first non-empty line, trimmed, at most 48 Unicode scalars) and `tabTitle(t: { title: string | null; firstLine: string }): string` (title, else firstLine, else `"Untitled"`).
- **`src/lib/legacyDrafts.ts`** (with `legacyDrafts.test.ts`, 5 tests), the one-time `localStorage` import:
  ```ts
  type LegacyStorage = Pick<Storage, "length" | "key" | "getItem" | "removeItem">;
  export function collectLegacyDrafts(storage: Pick<Storage, "length" | "key" | "getItem">):
    { drafts: LegacyDraft[]; activeRoot: string | null; keys: string[] };
  export function browserStorage(): LegacyStorage | undefined;   // undefined where localStorage can't be used
  /** Imports every non-empty draft; removes the old keys only after that succeeds. Returns the message to show on failure, else null. */
  export function importLegacy(windowId: string, storage: LegacyStorage | undefined,
    importDrafts: (window: string, drafts: LegacyDraft[], activeRoot: string | null) => Promise<void>): Promise<string | null>;
  ```
  The failure message is `Couldn't bring over your earlier drafts: <error>`. Task 6 reuses `browserStorage` and `importLegacy`.
- **`src/lib/tabContext.ts`** (with `tabContext.test.ts`): `tabContext(root, project, instructions)`, which only Phase 1's `App.tsx` uses. Task 9 deletes both files, because `projects.ts` (Task 7) takes over the rule.
- **`src/lib/useTabSession.ts`:** `useTabSession(s: TabSession | null): TabView | null` (with `useTabSession.test.ts`).
- **`src/lib/tabSession.ts`:** `MIN_CHARS`, `normalize`, `TabSession`, `TabSessionDeps` and `TabView` as in Task 4's Interfaces, without `view()` and without the optional `getTab` dep. Phase 1's `viewing` is always 0, `contextDirty` always false, `activate()` and `deactivate()` do nothing, `setContext` always re-runs, and `history` is restore's copy. `src/lib/tabSession.test.ts` holds 16 tests that only pin behaviour both versions share. `useLiveAnalysis.ts` is gone.
- **`src/components/NotesPane.tsx`:** props `{ analysis: AnalysisState & { stale: boolean; rerun: () => void }; hasProject: boolean; briefMissing: boolean; onBuildBrief: () => void }`. It still defines `type RuleState = "idle" | "thinking" | "stale" | "fresh";` and `const RULE: Record<RuleState, string> = { … };` itself and imports `{ Button, Eyebrow, cx }` from `./ui`. `ui.tsx`, `TopBar.tsx` and `BriefDrawer.tsx` are unchanged from today.
- **`src/App.tsx`:** a single-pane window driving one `TabSession` for the window's active tab. It imports the `localStorage` drafts on a `fresh` workspace (`importLegacy`), and creates a no-project tab when the window has none. It imports from `./lib/`: `analysisState`, `brief`, `events`, `instructions`, `ipc`, `legacyDrafts`, `providerOptions`, `saver`, `tabContext`, `tabSession`, `useTabSession` and `windowId`. Task 9 replaces it.
- **`README.md`** is unchanged by Phase 1. Phase 1's AGENTS.md edits: the Architecture rows for UI and Core, live analysis steps 1, 2 and 4, the brief paragraph, "Loaded projects", the persistence lines (sessions, upgrade), the sessions and events contracts, and the id-validation decision.
- **Backend:** `analyze` records the snapshot *before* it sends `done`, so `getTab` right after `done` includes it. `build_brief` emits `brief-status` to every window; a second build of a root whose build is still running returns `Ok(())` at once, with no Channel events. Every session command, `rename_tab` included, emits `workspace-changed`; `save_draft` emits it only when the draft's first line changed. `close_tab` moves the window's backend `active` to the tab that took the closed one's place in the *stored* order. `reopen_tab` makes the reopened tab active in the target window. `get_tab` on a closed tab fails with "This tab was closed."

## Global Constraints

- **Layout:** `main` is a three-column grid, `[sidebar 248px | draft 3fr | notes minmax(320px, 2fr)]`. The draft and notes panes keep their own `min-h-0` and `overflow-y-auto`.
- **Sidebar collapse:** ⌘\\ toggles it. It starts collapsed when the window is narrower than 1,100 px. The state is per window in `localStorage` (`livebrainstorm.sidebar.<windowId>`), read and written inside `try`/`catch`. A stored choice wins over the width rule.
- **Groups:** tabs are grouped by project, in order of first appearance in the window's tab list. Tabs without a project sit under "No project". A new tab joins the end of its project's group. ⌃Tab / ⌃⇧Tab follow the visual order, not the stored order.
- **Tab row:** a 2 px × 28 px rule at its left with `RULE`'s state classes (`animate-fill` while thinking, solid when current, dashed when the draft moved on, faint when idle). The active row uses `bg-subtle`. `RULE` and `RuleState` live in `src/components/ui.tsx`, shared with the rule between the panes.
- **Reduced motion:** the rule is static (`motion-reduce:animate-none`, `opacity-50`) and the status text carries the state. The collapsed group's dot uses `animate-pulse-dot motion-reduce:animate-none`.
- **History:** `viewing` 0 is the live state; `viewing` k ≥ 1 is `history[k]`. After `done` the session refetches history with `getTab`. On `started`, `viewing` goes back to 0. Arrows are disabled at the ends and hidden with fewer than two analyses.
- **Background tabs:** a pause fires whether or not the tab is shown. A context change on a background tab only sets `contextDirty`; `activate()` runs that re-run when the draft has at least 24 normalized characters.
- **Relaunch starts no run** (spec, success criteria): a restored tab whose draft matches its latest analysis starts none, and neither does the first context a tab with no saved analysis receives. A pending pause reads that context when it fires. A project that can't be read (moved, deleted) hands its tabs no key (`null`), only the instructions, so they keep the context their saved notes ran with and never re-run because the folder is missing.
- **Context per tab:** `analysisContextKey(root, brief?.createdAt, instructions.current)`. The window pushes a project's key to its tabs only after it has loaded both that root's summary and its instructions.
- **Shortcuts (webview, this phase):** ⌘T new tab on the active tab's project, ⌘O open a project, ⌘⇧T reopen the last closed tab, ⌘W close the tab, ⌃Tab / ⌃⇧Tab next / previous tab, ⌘\\ toggle the sidebar, ⌘, settings. Ctrl replaces ⌘ on Windows and Linux; ⌃Tab is Ctrl everywhere. `new-window` (⌘N) and `close-window` (⌘⇧W) are recognised by `keyToAction` but not bound until Phase 3.
- **Window title:** `document.title = "LiveBrainstorm — " + tabTitle(active)`, or `"LiveBrainstorm"` with no active tab.
- **Copy, verbatim:** "Analysing…", "Draft moved on", "Notes current", "No project", "Untitled", "Open a project…", "Recently closed (n)", "Reopen", "Delete", "Delete for good?", "Yes", "No", "Rename", "Close", "Project brief…", "Rebuild brief", "Notes on an earlier draft.", "Show that draft", "Checks stopped when you edited.", "Couldn't read a tab; kept it as \<file\>.bad".
- **No browser dialogs:** no `alert`, `confirm` or `prompt`. Delete asks inline.
- **Saving never blocks typing:** draft saves go through `createSaver` keyed by tab id (500 ms), flushed on tab switch, before closing a tab and on `pagehide`. A failed save shows in the sidebar footer in `text-fault` and retries with the next edit.
- **Process:**
  - Branch `feat/tabs-windows-usage`. One commit per task, each ending with the trailer lines below.
  - Never run `cargo fmt` across the repo. This phase changes no Rust; `cargo test -p livebrainstorm-core`, `cargo check -p livebrainstorm` and `cargo clippy --workspace --all-targets` must stay green and warning-free anyway.
  - UI uses semantic colour tokens and `ui.tsx` primitives, never raw palette classes or `dark:`.
  - The agent shell has Node 20.11, so `npm run tauri dev` can't run there. Manual checks are a checklist for the user.
  - Commit trailer:
    ```
    Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
    Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47
    ```

## Review Focus

1. **Typing in a tab, then switching away before the pause.** Expected: the hidden tab still analyses, and its pending draft save lands on that tab, never the one you switched to. Pinned in Task 4 (`a background tab's pause fires after you switch away`) and Task 6 (`switching flushes the pending save of the tab you left`).
2. **The brief is rebuilt or instructions edited with several tabs of one project open.** Expected: only the visible tab re-runs; hidden tabs re-run when shown, and a tab shown at launch starts no run while its project data is still loading. Pinned in Task 4 (`a context change on a background tab waits for activate()`, and `a tab with no saved analysis takes its first context without a run` for a long draft that was never analysed) and Task 7 (`a project's context reaches its tabs only after both its summary and its instructions load`).
3. **Closing the active tab.** Expected: the next tab in the sidebar's order shows (the previous one when it was last), the backend's own pick (the next tab in the *stored* order) never shows even for a moment, and closing the last tab leaves the window empty. Pinned in Task 2 (`closing a tab shows the next one, else the previous, else none`) and Task 6 (`closing the active tab shows the next one in the sidebar's order`, which also checks that the next tab is activated before the close, and `closing the last tab leaves the window empty`).
4. **Renaming a tab and confirming without a change, or clearing the name.** Expected: an automatic title stays automatic (it keeps following the draft), and a blank name returns to automatic. Pinned in Task 2 (`renameResult`).
5. **Two projects with the same folder name** (`~/work/app` and `~/oss/app`). Expected: their sidebar groups are told apart. Pinned in Task 2 (`two projects with the same folder name get their parent folder`).

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src/components/ui.tsx` (modify) | `RULE` and `RuleState` (moved from `NotesPane`), the `mini` button variant, `Menu` | 1, 8 |
| `src/lib/tabStatus.ts` (+ test) | `ruleState`, `statusLine`, `anyThinking` | 1 |
| `src/lib/tabList.ts` (+ test) | `projectName`, `groupTabs`, `visualOrder`, `stepTab`, `afterClose`, `rowTitle`, `windowTitle`, `renameResult`, `unreadableNotice`, `readCollapsed`, `writeCollapsed` | 2 |
| `src/lib/history.ts` (+ test) | `snapshotAnalysis`, `historyPosition`, `snapshotTime` | 3 |
| `src/lib/tabSession.ts` (replace) | Phase 1's controller plus `view`, history refetch and the background context rule | 4 |
| `src/lib/tabSession.tabs.test.ts` (new) | `TabSession` behaviour that only matters with several tabs | 4 |
| `src/lib/actions.ts` (+ test) | `ActionId`, `keyToAction`, `keyToActionFor`, `useActions` | 5 |
| `src/lib/tabs.ts` (+ test) | This window's tab store, the legacy import on a fresh workspace (through Phase 1's `legacyDrafts.ts`), workspace-changed refresh | 6 |
| `src/lib/projects.ts` (+ test) | Per-root summaries, instructions, templates, brief builds; context push | 7 |
| `src/components/TabRow.tsx`, `ClosedTabs.tsx`, `Sidebar.tsx` (new) | The sidebar | 8 |
| `src/components/NotesPane.tsx` (replace), `TopBar.tsx` (replace), `BriefDrawer.tsx`, `src/App.tsx` (replace) | History header, new top bar, the three-column window | 9 |
| `src/lib/tabContext.ts`, `src/lib/tabContext.test.ts` (delete) | Phase 1's single-tab context rule, replaced by `projects.ts` | 9 |
| `AGENTS.md`, `README.md` (modify) | AGENTS.md: the Architecture UI row, UI section (sidebar, `TabSession`/`tabs.ts`/`projects.ts`, the indicator, history, shortcuts), live analysis step 1, the sidebar flag, the saved-checks contract. README: a *Tabs* subsection and two sentences made per-tab | 9 |

---

### Task 1: Tab status rules and the shared rule classes

**Files:**
- Modify: `src/components/ui.tsx` (add `RuleState` and `RULE`)
- Modify: `src/components/NotesPane.tsx` (import them instead of defining them)
- Create: `src/lib/tabStatus.ts`
- Test: `src/lib/tabStatus.test.ts`

**Interfaces:**
- Consumes (Phase 1):
  - `TabView` from `src/lib/tabSession.ts`: `{ draft: string; analysis: AnalysisState; tooShort: boolean; stale: boolean; contextDirty: boolean; viewing: number; history: Snapshot[] }`
  - From `src/lib/analysisState.ts` (unchanged):
    ```ts
    export type AnalysisStatus = "idle" | "thinking" | "done" | "error";
    export interface AnalysisState {
      status: AnalysisStatus; lanes: string[]; markdown: string; error: string | null; analyzedDraft: string;
      checking: { count: number; total: number } | null; checks: CheckResult[]; notices: string[];
      checksStopped: boolean; partial: boolean;
    }
    export const initialAnalysis: AnalysisState;   // status "idle", empty everything
    ```
  - `firstLine(draft: string): string` from `src/lib/tabTitle.ts`
  - Phase 1's `NotesPane.tsx` defines `type RuleState` and `const RULE` itself (exact text in Step 4) and imports `{ Button, Eyebrow, cx }` from `./ui`.
- Produces:
  - In `src/components/ui.tsx`: `export type RuleState = "idle" | "thinking" | "stale" | "fresh";` and `export const RULE: Record<RuleState, string>`
  - In `src/lib/tabStatus.ts`:
    - `export type TabLike = Pick<TabView, "analysis" | "stale">`
    - `ruleState(v: TabLike | null): RuleState`
    - `statusLine(v: TabLike | null): { text: string; fault: boolean } | null`
    - `anyThinking(views: (TabLike | null | undefined)[]): boolean`

- [ ] **Step 1: Check what Phase 1 left behind**

Run:
```bash
grep -n "^export" src/lib/tabSession.ts src/lib/tabTitle.ts src/lib/windowId.ts src/lib/events.ts src/lib/useTabSession.ts src/lib/legacyDrafts.ts src/lib/tabContext.ts
grep -nE "interface (Snapshot|Tab|TabSummary|WindowView|LegacyDraft) |listWorkspace|getTab|createTab|saveDraft|renameTab|activateTab|closeTab|reopenTab|deleteTab|importLegacyDrafts|projectSummary|buildBrief|cancelAnalysis" src/lib/ipc.ts
grep -n "type RuleState\|const RULE" src/components/NotesPane.tsx
test ! -e src/lib/useLiveAnalysis.ts && echo "useLiveAnalysis is gone"
```
Expected: `MIN_CHARS`, `normalize`, `TabSession`, `TabSessionDeps`, `TabView`, `TITLE_CHARS`, `firstLine`, `tabTitle`, `currentWindowId`, `BriefStatusEvent`, `WorkspaceChangedEvent`, `onBriefStatus`, `onWorkspaceChanged`, `useTabSession`, `collectLegacyDrafts`, `browserStorage`, `importLegacy`, `LoadedProject`, `LoadedInstructions` and `tabContext` are exported; every `ipc.ts` name above is present; `NotesPane.tsx` still has both lines; the last line prints `useLiveAnalysis is gone`. If anything is missing, stop: Phase 1 isn't complete.

- [ ] **Step 2: Write the failing test**

Create `src/lib/tabStatus.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { initialAnalysis, type AnalysisState } from "./analysisState";
import { anyThinking, ruleState, statusLine } from "./tabStatus";

const tab = (status: AnalysisState["status"], stale = false, error: string | null = null) => ({
  analysis: { ...initialAnalysis, status, error },
  stale,
});

describe("ruleState", () => {
  it("follows the rule between the panes", () => {
    expect(ruleState(null)).toBe("idle");
    expect(ruleState(tab("idle"))).toBe("idle");
    expect(ruleState(tab("thinking", true))).toBe("thinking");
    expect(ruleState(tab("done", true))).toBe("stale");
    expect(ruleState(tab("done"))).toBe("fresh");
    expect(ruleState(tab("error"))).toBe("idle");
  });
});

describe("statusLine", () => {
  it("says what the tab is doing", () => {
    expect(statusLine(tab("thinking"))).toEqual({ text: "Analysing…", fault: false });
    expect(statusLine(tab("done", true))).toEqual({ text: "Draft moved on", fault: false });
    expect(statusLine(tab("done"))).toEqual({ text: "Notes current", fault: false });
    expect(statusLine(tab("idle"))).toBeNull();
    expect(statusLine(null)).toBeNull();
  });

  it("shows an error's first line as a fault", () => {
    expect(statusLine(tab("error", false, "\nClaude Code isn't installed.\nInstall it, then try again."))).toEqual({
      text: "Claude Code isn't installed.",
      fault: true,
    });
    expect(statusLine(tab("error"))).toEqual({ text: "Couldn't think this through", fault: true });
  });
});

describe("anyThinking", () => {
  it("is true when any tab in a group is analysing", () => {
    expect(anyThinking([null, tab("done"), tab("thinking")])).toBe(true);
    expect(anyThinking([undefined, tab("done", true)])).toBe(false);
    expect(anyThinking([])).toBe(false);
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `npx vitest run src/lib/tabStatus.test.ts`
Expected: FAIL with `Failed to resolve import "./tabStatus"`.

- [ ] **Step 4: Move the rule's state classes into `ui.tsx`**

In `src/components/ui.tsx`, add after the `cx` line:

```ts
/** The accent rule's states: solid when notes are current, animated while thinking, dashed when the draft has moved on. */
export type RuleState = "idle" | "thinking" | "stale" | "fresh";

/** State classes for the accent rule, shared by the rule between the panes and each tab row's rule. */
export const RULE: Record<RuleState, string> = {
  idle: "bg-accent scale-y-0",
  fresh: "bg-accent",
  thinking: "bg-accent animate-fill motion-reduce:animate-none motion-reduce:opacity-50",
  // Dashed: the notes are still there, but your draft has moved on.
  stale: "bg-[repeating-linear-gradient(to_bottom,var(--color-accent)_0_6px,transparent_6px_12px)] opacity-70",
};
```

In `src/components/NotesPane.tsx`, delete the line `type RuleState = "idle" | "thinking" | "stale" | "fresh";` and the whole `const RULE: Record<RuleState, string> = { … };` block, and add `RULE` and `type RuleState` to the existing import from `./ui`, for example:

```ts
import { Button, Eyebrow, RULE, cx, type RuleState } from "./ui";
```

- [ ] **Step 5: Create `src/lib/tabStatus.ts`**

```ts
// What a tab shows about its analysis: the rule's state and a status line.
// Shared by the rule between the panes (NotesPane) and each sidebar row.
import type { RuleState } from "../components/ui";
import type { TabView } from "./tabSession";
import { firstLine } from "./tabTitle";

/** The parts of a tab's state the indicator reads. */
export type TabLike = Pick<TabView, "analysis" | "stale">;

/** `null` is a tab with no session yet (not shown since launch): faint, like idle. */
export function ruleState(v: TabLike | null): RuleState {
  if (!v) return "idle";
  if (v.analysis.status === "thinking") return "thinking";
  if (v.stale) return "stale";
  return v.analysis.status === "done" ? "fresh" : "idle";
}

/** The line under a tab's title. The text carries the state when motion is reduced. */
export function statusLine(v: TabLike | null): { text: string; fault: boolean } | null {
  if (!v) return null;
  const a = v.analysis;
  if (a.status === "thinking") return { text: "Analysing…", fault: false };
  if (a.status === "error") return { text: firstLine(a.error ?? "") || "Couldn't think this through", fault: true };
  if (v.stale) return { text: "Draft moved on", fault: false };
  if (a.status === "done") return { text: "Notes current", fault: false };
  return null;
}

/** A collapsed project group pulses when any of its tabs is analysing. */
export const anyThinking = (views: (TabLike | null | undefined)[]) => views.some((v) => v?.analysis.status === "thinking");
```

- [ ] **Step 6: Run the tests and the typecheck**

Run: `npx vitest run src/lib/tabStatus.test.ts && npm test && npm run typecheck`
Expected: the four new tests PASS, the whole suite passes (including Phase 1's), and there are no type errors.

- [ ] **Step 7: Commit**

```bash
git add src/components/ui.tsx src/components/NotesPane.tsx src/lib/tabStatus.ts src/lib/tabStatus.test.ts
git commit -m "feat(ui): tab status rules; share the rule's state classes from ui.tsx

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 2: Tab list rules: grouping, order, titles, rename and the sidebar flag

**Files:**
- Create: `src/lib/tabList.ts`
- Test: `src/lib/tabList.test.ts`

**Interfaces:**
- Consumes (Phase 1):
  - `TabSummary` from `src/lib/ipc.ts`: `{ id: string; root: string | null; title: string | null; firstLine: string; updatedAt: number; closedAt: number | null }`
  - `firstLine(draft: string): string` and `tabTitle(t: { title: string | null; firstLine: string }): string` from `src/lib/tabTitle.ts`
- Produces (`src/lib/tabList.ts`):
  - `NO_PROJECT = "No project"`, `NARROW_WINDOW = 1100`
  - `projectName(root: string | null): string`
  - `interface TabGroup<T> { root: string | null; name: string; tabs: T[] }`
  - `groupKey(root: string | null): string` (`""` for no project)
  - `groupTabs<T extends { root: string | null }>(list: T[]): TabGroup<T>[]`
  - `visualOrder(groups: TabGroup<{ id: string }>[]): string[]`
  - `stepTab(order: string[], current: string | null, dir: 1 | -1): string | null`
  - `afterClose(order: string[], closing: string): string | null`
  - `rowTitle(t: Pick<TabSummary, "title" | "firstLine">, liveDraft: string | null): string`
  - `windowTitle(title: string | null): string`
  - `renameResult(input: string, current: string | null, auto: string): string | null | undefined` (`undefined` = no change)
  - `unreadableNotice(path: string): string`
  - `sidebarKey(windowId: string): string`
  - `readCollapsed(windowId: string, width: number, storage?: Pick<Storage, "getItem" | "setItem">): boolean`
  - `writeCollapsed(windowId: string, collapsed: boolean, storage?: Pick<Storage, "getItem" | "setItem">): void`

- [ ] **Step 1: Write the failing test**

Create `src/lib/tabList.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { TabSummary } from "./ipc";
import {
  afterClose,
  groupTabs,
  projectName,
  readCollapsed,
  renameResult,
  rowTitle,
  stepTab,
  unreadableNotice,
  visualOrder,
  windowTitle,
  writeCollapsed,
} from "./tabList";
import { tabTitle } from "./tabTitle";

const t = (id: string, root: string | null, firstLine = "", title: string | null = null): TabSummary => ({
  id,
  root,
  title,
  firstLine,
  updatedAt: 0,
  closedAt: null,
});

describe("projectName", () => {
  it("is the folder name, for either separator", () => {
    expect(projectName("/Users/me/code/shop-api")).toBe("shop-api");
    expect(projectName("/Users/me/code/shop-api/")).toBe("shop-api");
    expect(projectName("C:\\code\\shop-api")).toBe("shop-api");
    expect(projectName(null)).toBe("No project");
  });
});

describe("groupTabs", () => {
  it("groups by project in order of first appearance, keeping list order inside a group", () => {
    const groups = groupTabs([t("a1", "/p/a"), t("n1", null), t("b1", "/p/b"), t("a2", "/p/a")]);
    expect(groups.map((g) => g.name)).toEqual(["a", "No project", "b"]);
    expect(groups[0].tabs.map((x) => x.id)).toEqual(["a1", "a2"]);
  });

  it("a new tab joins the end of its project's group", () => {
    // create_tab appends to the stored list; the group shows it last.
    const after = groupTabs([t("a1", "/p/a"), t("b1", "/p/b"), t("a2", "/p/a")]);
    expect(visualOrder(after)).toEqual(["a1", "a2", "b1"]);
  });

  it("two projects with the same folder name get their parent folder", () => {
    const names = groupTabs([t("x", "/work/app"), t("y", "/oss/app"), t("z", "/work/api")]).map((g) => g.name);
    expect(names).toEqual(["app · work", "app · oss", "api"]);
  });
});

describe("tab order", () => {
  const order = visualOrder(groupTabs([t("a1", "/a"), t("b1", "/b"), t("a2", "/a")]));

  it("⌃Tab follows the sidebar's order and wraps", () => {
    expect(order).toEqual(["a1", "a2", "b1"]);
    expect(stepTab(order, "a2", 1)).toBe("b1"); // the stored list would wrap to a1
    expect(stepTab(order, "b1", 1)).toBe("a1");
    expect(stepTab(order, "a1", -1)).toBe("b1");
    expect(stepTab(order, null, 1)).toBe("a1");
    expect(stepTab(order, null, -1)).toBe("b1");
    expect(stepTab([], null, 1)).toBeNull();
  });

  it("closing a tab shows the next one, else the previous, else none", () => {
    expect(afterClose(order, "a1")).toBe("a2");
    expect(afterClose(order, "b1")).toBe("a2");
    expect(afterClose(["only"], "only")).toBeNull();
    expect(afterClose(order, "gone")).toBeNull();
  });
});

describe("titles", () => {
  it("tabTitle: the title, else the first line, else Untitled", () => {
    expect(tabTitle({ title: "Checkout retry", firstLine: "what if" })).toBe("Checkout retry");
    expect(tabTitle({ title: null, firstLine: "what if" })).toBe("what if");
    expect(tabTitle({ title: null, firstLine: "" })).toBe("Untitled");
  });

  it("a shown tab's title follows its live draft", () => {
    const saved = t("a", "/a", "old first line");
    expect(rowTitle(saved, null)).toBe("old first line");
    expect(rowTitle(saved, "\n  new first line\nmore")).toBe("new first line");
    expect(rowTitle({ ...saved, title: "Named" }, "new first line")).toBe("Named");
  });

  it("the window title names the active tab", () => {
    expect(windowTitle("Checkout retry")).toBe("LiveBrainstorm — Checkout retry");
    expect(windowTitle(null)).toBe("LiveBrainstorm");
  });
});

describe("renameResult", () => {
  it("confirming the automatic title unchanged keeps it automatic", () => {
    expect(renameResult("what if", null, "what if")).toBeUndefined();
    expect(renameResult("  what if  ", null, "what if")).toBeUndefined();
  });

  it("a blank name returns to the automatic title", () => {
    expect(renameResult("   ", "Named", "what if")).toBeNull();
    expect(renameResult("", null, "what if")).toBeUndefined();
  });

  it("a new name is trimmed; the same name is no change", () => {
    expect(renameResult("  Checkout retry ", null, "what if")).toBe("Checkout retry");
    expect(renameResult("Named", "Named", "what if")).toBeUndefined();
  });
});

describe("unreadableNotice", () => {
  it("names the kept copy", () => {
    expect(unreadableNotice("/data/sessions/tabs/t-1a-0.json.bad")).toBe("Couldn't read a tab; kept it as t-1a-0.json.bad");
    expect(unreadableNotice("C:\\data\\sessions\\workspace.json.bad.1")).toBe(
      "Couldn't read the saved windows; kept them as workspace.json.bad.1",
    );
  });
});

describe("sidebar flag", () => {
  const memory = () => {
    const m = new Map<string, string>();
    return { m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
  };

  it("starts collapsed in a narrow window, unless the window remembers a choice", () => {
    const s = memory();
    expect(readCollapsed("w-1", 1099, s)).toBe(true);
    expect(readCollapsed("w-1", 1100, s)).toBe(false);
    writeCollapsed("w-1", false, s);
    expect(s.m.get("livebrainstorm.sidebar.w-1")).toBe("0");
    expect(readCollapsed("w-1", 900, s)).toBe(false);
    expect(readCollapsed("w-2", 900, s)).toBe(true);
  });

  it("storage that throws falls back to the width and never breaks", () => {
    const broken = {
      getItem: (): string | null => {
        throw new Error("SecurityError");
      },
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
    };
    expect(readCollapsed("w-1", 900, broken)).toBe(true);
    expect(() => writeCollapsed("w-1", true, broken)).not.toThrow();
    expect(readCollapsed("w-1", 1400, undefined)).toBe(false);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/lib/tabList.test.ts`
Expected: FAIL with `Failed to resolve import "./tabList"`.

- [ ] **Step 3: Create `src/lib/tabList.ts`**

```ts
// The sidebar's tab list, kept pure so it's testable: grouping by project,
// the visual order that ⌃Tab follows, titles, renaming, notices, and the
// per-window collapse flag.
import type { TabSummary } from "./ipc";
import { firstLine, tabTitle } from "./tabTitle";

export const NO_PROJECT = "No project";
export const NARROW_WINDOW = 1100;

const segments = (path: string) => path.split(/[\\/]+/).filter(Boolean);

/** The folder name, for either path separator; "No project" for null. */
export function projectName(root: string | null): string {
  if (root === null) return NO_PROJECT;
  return segments(root).at(-1) ?? root;
}

export interface TabGroup<T> {
  root: string | null;
  name: string;
  tabs: T[];
}

/** A stable key for a group: its root, or "" for no project. */
export const groupKey = (root: string | null) => root ?? "";

/**
 * Groups tabs by project in order of first appearance; inside a group, tabs
 * keep the list's order. `create_tab` appends to the list, so a new tab joins
 * the end of its group. Two projects with the same folder name also show
 * their parent folder ("app · work").
 */
export function groupTabs<T extends { root: string | null }>(list: T[]): TabGroup<T>[] {
  const groups = new Map<string | null, TabGroup<T>>();
  for (const t of list) {
    let g = groups.get(t.root);
    if (!g) {
      g = { root: t.root, name: projectName(t.root), tabs: [] };
      groups.set(t.root, g);
    }
    g.tabs.push(t);
  }
  const all = [...groups.values()];
  for (const g of all) {
    if (g.root === null) continue;
    const base = projectName(g.root);
    const twin = all.some((o) => o !== g && o.root !== null && projectName(o.root) === base);
    const parent = segments(g.root).at(-2);
    if (twin && parent) g.name = `${base} · ${parent}`;
  }
  return all;
}

/** Tab ids in the order the sidebar shows them. ⌃Tab follows this, not the stored list. */
export const visualOrder = (groups: TabGroup<{ id: string }>[]): string[] => groups.flatMap((g) => g.tabs.map((t) => t.id));

/** The tab `dir` steps to from `current`, wrapping at the ends; null when there are none. */
export function stepTab(order: string[], current: string | null, dir: 1 | -1): string | null {
  if (order.length === 0) return null;
  const i = current === null ? -1 : order.indexOf(current);
  if (i === -1) return dir === 1 ? order[0] : order[order.length - 1];
  return order[(i + dir + order.length) % order.length];
}

/** The tab to show after closing `closing`: the next in visual order, else the previous, else none. */
export function afterClose(order: string[], closing: string): string | null {
  const i = order.indexOf(closing);
  if (i === -1) return null;
  return order[i + 1] ?? order[i - 1] ?? null;
}

/** A row's title. A shown tab's live draft beats the saved first line, so the title follows typing. */
export const rowTitle = (t: Pick<TabSummary, "title" | "firstLine">, liveDraft: string | null) =>
  tabTitle({ title: t.title, firstLine: liveDraft === null ? t.firstLine : firstLine(liveDraft) });

export const windowTitle = (title: string | null) => (title === null ? "LiveBrainstorm" : `LiveBrainstorm — ${title}`);

/**
 * What a rename commits: a trimmed title, null for the automatic title, or
 * undefined when nothing changed. Confirming the automatic title as shown
 * keeps it automatic, so it goes on following the draft.
 */
export function renameResult(input: string, current: string | null, auto: string): string | null | undefined {
  const v = input.trim();
  const next = v === "" || (current === null && v === auto) ? null : v;
  return next === current ? undefined : next;
}

/** The sidebar notice for a file kept as `.bad` (spec: "Couldn't read a tab; kept it as <file>.bad"). */
export function unreadableNotice(path: string): string {
  const file = segments(path).at(-1) ?? path;
  return file.startsWith("workspace.json")
    ? `Couldn't read the saved windows; kept them as ${file}`
    : `Couldn't read a tab; kept it as ${file}`;
}

type FlagStore = Pick<Storage, "getItem" | "setItem">;

export const sidebarKey = (windowId: string) => `livebrainstorm.sidebar.${windowId}`;

/** localStorage, or undefined where touching it throws (private mode, blocked site data). */
function browserStorage(): FlagStore | undefined {
  try {
    return typeof localStorage === "undefined" ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

/** A stored choice wins; without one, a window narrower than 1,100 px starts collapsed. */
export function readCollapsed(windowId: string, width: number, storage: FlagStore | undefined = browserStorage()): boolean {
  try {
    const v = storage?.getItem(sidebarKey(windowId));
    if (v === "1") return true;
    if (v === "0") return false;
  } catch {
    // Unreadable storage: fall back to the width.
  }
  return width < NARROW_WINDOW;
}

export function writeCollapsed(windowId: string, collapsed: boolean, storage: FlagStore | undefined = browserStorage()): void {
  try {
    storage?.setItem(sidebarKey(windowId), collapsed ? "1" : "0");
  } catch {
    // The choice just isn't remembered.
  }
}
```

- [ ] **Step 4: Run the tests and the typecheck**

Run: `npx vitest run src/lib/tabList.test.ts && npm test && npm run typecheck`
Expected: the 15 new tests PASS, the suite passes, and there are no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/tabList.ts src/lib/tabList.test.ts
git commit -m "feat(ui): tab list rules: groups by project, visual order, titles, rename, sidebar flag

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 3: History rules: a snapshot as notes, and the position header

**Files:**
- Create: `src/lib/history.ts`
- Test: `src/lib/history.test.ts`

**Interfaces:**
- Consumes:
  - `Snapshot` (Phase 1, `src/lib/ipc.ts`): `{ id: number; at: number; draft: string; context: string; markdown: string; partial: boolean; notices: string[]; checks: CheckResult[]; checksTotal: number | null; checksFaults: number | null; checksStopped: boolean }`. `at` is unix seconds; `checksTotal` is the claim count (`count`) from the last `checking` event, so fewer checks than that without `checksStopped` means the run was cut off (spec Section 1). `checksFaults` is that event's `total` (all Faults), null in files saved before the field existed.
  - `AnalysisState` (`src/lib/analysisState.ts`, unchanged; full definition in Task 1's Interfaces).
  - `checkSummary(checking: { count: number; total: number }, done: number, stopped: boolean): string` (`src/lib/checks.ts`, unchanged): "Checking 3 of 3 faults…" while `!stopped && done < checking.count`, else "1 of 3 faults checked".
  - `CheckResult` (`src/lib/ipc.ts`, unchanged): `{ claim: string; verdict: Verdict; confidence: number | null; path: string | null; line: number | null }`, with `Verdict = "confirmed" | "contradicted" | "unclear" | "notShown" | "notFound"`.
- Produces (`src/lib/history.ts`):
  - `CHECKS_CUT_OFF = "Checks stopped when you edited."`
  - `snapshotAnalysis(s: Snapshot): AnalysisState` (Task 4's `restore` and Task 9's `NotesPane` use it)
  - `interface HistoryPosition { show: boolean; label: string; earlier: number | null; later: number | null }`
  - `historyPosition(viewing: number, length: number): HistoryPosition`
  - `snapshotTime(at: number, now?: Date): string`

- [ ] **Step 1: Write the failing test**

Create `src/lib/history.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { checkSummary } from "./checks";
import { historyPosition, snapshotAnalysis, snapshotTime } from "./history";
import type { CheckResult, Snapshot } from "./ipc";

const snap = (over: Partial<Snapshot> = {}): Snapshot => ({
  id: 3,
  at: 1760443320,
  draft: "the draft",
  context: "/p|1|",
  markdown: "## Intent\n- a",
  partial: false,
  notices: [],
  checks: [],
  checksTotal: null,
  checksFaults: null,
  checksStopped: false,
  ...over,
});
const check: CheckResult = { claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("snapshotAnalysis", () => {
  it("shows the saved notes as done", () => {
    const a = snapshotAnalysis(snap({ partial: true, notices: ["Couldn't write Questions."] }));
    expect(a.status).toBe("done");
    expect(a.markdown).toBe("## Intent\n- a");
    expect(a.analyzedDraft).toBe("the draft");
    expect(a.partial).toBe(true);
    expect(a.notices).toEqual(["Couldn't write Questions."]);
    expect(a.checking).toBeNull();
  });

  it("checks cut off by an edit read as stopped, as they did live", () => {
    const a = snapshotAnalysis(snap({ checks: [check], checksTotal: 3, checksFaults: 5 }));
    expect(a.checksStopped).toBe(true);
    expect(a.notices).toEqual(["Checks stopped when you edited."]);
    // Live, 3 of the 5 Faults named code; the saved notes read the same way.
    expect(checkSummary(a.checking!, a.checks.length, a.checksStopped)).toBe("1 of 5 faults checked");
  });

  it("finished or failed checks stay as they were", () => {
    const finished = snapshotAnalysis(snap({ checks: [check], checksTotal: 1 }));
    expect(finished.checksStopped).toBe(false);
    expect(finished.notices).toEqual([]);
    // A file saved without checksFaults counts against the claims.
    expect(checkSummary(finished.checking!, 1, false)).toBe("1 of 1 fault checked");
    const failed = snapshotAnalysis(snap({ checksTotal: 2, checksStopped: true, notices: ["Jev is unavailable."] }));
    expect(failed.checksStopped).toBe(true);
    expect(failed.notices).toEqual(["Jev is unavailable."]);
  });
});

describe("historyPosition", () => {
  it("is hidden with fewer than two analyses", () => {
    expect(historyPosition(0, 0).show).toBe(false);
    expect(historyPosition(0, 1).show).toBe(false);
  });

  it("numbers chronologically, so the latest is n of n and ‹ goes back in time", () => {
    expect(historyPosition(0, 5)).toEqual({ show: true, label: "5 of 5", earlier: 1, later: null });
    expect(historyPosition(3, 5)).toEqual({ show: true, label: "2 of 5", earlier: 4, later: 2 });
    expect(historyPosition(4, 5)).toEqual({ show: true, label: "1 of 5", earlier: null, later: 3 });
    expect(historyPosition(9, 5).label).toBe("1 of 5");
  });
});

describe("snapshotTime", () => {
  it("adds the date only for another day", () => {
    const at = Date.UTC(2026, 9, 14, 12, 2) / 1000;
    const time = snapshotTime(at, new Date(at * 1000 + 60_000));
    const otherDay = snapshotTime(at, new Date(at * 1000 + 8 * 86_400_000));
    expect(time.length).toBeGreaterThan(0);
    expect(otherDay).not.toBe(time);
    expect(otherDay.endsWith(time)).toBe(true);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/lib/history.test.ts`
Expected: FAIL with `Failed to resolve import "./history"`.

- [ ] **Step 3: Create `src/lib/history.ts`**

```ts
// A tab's history of analyses (newest first, at most 5): an older snapshot
// shown as notes, and the `‹ 2 of 5 ›` header. Snapshot mirrors
// sessions::Snapshot in crates/livebrainstorm-core.
import type { AnalysisState } from "./analysisState";
import type { Snapshot } from "./ipc";

export const CHECKS_CUT_OFF = "Checks stopped when you edited.";

/**
 * A saved analysis as the notes pane shows it. Fewer checks than
 * `checksTotal`, without `checksStopped`, means an edit cut the run off: it
 * reads as stopped, as it did live. `checksFaults` keeps the summary's "of 5
 * faults"; a file saved before it existed counts against the claims instead.
 */
export function snapshotAnalysis(s: Snapshot): AnalysisState {
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

export interface HistoryPosition {
  show: boolean;
  label: string;
  /** The `viewing` index of the next older analysis, or null at the oldest. */
  earlier: number | null;
  /** The `viewing` index of the next newer analysis, or null at the latest. */
  later: number | null;
}

/**
 * `viewing` 0 is the latest. The label counts in time order, so the oldest is
 * "1 of n" and the latest "n of n", and ‹ goes back in time. Hidden with
 * fewer than two analyses.
 */
export function historyPosition(viewing: number, length: number): HistoryPosition {
  if (length < 2) return { show: false, label: "", earlier: null, later: null };
  const k = Math.min(Math.max(Math.trunc(viewing) || 0, 0), length - 1);
  return { show: true, label: `${length - k} of ${length}`, earlier: k < length - 1 ? k + 1 : null, later: k > 0 ? k - 1 : null };
}

/** "14:02" today; "Oct 12, 14:02" on another day. In the user's locale and time zone. */
export function snapshotTime(at: number, now: Date = new Date()): string {
  const d = new Date(at * 1000);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return time;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}
```

- [ ] **Step 4: Run the tests and the typecheck**

Run: `npx vitest run src/lib/history.test.ts && npm test && npm run typecheck`
Expected: the 6 new tests PASS, the suite passes, and there are no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/history.ts src/lib/history.test.ts
git commit -m "feat(ui): history rules: a snapshot as notes, cut-off checks, the position header

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 4: `TabSession`: history browsing, history refetch, and the background context rule

Phase 1's `TabSession` drives the one shown tab. This task replaces `src/lib/tabSession.ts` with a version that keeps Phase 1's public API and behaviour (the spec's "Per-tab state") and adds what several tabs need: `view(index)`, a history refetch after `done`, `viewing` reset on `started`, and `contextDirty` for hidden tabs. The whole file is given so it reads as one piece. Against Phase 1's file, the changes are: Phase 1's private `fromSnapshot` becomes Task 3's `snapshotAnalysis`; new fields `viewing`, `contextDirty`, `active` and `historyRequest`; `setContext` defers on a hidden tab, treats a tab's first context (no saved analysis) as no change, and takes a null key (a project that couldn't be read) as instructions only; `activate()` and `deactivate()` gain bodies; `view()` is new; `run` resets `viewing` on `started` and refetches the history on `done`; `emit` publishes `contextDirty` and `viewing`. Phase 1's own tests (`tabSession.test.ts`, 16 tests, and `useTabSession.test.ts`) are the guard that nothing drifted, and they pass unchanged against this file.

**Files:**
- Replace: `src/lib/tabSession.ts`
- Test: `src/lib/tabSession.tabs.test.ts` (new). Phase 1's `src/lib/tabSession.test.ts` and `src/lib/useTabSession.test.ts` must pass unchanged.

**Interfaces:**
- Consumes:
  - From `src/lib/analysisState.ts` (unchanged; `AnalysisState` in full in Task 1's Interfaces):
    ```ts
    export const initialAnalysis: AnalysisState;
    export const startAnalysis: (prev: AnalysisState, draft: string) => AnalysisState;  // status "thinking", old notes kept
    export function applyEvent(s: AnalysisState, ev: UiEvent): AnalysisState;
    export interface Analyzed { draft: string; context: string }
    /** done → { draft: sent, context } unless partial (then null); started → null; else unchanged. */
    export const lastAnalyzedAfter: (last: Analyzed | null, ev: UiEvent, sent: string, context: string) => Analyzed | null;
    /** null for a rerun, with no last analysis, or when its context differs; else last.draft. */
    export const previousDraftFor: (last: Analyzed | null, rerun: boolean, context: string) => string | null;
    ```
  - `snapshotAnalysis(s: Snapshot): AnalysisState` (Task 3)
  - From `src/lib/ipc.ts`: `errorMessage(e: unknown): string`; `UiEvent` (unchanged: `text { text, lane }`, `activity`, `done { text, partial? }`, `error { message }`, `checking { count, total }`, `check`, `notice`, `checksStopped`, `started`, `skipped`, `progress`); and Phase 1's `Snapshot` and `Tab` (as in "What Phase 1 leaves behind")
- Produces (`src/lib/tabSession.ts`; the contract's API, plus the optional `getTab` dep and two helper exports):
  ```ts
  export const MIN_CHARS = 24;
  export const normalize: (s: string) => string;
  export interface TabSessionDeps {
    analyze: (tabId: string, draft: string, previousDraft: string | null, instructions: string, context: string,
              onEvent: (e: UiEvent) => void) => Promise<void>;
    saveDraft: (tabId: string, draft: string) => void;   // caller wires createSaver (500 ms) keyed by tab id
    debounceMs: () => number;
    getTab?: (tabId: string) => Promise<Tab>;            // new, optional: the history refetch after `done`
  }
  export interface TabView {                // a new object only when something changed (useSyncExternalStore)
    draft: string; analysis: AnalysisState; tooShort: boolean; stale: boolean; contextDirty: boolean;
    viewing: number;                        // 0 = live state; k >= 1 = history[k]
    history: Snapshot[];                    // newest first
  }
  export class TabSession {
    constructor(tab: Tab, deps: TabSessionDeps);   // calls restore(tab); a new session starts active
    readonly id: string;
    readonly root: string | null;
    getState(): TabView;
    subscribe(fn: () => void): () => void;
    restore(tab: Tab): void;
    setDraft(text: string): void;
    setContext(key: string | null, instructions: string): void;  // null: the project couldn't be read; keep the context
    activate(): void;
    deactivate(): void;
    rerun(): void;
    dispose(): void;
    view(index: number): void;
  }
  ```

- [ ] **Step 1: Read Phase 1's version and its tests**

Run:
```bash
grep -n "^export\|^  [a-zA-Z]*(.*): \|^  readonly" src/lib/tabSession.ts
ls src/lib/tabSession*.test.ts
grep -rnE 'from "(\./|\.\./lib/|\./lib/)tabSession"' src
```
Expected: the exports `MIN_CHARS`, `normalize`, `TabSessionDeps`, `TabView` and `TabSession`, and the methods listed in Interfaces except `view`; the test files `tabSession.test.ts` and `useTabSession.test.ts`; and importers `src/App.tsx`, `src/lib/useTabSession.ts`, `src/lib/tabSession.test.ts`, `src/lib/useTabSession.test.ts` and (from Task 1) `src/lib/tabStatus.ts`. After Step 4, every name those files import must still be exported. If Phase 1 exported something not in the code below, copy that export into the new file unchanged.

- [ ] **Step 2: Write the failing tests**

Create `src/lib/tabSession.tabs.test.ts`:

```ts
// TabSession behaviour that only matters with several tabs: hidden tabs keep
// analysing, context changes wait for the tab to be shown, and the history
// can be browsed and stays current.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot, Tab, UiEvent } from "./ipc";
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
  draft: string;
  previousDraft: string | null;
  instructions: string;
  context: string;
  emit: (e: UiEvent) => void;
}

function harness(t: Tab, getTab?: TabSessionDeps["getTab"]) {
  const calls: Call[] = [];
  const deps: TabSessionDeps = {
    analyze: (_id, draft, previousDraft, instructions, context, onEvent) => {
      calls.push({ draft, previousDraft, instructions, context, emit: onEvent });
      return Promise.resolve();
    },
    saveDraft: vi.fn(),
    debounceMs: () => 1000,
    getTab,
  };
  return { s: new TabSession(t, deps), calls };
}

/** Lets promise callbacks run; fake timers don't touch microtasks. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("background tabs", () => {
  it("a background tab's pause fires after you switch away", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    s.deactivate();
    vi.advanceTimersByTime(1000);
    expect(calls.map((c) => c.draft)).toEqual([DRAFT]);
  });

  it("a context change on a background tab waits for activate()", () => {
    const { s, calls } = harness(tab());
    s.deactivate();
    s.setContext("/p|2|", "Focus on retries.");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().contextDirty).toBe(true);
    s.activate();
    expect(s.getState().contextDirty).toBe(false);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null, instructions: "Focus on retries.", context: "/p|2|" });
  });

  it("activate() skips the re-run when the draft is too short", () => {
    const { s, calls } = harness(tab({ draft: "too short", history: [snap({ draft: "too short" })] }));
    s.deactivate();
    s.setContext("/p|2|", "");
    s.activate();
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
  });

  it("on the shown tab, a new context re-reads the same draft with no gate", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|2|", "");
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].previousDraft).toBeNull();
  });

  it("a background run reads the new context, so showing the tab adds no second run", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setContext("/p|1|", "");
    s.setDraft(DRAFT);
    s.deactivate();
    s.setContext("/p|2|", "");
    expect(s.getState().contextDirty).toBe(true);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].context).toBe("/p|2|");
    expect(s.getState().contextDirty).toBe(false);
    s.activate();
    vi.advanceTimersByTime(5000);
    expect(calls).toHaveLength(1);
  });
});

describe("restore", () => {
  it("a tab with no saved analysis takes its first context without a run", () => {
    const { s, calls } = harness(tab({ history: [] })); // a long draft that was never analysed
    s.setContext("/p|1|", "Focus.");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ previousDraft: null, instructions: "Focus.", context: "/p|1|" });
  });

  it("a project that can't be read keeps the saved context and starts no run", () => {
    const { s, calls } = harness(tab()); // saved notes that ran with "/p|1|"
    s.setContext(null, "Focus.");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().analysis.status).toBe("done");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ instructions: "Focus.", context: "/p|1|" });
  });

  it("starts no run when the draft matches the latest analysis", () => {
    const { s, calls } = harness(tab({ draft: `  ${DRAFT}\n` }));
    s.setContext("/p|1|", "");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().analysis.status).toBe("done");
    expect(s.getState().stale).toBe(false);
  });

  it("a partial latest analysis is never the gate's comparison point", () => {
    const { s, calls } = harness(tab({ history: [snap({ partial: true })] }));
    s.setContext("/p|1|", "");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBeNull();
  });

  it("a complete latest analysis is", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBe(DRAFT);
  });
});

describe("history", () => {
  const three = [snap({ id: 3 }), snap({ id: 2, draft: "an earlier draft of the idea" }), snap({ id: 1, draft: "the first draft of the idea" })];

  it("view() clamps to the history, and stays at 0 with fewer than two", () => {
    const one = harness(tab()).s;
    one.view(1);
    expect(one.getState().viewing).toBe(0);
    const { s } = harness(tab({ history: three }));
    s.view(2);
    expect(s.getState().viewing).toBe(2);
    s.view(9);
    expect(s.getState().viewing).toBe(2);
    s.view(-1);
    expect(s.getState().viewing).toBe(0);
  });

  it("started goes back to the latest", () => {
    const { s, calls } = harness(tab({ history: three }));
    s.view(2);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(s.getState().viewing).toBe(2); // the gate may still skip: keep browsing until a run starts
    calls[0].emit({ type: "started" });
    expect(s.getState().viewing).toBe(0);
  });

  it("done refetches the history, which now starts with this analysis", async () => {
    const fresh = [snap({ id: 4, draft: `${DRAFT}, and alert after three` }), ...three];
    const getTab = vi.fn(async () => tab({ history: fresh }));
    const { s, calls } = harness(tab({ history: three }), getTab);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "started" });
    calls[0].emit({ type: "done", text: "## Intent\n- alert" });
    await settle();
    expect(getTab).toHaveBeenCalledWith("t-1");
    expect(s.getState().history.map((h) => h.id)).toEqual([4, 3, 2, 1]);
  });

  it("skips and errors never refetch", async () => {
    const getTab = vi.fn(async () => tab());
    const { s, calls } = harness(tab(), getTab);
    s.setDraft(`${DRAFT}, reworded`);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "skipped" });
    s.setDraft(`${DRAFT}, reworded again with more`);
    vi.advanceTimersByTime(1000);
    calls[1].emit({ type: "error", message: "Claude Code isn't installed." });
    await settle();
    expect(getTab).not.toHaveBeenCalled();
  });

  it("a slow history fetch never replaces a newer one", async () => {
    const resolvers: ((t: Tab) => void)[] = [];
    const getTab = vi.fn(() => new Promise<Tab>((resolve) => resolvers.push(resolve)));
    const { s, calls } = harness(tab({ history: three }), getTab);
    s.setDraft(`${DRAFT}, first change to it`);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "done", text: "## Intent\n- one" });
    s.setDraft(`${DRAFT}, second change to it`);
    vi.advanceTimersByTime(1000);
    calls[1].emit({ type: "done", text: "## Intent\n- two" });
    resolvers[1](tab({ history: [snap({ id: 5 }), snap({ id: 4 })] }));
    await settle();
    resolvers[0](tab({ history: [snap({ id: 4 })] }));
    await settle();
    expect(s.getState().history.map((h) => h.id)).toEqual([5, 4]);
  });
});

describe("requests", () => {
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

  it("dispose stops the pause and drops late events", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    s.setDraft(`${DRAFT}, more`);
    s.dispose();
    vi.advanceTimersByTime(5000);
    expect(calls).toHaveLength(1);
    calls[0].emit({ type: "done", text: "## Intent\n- late" });
    expect(s.getState().analysis.status).toBe("thinking");
  });
});
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `npx vitest run src/lib/tabSession.tabs.test.ts`
Expected: FAIL, 8 failed and 9 passed. Phase 1's session has no `view()`, no history refetch, no `contextDirty` rule and re-runs on every new context, so these fail: `a context change on a background tab waits for activate()` (`expected [ { …(5) } ] to deeply equal []`), `a background run reads the new context, …` (`expected false to be true`), `a tab with no saved analysis takes its first context without a run` and `a project that can't be read keeps the saved context and starts no run` (both `expected [ { …(5) } ] to deeply equal []`), `view() clamps …` and `started goes back to the latest` (`TypeError: … .view is not a function`), `done refetches the history, …` (`expected "spy" to be called with arguments: [ 't-1' ]`) and `a slow history fetch never replaces a newer one`.

- [ ] **Step 4: Replace `src/lib/tabSession.ts`**

```ts
// One tab's live analysis, kept outside React: a timer inside a hook would
// die when you switch tabs. Like a colleague reading over your shoulder, it
// waits until you pause, then comments; if you keep going, the last notes stay
// visible but marked out of date. The pause fires whether or not the tab is
// shown. Old responses are dropped by request id; the backend also aborts a
// tab's superseded run (its task slots are keyed by tab id).
import {
  applyEvent,
  initialAnalysis,
  lastAnalyzedAfter,
  previousDraftFor,
  startAnalysis,
  type AnalysisState,
  type Analyzed,
} from "./analysisState";
import { snapshotAnalysis } from "./history";
import { errorMessage, type Snapshot, type Tab, type UiEvent } from "./ipc";

export const MIN_CHARS = 24;
export const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

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
  /** Refetches the tab after `done`, for its history. Without it the history stays restore's copy. */
  getTab?: (tabId: string) => Promise<Tab>;
}

/** What React renders. A new object only when something changed (useSyncExternalStore). */
export interface TabView {
  draft: string;
  analysis: AnalysisState;
  tooShort: boolean;
  stale: boolean;
  contextDirty: boolean;
  /** 0 is the live state; k >= 1 is history[k]. */
  viewing: number;
  /** Newest first, at most 5 (sessions::HISTORY_LEN). */
  history: Snapshot[];
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
  private viewing = 0;
  private context = "";
  private instructions = "";
  private lastSent = "";
  private lastAnalyzed: Analyzed | null = null;
  private contextDirty = false;
  /** A session is created for the tab being shown; tabs.ts deactivates it when another tab is shown. */
  private active = true;
  private disposed = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private requestId = 0;
  private historyRequest = 0;

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
   * run; partial notes are never the gate's comparison point.
   */
  restore(tab: Tab): void {
    this.clearTimer();
    this.requestId++; // events from a run before the restore no longer apply
    const latest = tab.history[0];
    this.draft = tab.draft;
    this.history = tab.history;
    this.viewing = 0;
    this.analysis = latest ? snapshotAnalysis(latest) : initialAnalysis;
    this.context = latest?.context ?? "";
    this.lastSent = latest && normalize(latest.draft) === normalize(tab.draft) ? normalize(tab.draft) : "";
    this.lastAnalyzed = latest && !latest.partial ? { draft: latest.draft, context: latest.context } : null;
    this.contextDirty = false;
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
   * instructions. A new key on the shown tab re-reads the same draft with no
   * gate. On a hidden tab it waits for activate(), so a brief rebuilt under
   * ten open tabs re-runs only the one you're looking at. The first key a tab
   * with no saved analysis gets is not a change: it starts no run. A null key
   * means the project couldn't be read: the tab keeps the context its saved
   * notes ran with, so a missing folder never re-runs them, and the next pause
   * still reads the new instructions.
   */
  setContext(key: string | null, instructions: string): void {
    this.instructions = instructions;
    if (key === null || key === this.context || this.disposed) return;
    const first = this.context === "";
    this.context = key;
    // A tab with no saved analysis has no context until the window hands it one. That first
    // context isn't a change, so relaunching never starts a run by itself; a pending pause
    // reads it when it fires.
    if (first) return;
    if (this.active) this.resetAndSchedule();
    else this.contextDirty = true;
    this.emit();
  }

  activate(): void {
    if (this.active || this.disposed) return;
    this.active = true;
    if (!this.contextDirty) return;
    this.contextDirty = false;
    this.resetAndSchedule(); // schedule() skips drafts under MIN_CHARS
    this.emit();
  }

  /** Leaves any timer or run alone: a hidden tab keeps analysing. */
  deactivate(): void {
    this.active = false;
  }

  /** "Try again": always re-analyses, past the gate. */
  rerun(): void {
    if (!this.disposed) this.run(this.draft, true);
  }

  /** Clears the timer and drops late events. close_tab and move_tab cancel the backend run. */
  dispose(): void {
    this.disposed = true;
    this.clearTimer();
    this.requestId++;
    this.historyRequest++;
    this.listeners.clear();
  }

  /** Shows analysis `index` (0 = latest), clamped to the history; 0 with fewer than two. */
  view(index: number): void {
    const max = this.history.length < 2 ? 0 : this.history.length - 1;
    const next = Number.isFinite(index) ? Math.min(Math.max(Math.trunc(index), 0), max) : 0;
    if (next === this.viewing) return;
    this.viewing = next;
    this.emit();
  }

  private resetAndSchedule() {
    this.lastSent = "";
    this.lastAnalyzed = null;
    this.schedule();
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
    this.contextDirty = false; // this run reads the current context
    this.analysis = startAnalysis(this.analysis, text);
    this.emit();
    const mine = () => id === this.requestId;
    this.deps
      .analyze(this.id, text, previousDraftFor(this.lastAnalyzed, rerun, ctx), this.instructions, ctx, (ev) => {
        if (!mine()) return;
        this.lastAnalyzed = lastAnalyzedAfter(this.lastAnalyzed, ev, text, ctx);
        if (ev.type === "started") this.viewing = 0; // live streaming always shows the latest
        this.analysis = applyEvent(this.analysis, ev);
        this.emit();
        if (ev.type === "done") this.refreshHistory();
      })
      .catch((e) => {
        if (!mine()) return;
        this.analysis = { ...this.analysis, status: "error", error: errorMessage(e) };
        this.emit();
      });
  }

  /** `analyze` records the snapshot before it sends `done`, so this fetch includes it. */
  private refreshHistory() {
    const getTab = this.deps.getTab;
    if (!getTab) return;
    const n = ++this.historyRequest;
    getTab(this.id).then(
      (tab) => {
        if (n !== this.historyRequest) return; // a newer fetch, or dispose, won
        this.history = tab.history;
        this.viewing = Math.min(this.viewing, Math.max(tab.history.length - 1, 0));
        this.emit();
      },
      () => {}, // the notes on screen are right; the next `done` refetches
    );
  }

  private emit() {
    const norm = normalize(this.draft);
    this.state = {
      draft: this.draft,
      analysis: this.analysis,
      tooShort: norm.length < MIN_CHARS,
      stale: this.analysis.status !== "idle" && normalize(this.analysis.analyzedDraft) !== norm,
      contextDirty: this.contextDirty,
      viewing: this.viewing,
      history: this.history,
    };
    this.listeners.forEach((l) => l());
  }
}
```

If Step 1 found other exports that some file imports, add them back now, unchanged.

- [ ] **Step 5: Run every `TabSession` test, the suite and the typecheck**

Run: `npx vitest run src/lib/tabSession && npm test && npm run typecheck`
Expected: the 17 new tests PASS, Phase 1's 16 tests in `tabSession.test.ts` and 1 in `useTabSession.test.ts` still pass unchanged, and the whole suite passes. If a Phase 1 test fails, the replacement drifted from Phase 1's behaviour: fix the replacement, never the Phase 1 test. No type errors (Phase 1's `App.tsx` still compiles: the API only grew).

- [ ] **Step 6: Commit**

```bash
git add src/lib/tabSession.ts src/lib/tabSession.tabs.test.ts
git commit -m "feat(ui): TabSession browses its history, refetches it after done, and defers hidden tabs' re-runs

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 5: Keyboard actions

**Files:**
- Create: `src/lib/actions.ts`
- Test: `src/lib/actions.test.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (`src/lib/actions.ts`):
  ```ts
  export type ActionId = "new-tab" | "new-window" | "open-project" | "reopen-tab" | "close-tab" | "close-window"
    | "next-tab" | "prev-tab" | "toggle-sidebar" | "settings";
  export function keyToAction(e: KeyboardEvent): ActionId | null;   // ⌘/Ctrl bindings from the spec
  export function useActions(handlers: Partial<Record<ActionId, () => void>>): void;  // window keydown listener
  // Extra, for tests and Phase 3: the same rules with the platform given.
  export type KeyPress = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"> & { isComposing?: boolean };
  export function keyToActionFor(e: KeyPress, mac: boolean): ActionId | null;
  ```
  `useActions` runs a handler only for actions it was given, calls `preventDefault()` only then, and ignores key presses while a modal `<dialog>` (Settings) is open. Task 9 binds every action except `new-window` and `close-window`; Phase 3 routes its `menu` events into the same handlers.

- [ ] **Step 1: Write the failing test**

Create `src/lib/actions.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { keyToActionFor, type KeyPress } from "./actions";

const press = (key: string, mods: Partial<Omit<KeyPress, "key">> = {}): KeyPress => ({
  key,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe("keyToActionFor", () => {
  it("maps the ⌘ shortcuts on macOS", () => {
    const cmd = (key: string, shiftKey = false) => keyToActionFor(press(key, { metaKey: true, shiftKey }), true);
    expect(cmd("t")).toBe("new-tab");
    expect(cmd("T", true)).toBe("reopen-tab");
    expect(cmd("n")).toBe("new-window");
    expect(cmd("o")).toBe("open-project");
    expect(cmd("w")).toBe("close-tab");
    expect(cmd("W", true)).toBe("close-window");
    expect(cmd("\\")).toBe("toggle-sidebar");
    expect(cmd(",")).toBe("settings");
    expect(cmd("x")).toBeNull();
  });

  it("uses Ctrl on Windows and Linux, and leaves ⌘ alone there", () => {
    expect(keyToActionFor(press("t", { ctrlKey: true }), false)).toBe("new-tab");
    expect(keyToActionFor(press("t", { metaKey: true }), false)).toBeNull();
  });

  it("keeps the text editing keys of macOS: Ctrl+T transposes and Ctrl+O opens a line", () => {
    expect(keyToActionFor(press("t", { ctrlKey: true }), true)).toBeNull();
    expect(keyToActionFor(press("o", { ctrlKey: true }), true)).toBeNull();
  });

  it("⌃Tab and ⌃⇧Tab step through tabs on every platform", () => {
    for (const mac of [true, false]) {
      expect(keyToActionFor(press("Tab", { ctrlKey: true }), mac)).toBe("next-tab");
      expect(keyToActionFor(press("Tab", { ctrlKey: true, shiftKey: true }), mac)).toBe("prev-tab");
    }
    expect(keyToActionFor(press("Tab"), true)).toBeNull();
    expect(keyToActionFor(press("Tab", { metaKey: true, ctrlKey: true }), true)).toBeNull();
  });

  it("ignores Alt combinations, unmodified keys and IME composition", () => {
    expect(keyToActionFor(press("t", { metaKey: true, altKey: true }), true)).toBeNull();
    expect(keyToActionFor(press("t"), true)).toBeNull();
    expect(keyToActionFor(press("t", { metaKey: true, isComposing: true }), true)).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/lib/actions.test.ts`
Expected: FAIL with `Failed to resolve import "./actions"`.

- [ ] **Step 3: Create `src/lib/actions.ts`**

```ts
// Keyboard actions for tabs, handled in the webview for now. Phase 3's app
// menu emits the same ids as `menu` events and takes over the keys it owns.
import { useEffect, useRef } from "react";

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

export type KeyPress = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"> & { isComposing?: boolean };

const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const PLAIN: Record<string, ActionId> = {
  t: "new-tab",
  n: "new-window",
  o: "open-project",
  w: "close-tab",
  "\\": "toggle-sidebar",
  ",": "settings",
};
const SHIFTED: Record<string, ActionId> = { t: "reopen-tab", w: "close-window" };

/**
 * ⌘ on macOS, Ctrl elsewhere. ⌃Tab is Ctrl everywhere. On macOS, Ctrl+letter
 * keeps its text-editing meaning (⌃T transposes, ⌃O opens a line).
 */
export function keyToActionFor(e: KeyPress, mac: boolean): ActionId | null {
  if (e.isComposing || e.altKey) return null;
  if (e.key === "Tab") return e.ctrlKey && !e.metaKey ? (e.shiftKey ? "prev-tab" : "next-tab") : null;
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod) return null;
  const key = e.key.toLowerCase();
  return (e.shiftKey ? SHIFTED[key] : PLAIN[key]) ?? null;
}

export function keyToAction(e: KeyboardEvent): ActionId | null {
  return keyToActionFor(e, IS_MAC);
}

/**
 * Runs the handler for each recognised key press. Actions without a handler
 * are left to the webview, and nothing fires while a modal dialog (Settings)
 * is open, so ⌘W never closes a tab behind it.
 */
export function useActions(handlers: Partial<Record<ActionId, () => void>>): void {
  const current = useRef(handlers);
  current.current = handlers;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = keyToAction(e);
      const run = action ? current.current[action] : undefined;
      if (!run || document.querySelector("dialog[open]")) return;
      e.preventDefault();
      run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
```

- [ ] **Step 4: Run the tests and the typecheck**

Run: `npx vitest run src/lib/actions.test.ts && npm test && npm run typecheck`
Expected: the 5 new tests PASS, the suite passes, and there are no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/actions.ts src/lib/actions.test.ts
git commit -m "feat(ui): keyboard actions for tabs, Ctrl on Windows and Linux

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 6: The window's tab store (`src/lib/tabs.ts`)

**Files:**
- Create: `src/lib/tabs.ts`
- Test: `src/lib/tabs.test.ts`

**Interfaces:**
- Consumes:
  - Phase 1, `src/lib/ipc.ts` (the types `Tab`, `TabSummary`, `WindowView` and `LegacyDraft` are in "What Phase 1 leaves behind"):
    ```ts
    api.listWorkspace(window: string): Promise<WindowView>
    api.getTab(tabId: string): Promise<Tab>
    api.createTab(window: string, root: string | null): Promise<Tab>       // appended to the window, made active
    api.saveDraft(tabId: string, draft: string): Promise<void>
    api.renameTab(tabId: string, title: string | null): Promise<void>
    api.activateTab(window: string, tabId: string): Promise<void>
    api.closeTab(tabId: string): Promise<void>                             // cancels the run; backend active moves to the stored-order neighbour
    api.reopenTab(tabId: string, window: string): Promise<void>            // appended, made active
    api.deleteTab(tabId: string): Promise<void>
    api.importLegacyDrafts(window: string, drafts: LegacyDraft[], activeRoot: string | null): Promise<void>
    api.analyze(tabId: string, draft: string, previousDraft: string | null, instructions: string, context: string,
                onEvent: (e: UiEvent) => void): Promise<void>
    errorMessage(e: unknown): string
    ```
  - Phase 1: `currentWindowId(): string` (`src/lib/windowId.ts`); `onWorkspaceChanged(fn: (e: { windows: string[] }) => void): Promise<UnlistenFn>` (`src/lib/events.ts`); `browserStorage()` and `importLegacy(windowId, storage, importDrafts): Promise<string | null>` (`src/lib/legacyDrafts.ts`, signatures in "What Phase 1 leaves behind").
  - `createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void): { schedule(key: string, value: T): void; flush(): Promise<void> }` (`src/lib/saver.ts`, unchanged). A pending save remembers its key; scheduling another key flushes the pending one first.
  - `groupTabs`, `visualOrder`, `stepTab`, `afterClose` (Task 2, signatures in its Interfaces).
  - `TabSession`, `TabSessionDeps` with the optional `getTab` (Task 4).
- Produces (`src/lib/tabs.ts`):
  ```ts
  export interface WorkspaceState {
    windowId: string; view: WindowView | null; activeId: string | null; error: string | null;
    saveError: string | null;          // extra: the last draft save failed
  }
  export function useWorkspace(): WorkspaceState;
  export function sessionFor(tabId: string): TabSession | undefined;
  export const tabs: {
    load(): Promise<void>; switchTo(tabId: string): Promise<void>; newTab(root: string | null): Promise<void>;
    close(tabId: string): Promise<void>; reopen(tabId: string): Promise<void>; remove(tabId: string): Promise<void>;
    rename(tabId: string, title: string | null): Promise<void>; next(dir: 1 | -1): void;
    // extras
    refresh(): Promise<void>;                                              // refetch list_workspace
    setContext(root: string | null, key: string | null, instructions: string): void;  // applies to this root's sessions, now and later
    setDebounce(ms: number): void;
  };
  // extra, for tests
  export function workspaceState(): WorkspaceState;
  ```
  Behaviour: `load()` runs once per window, imports the legacy `localStorage` drafts when `fresh` through Phase 1's `importLegacy` (keys removed only after the import succeeds), and refetches on every `workspace-changed`. Switching, creating and closing flush the pending draft save first. Closing the shown tab first shows the next tab in visual order, else the previous, else none, and only then closes it, so the backend's stored-order choice of active tab never flashes on screen.

- [ ] **Step 1: Write the failing test**

Create `src/lib/tabs.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Tab, TabSummary, WindowView } from "./ipc";

const api = vi.hoisted(() => ({
  listWorkspace: vi.fn(),
  getTab: vi.fn(),
  createTab: vi.fn(),
  saveDraft: vi.fn(),
  renameTab: vi.fn(),
  activateTab: vi.fn(),
  closeTab: vi.fn(),
  reopenTab: vi.fn(),
  deleteTab: vi.fn(),
  importLegacyDrafts: vi.fn(),
  analyze: vi.fn(),
}));
vi.mock("./ipc", () => ({
  api,
  errorMessage: (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e)),
}));
vi.mock("./windowId", () => ({ currentWindowId: () => "w-1" }));
vi.mock("./events", () => ({ onWorkspaceChanged: vi.fn() }));

const summary = (id: string, root: string | null): TabSummary => ({ id, root, title: null, firstLine: "", updatedAt: 0, closedAt: null });
const full = (id: string, root: string | null, draft = ""): Tab => ({
  id,
  root,
  title: null,
  createdAt: 0,
  updatedAt: 0,
  closedAt: null,
  draft,
  nextSnapshot: 1,
  history: [],
});
const view = (tabs: TabSummary[], active: Tab | null, over: Partial<WindowView> = {}): WindowView => ({
  window: "w-1",
  tabs,
  active,
  closed: [],
  fresh: false,
  unreadable: [],
  ...over,
});

// Stored order a1, b1, a2; the sidebar shows a1, a2 | b1.
const stored = [summary("a1", "/a"), summary("b1", "/b"), summary("a2", "/a")];

/** A fresh store per test: the module keeps one window's state. */
async function loadStore(first: WindowView) {
  vi.resetModules();
  api.listWorkspace.mockResolvedValue(first);
  const mod = await import("./tabs");
  await mod.tabs.load();
  return mod;
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.unstubAllGlobals();
  for (const f of [api.saveDraft, api.renameTab, api.activateTab, api.closeTab, api.reopenTab, api.deleteTab, api.importLegacyDrafts, api.analyze]) {
    f.mockResolvedValue(undefined);
  }
});

describe("tabs store", () => {
  it("closing the active tab shows the next one in the sidebar's order", async () => {
    const { tabs, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    expect(workspaceState().activeId).toBe("a1");
    api.getTab.mockResolvedValue(full("a2", "/a"));
    api.listWorkspace.mockResolvedValue(view(stored.slice(1), full("a2", "/a")));
    await tabs.close("a1");
    expect(api.closeTab).toHaveBeenCalledWith("a1");
    expect(api.getTab).toHaveBeenCalledWith("a2"); // the stored order would pick b1
    expect(api.activateTab).toHaveBeenCalledWith("w-1", "a2");
    // a2 is active before a1 closes, so the backend never moves `active` to b1.
    expect(api.activateTab.mock.invocationCallOrder[0]).toBeLessThan(api.closeTab.mock.invocationCallOrder[0]);
    expect(workspaceState().activeId).toBe("a2");
  });

  it("closing the last tab leaves the window empty", async () => {
    const { tabs, workspaceState } = await loadStore(view([summary("a1", "/a")], full("a1", "/a")));
    api.listWorkspace.mockResolvedValue(view([], null));
    await tabs.close("a1");
    expect(workspaceState().activeId).toBeNull();
    expect(workspaceState().view?.tabs).toEqual([]);
  });

  it("switching flushes the pending save of the tab you left", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    expect(api.saveDraft).not.toHaveBeenCalled();
    api.getTab.mockResolvedValue(full("b1", "/b"));
    await tabs.switchTo("b1");
    expect(api.saveDraft).toHaveBeenCalledWith("a1", "typed in a1");
    expect(api.saveDraft.mock.invocationCallOrder[0]).toBeLessThan(api.getTab.mock.invocationCallOrder[0]);
  });

  it("next() follows the sidebar's order", async () => {
    const { tabs, workspaceState } = await loadStore(view(stored, full("a2", "/a")));
    api.getTab.mockResolvedValue(full("b1", "/b"));
    tabs.next(1); // the stored order would wrap to a1
    await vi.waitFor(() => expect(workspaceState().activeId).toBe("b1"));
  });

  it("a project's context applies to its tabs, including ones shown later", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    tabs.setDebounce(0);
    tabs.setContext("/a", "/a|1|Focus.", "Focus.");
    api.getTab.mockResolvedValue(full("a2", "/a"));
    await tabs.switchTo("a2");
    sessionFor("a2")!.setDraft("Retry failed checkout payments with backoff");
    await vi.waitFor(() => expect(api.analyze).toHaveBeenCalled());
    expect(api.analyze.mock.calls[0].slice(0, 5)).toEqual([
      "a2",
      "Retry failed checkout payments with backoff",
      null,
      "Focus.",
      "/a|1|Focus.",
    ]);
  });
});

describe("legacy drafts", () => {
  const storage = (entries: Record<string, string>) => {
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
  };

  // Collecting and removing the keys is Phase 1's legacyDrafts.test.ts; these pin the store's use of it.
  it("imports them once on a fresh workspace, then removes the keys", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea", "livebrainstorm.lastProject": "/a" });
    vi.stubGlobal("localStorage", s);
    api.listWorkspace
      .mockResolvedValueOnce(view([], null, { fresh: true }))
      .mockResolvedValue(view([summary("t-a", "/a")], full("t-a", "/a", "a idea")));
    vi.resetModules();
    const { tabs, workspaceState } = await import("./tabs");
    await tabs.load();
    expect(api.importLegacyDrafts).toHaveBeenCalledWith("w-1", [{ root: "/a", draft: "a idea" }], "/a");
    expect(s.m.size).toBe(0);
    expect(workspaceState().activeId).toBe("t-a");
  });

  it("keeps the keys when the import fails", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea" });
    vi.stubGlobal("localStorage", s);
    api.importLegacyDrafts.mockRejectedValue(new Error("disk full"));
    api.listWorkspace.mockResolvedValue(view([], null, { fresh: true }));
    vi.resetModules();
    const { tabs, workspaceState } = await import("./tabs");
    await tabs.load();
    expect(s.m.has("livebrainstorm.draft./a")).toBe(true);
    expect(workspaceState().error).toBe("Couldn't bring over your earlier drafts: disk full");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/lib/tabs.test.ts`
Expected: FAIL, all 7 tests, each with `Error: Cannot find module './tabs'` (the test imports the store per test, after `vi.resetModules()`).

- [ ] **Step 3: Create `src/lib/tabs.ts`**

```ts
// This window's tabs: the workspace view from the backend, which tab is
// shown, and one TabSession per tab shown since launch. Sessions live here,
// outside React, so a hidden tab keeps its pause timer and its run. A
// per-window module store read through useSyncExternalStore, like downloads.ts.
import { useSyncExternalStore } from "react";
import { onWorkspaceChanged } from "./events";
import { api, errorMessage, type Tab, type WindowView } from "./ipc";
import { browserStorage, importLegacy } from "./legacyDrafts";
import { createSaver } from "./saver";
import { afterClose, groupTabs, stepTab, visualOrder } from "./tabList";
import { TabSession, type TabSessionDeps } from "./tabSession";
import { currentWindowId } from "./windowId";

export interface WorkspaceState {
  windowId: string;
  view: WindowView | null;
  activeId: string | null;
  /** The last tab action that failed, shown in the sidebar footer. */
  error: string | null;
  /** The last draft save failed; cleared when a later save succeeds. */
  saveError: string | null;
}

let state: WorkspaceState = { windowId: "", view: null, activeId: null, error: null, saveError: null };
const listeners = new Set<() => void>();
const sessions = new Map<string, TabSession>();
/** Each project's analysis context by root ("" = no project), for its sessions now and later. */
const contexts = new Map<string, { key: string | null; instructions: string }>();
let debounce = 1200;
let loaded = false;
let switching = 0;
let refreshing = 0;

function set(patch: Partial<WorkspaceState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

// Draft edits save 500 ms after typing stops, each under the tab it was typed in.
const saver = createSaver<string>(
  (id, draft) =>
    api.saveDraft(id, draft).then(() => {
      if (state.saveError) set({ saveError: null });
    }),
  500,
  (e) => set({ saveError: `Couldn't save your draft: ${errorMessage(e)}` }),
);

const deps: TabSessionDeps = {
  analyze: (tabId, draft, previousDraft, instructions, context, onEvent) =>
    api.analyze(tabId, draft, previousDraft, instructions, context, onEvent),
  saveDraft: (tabId, draft) => saver.schedule(tabId, draft),
  debounceMs: () => debounce,
  getTab: (tabId) => api.getTab(tabId),
};

const rootKey = (root: string | null) => root ?? "";

/** The tab's session, created on first show with its project's context, if known yet. */
function sessionOf(tab: Tab): TabSession {
  let s = sessions.get(tab.id);
  if (!s) {
    s = new TabSession(tab, deps);
    sessions.set(tab.id, s);
    const c = contexts.get(rootKey(tab.root));
    if (c) s.setContext(c.key, c.instructions);
  }
  return s;
}

function drop(id: string) {
  sessions.get(id)?.dispose();
  sessions.delete(id);
}

/** Shows `s` and hides the tab shown before; the hidden one keeps any timer or run. */
function show(s: TabSession | null) {
  const prev = state.activeId;
  if (prev !== null && prev !== s?.id) sessions.get(prev)?.deactivate();
  s?.activate();
  set({ activeId: s?.id ?? null });
}

const order = () => visualOrder(groupTabs(state.view?.tabs ?? []));

async function refresh(): Promise<void> {
  const n = ++refreshing;
  const view = await api.listWorkspace(state.windowId);
  if (n !== refreshing) return; // a newer refresh is on its way
  for (const id of [...sessions.keys()]) if (!view.tabs.some((t) => t.id === id)) drop(id);
  set({ view });
  // Nothing shown yet, or the shown tab left this window: show what the backend says is active.
  const shown = state.activeId;
  if (shown === null || !view.tabs.some((t) => t.id === shown)) {
    const next = view.active ? sessionOf(view.active) : null;
    if ((next?.id ?? null) !== shown) show(next);
  }
}

async function load(): Promise<void> {
  if (loaded) return; // StrictMode runs effects twice; there is one store per window
  loaded = true;
  set({ windowId: currentWindowId() });
  // Closing the window doesn't unmount React, so flush when the page goes away.
  if (typeof window !== "undefined") window.addEventListener("pagehide", () => void saver.flush());
  // Any session change, in this window or another, refetches this window's list and the shared closed list.
  void onWorkspaceChanged(() => void refresh().catch(() => {}));
  try {
    const first = await api.listWorkspace(state.windowId);
    if (first.fresh) {
      // The first launch of the tabs build: the drafts the old build kept in localStorage become tabs.
      // The keys go only once the import has succeeded, so a failure loses nothing.
      const failed = await importLegacy(state.windowId, browserStorage(), api.importLegacyDrafts);
      if (failed) set({ error: failed });
    }
    await refresh();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

async function switchTo(tabId: string): Promise<void> {
  if (tabId === state.activeId) return;
  const n = ++switching;
  set({ error: null });
  await saver.flush(); // the tab you leave keeps its last edit
  try {
    const s = sessions.get(tabId) ?? sessionOf(await api.getTab(tabId));
    if (n !== switching) {
      if (s.id !== state.activeId) s.deactivate(); // a later switch won
      return;
    }
    show(s);
    await api.activateTab(state.windowId, tabId);
  } catch (e) {
    if (n === switching) set({ error: errorMessage(e) });
  }
}

async function newTab(root: string | null): Promise<void> {
  ++switching;
  set({ error: null });
  await saver.flush();
  try {
    const tab = await api.createTab(state.windowId, root); // appended to the window and made active
    show(sessionOf(tab));
    await refresh();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

async function close(tabId: string): Promise<void> {
  const wasShown = tabId === state.activeId;
  const next = wasShown ? afterClose(order(), tabId) : null;
  set({ error: null });
  // Show the next tab first. Closing the shown tab makes the backend activate its neighbour in the
  // *stored* order, and the workspace-changed refresh would show that tab for a moment.
  if (next) await switchTo(next);
  await saver.flush();
  try {
    await api.closeTab(tabId); // also cancels its run
  } catch (e) {
    set({ error: errorMessage(e) });
    return;
  }
  drop(tabId);
  if (wasShown && !next) show(null); // the window stays open and empty
  await refresh().catch((e) => set({ error: errorMessage(e) }));
}

async function reopen(tabId: string): Promise<void> {
  set({ error: null });
  try {
    await api.reopenTab(tabId, state.windowId);
    await refresh();
    await switchTo(tabId);
  } catch (e) {
    set({ error: errorMessage(e) }); // e.g. "That tab is already open in another window."
  }
}

async function remove(tabId: string): Promise<void> {
  set({ error: null });
  try {
    await api.deleteTab(tabId);
    await refresh();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

async function rename(tabId: string, title: string | null): Promise<void> {
  set({ error: null });
  try {
    await api.renameTab(tabId, title?.trim() ? title.trim() : null); // null: back to the automatic title
    await refresh();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

/** ⌃Tab / ⌃⇧Tab: the sidebar's order, wrapping. */
function next(dir: 1 | -1): void {
  const id = stepTab(order(), state.activeId, dir);
  if (id !== null && id !== state.activeId) void switchTo(id);
}

/** A project's context, from projects.ts once its summary and instructions have loaded (null: it couldn't be read). */
function setContext(root: string | null, key: string | null, instructions: string): void {
  contexts.set(rootKey(root), { key, instructions });
  for (const s of sessions.values()) if (s.root === root) s.setContext(key, instructions);
}

export const tabs = {
  load,
  switchTo,
  newTab,
  close,
  reopen,
  remove,
  rename,
  next,
  refresh,
  setContext,
  setDebounce: (ms: number) => {
    debounce = ms;
  },
  // Phase 3 (Task 4) adds `flush()` here, returning `saver.flush()`; this phase flushes inside the store.
};

export function sessionFor(tabId: string): TabSession | undefined {
  return sessions.get(tabId);
}

export function workspaceState(): WorkspaceState {
  return state;
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export function useWorkspace(): WorkspaceState {
  return useSyncExternalStore(subscribe, workspaceState);
}
```

- [ ] **Step 4: Run the tests and the typecheck**

Run: `npx vitest run src/lib/tabs.test.ts && npm test && npm run typecheck`
Expected: the 7 new tests PASS, the suite passes, and there are no type errors.

- [ ] **Step 5: Commit**

```bash
git add src/lib/tabs.ts src/lib/tabs.test.ts
git commit -m "feat(ui): per-window tab store: sessions per tab, visual-order switching, legacy import

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 7: The window's project store (`src/lib/projects.ts`)

Today `App.tsx` holds one project, one set of instructions and one brief build. With tabs on several projects, each window caches these per root: summaries from `project_summary` (refreshed on `brief-status: done`), instructions fetched on first use, and a brief build per root. This store also computes each project's analysis context and hands it to `tabs.setContext`, but only once both the summary and the instructions have loaded (spec, "restore", "Wait for both").

**Files:**
- Create: `src/lib/projects.ts`
- Test: `src/lib/projects.test.ts`

**Interfaces:**
- Consumes:
  - Phase 1:
    ```ts
    api.projectSummary(root: string): Promise<ProjectSummary>   // scans only a root that isn't loaded; a missing folder rejects with its Precondition message
    api.openProject(path: string): Promise<ProjectSummary>      // scans and replaces the registry entry
    api.buildBrief(root: string, onEvent: (e: UiEvent) => void): Promise<void>  // a second build of a running root resolves with no events
    onBriefStatus(fn: (e: { root: string; state: "building" | "done" | "error"; message: string | null }) => void): Promise<UnlistenFn>  // src/lib/events.ts
    ```
  - Unchanged, `src/lib/ipc.ts`:
    ```ts
    api.getInstructions(root: string): Promise<InstructionsView>          // "" = no project
    api.saveProjectInstructions(root: string, instructions: ProjectInstructions): Promise<void>
    api.saveTemplates(templates: Template[]): Promise<void>
    api.restoreStarterTemplates(): Promise<Template[]>
    interface Template { id: string; name: string; text: string }
    interface ProjectInstructions { default: string; current: string; templateId: string | null }
    interface InstructionsView { templates: Template[]; project: ProjectInstructions }
    interface StoredBrief { root: string; fingerprint: string; text: string; createdAt: number; source: string }
    interface ProjectSummary { name: string; root: string; indexedFiles: number; totalFiles: number;
                               briefStatus: "missing" | "fresh" | "stale"; brief: StoredBrief | null }
    ```
  - Unchanged helpers: `EMPTY_INSTRUCTIONS: ProjectInstructions` and `analysisContextKey(root: string | undefined, briefCreatedAt: number | undefined, instructions: string): string`, which is `` `${root ?? ""}|${briefCreatedAt ?? 0}|${instructions.trim()}` `` (`src/lib/instructions.ts`); `activityLine(label: string, lane: number): string` (`src/lib/brief.ts`); `createSaver` (`src/lib/saver.ts`, signature in Task 6's Interfaces).
  - `tabs.setContext(root: string | null, key: string | null, instructions: string): void` (Task 6). A null key tells the root's sessions to keep their context and take only the instructions.
- Produces (`src/lib/projects.ts`):
  ```ts
  export interface BriefRun { building: boolean; live: string; activity: string[]; error: string | null }
  export const IDLE_BRIEF: BriefRun;
  export interface ProjectsState {
    summaries: Record<string, ProjectSummary>;
    summaryErrors: Record<string, string>;          // a project that couldn't be read: the message to show
    instructions: Record<string, ProjectInstructions>;  // "" = no project
    templates: Template[];
    instructionsError: string | null;
    briefs: Record<string, BriefRun>;
  }
  export const projects: {
    start(): void;                                   // once per window: templates, no-project instructions, brief-status
    ensure(root: string | null): void;               // load what a shown tab's project needs
    openFolder(path: string): Promise<string>;       // scans; returns the root
    refreshSummary(root: string): Promise<void>;
    reloadInstructions(root: string | null): Promise<void>;   // Phase 3's instructions-changed calls this; "" and null both mean no project
    changeInstructions(root: string | null, next: ProjectInstructions): void;
    changeTemplates(next: Template[]): Promise<void>;
    restoreStarters(): Promise<void>;
    buildBrief(root: string): void;
    briefStatus(e: { root: string; state: string; message?: string | null }): void;
    flush(): Promise<void>;
  };
  export function useProjects(): ProjectsState;
  export function projectsState(): ProjectsState;    // for tests
  ```

- [ ] **Step 1: Write the failing test**

Create `src/lib/projects.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstructionsView, ProjectSummary } from "./ipc";

const api = vi.hoisted(() => ({
  projectSummary: vi.fn(),
  getInstructions: vi.fn(),
  saveProjectInstructions: vi.fn(),
  saveTemplates: vi.fn(),
  restoreStarterTemplates: vi.fn(),
  openProject: vi.fn(),
  buildBrief: vi.fn(),
}));
const setContext = vi.hoisted(() => vi.fn());
vi.mock("./ipc", () => ({
  api,
  errorMessage: (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e)),
}));
vi.mock("./tabs", () => ({ tabs: { setContext } }));
vi.mock("./events", () => ({ onBriefStatus: vi.fn() }));

const summary = (root: string, createdAt: number | null): ProjectSummary => ({
  name: "p",
  root,
  indexedFiles: 1,
  totalFiles: 1,
  briefStatus: createdAt ? "fresh" : "missing",
  brief: createdAt ? { root, fingerprint: "f", text: "brief", createdAt, source: "test" } : null,
});
const instructionsView = (current: string): InstructionsView => ({ templates: [], project: { default: "", current, templateId: null } });

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

async function fresh() {
  vi.resetModules();
  return import("./projects");
}

beforeEach(() => {
  vi.resetAllMocks();
  api.saveProjectInstructions.mockResolvedValue(undefined);
});

describe("project contexts", () => {
  it("a project's context reaches its tabs only after both its summary and its instructions load", async () => {
    const { projects } = await fresh();
    const s = deferred<ProjectSummary>();
    const i = deferred<InstructionsView>();
    api.projectSummary.mockReturnValue(s.promise);
    api.getInstructions.mockReturnValue(i.promise);
    projects.ensure("/p");
    i.resolve(instructionsView("Focus."));
    await settle();
    expect(setContext).not.toHaveBeenCalled(); // "/p|0|Focus." would not match the saved notes' context
    s.resolve(summary("/p", 7));
    await settle();
    expect(setContext).toHaveBeenCalledTimes(1);
    expect(setContext).toHaveBeenCalledWith("/p", "/p|7|Focus.", "Focus.");
  });

  it("tabs with no project need only the instructions", async () => {
    const { projects } = await fresh();
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure(null);
    await settle();
    expect(api.projectSummary).not.toHaveBeenCalled();
    expect(setContext).toHaveBeenCalledWith(null, "|0|", "");
  });

  it("a project that can't be read hands its tabs only the instructions, and is retried when shown again", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockRejectedValueOnce("That folder no longer exists. Open it again from its new place.");
    api.projectSummary.mockResolvedValue(summary("/gone", 3));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/gone");
    await settle();
    expect(projectsState().summaryErrors["/gone"]).toBe("That folder no longer exists. Open it again from its new place.");
    expect(setContext).toHaveBeenLastCalledWith("/gone", null, ""); // keep the saved notes' context: no re-run
    projects.ensure("/gone");
    await settle();
    expect(projectsState().summaryErrors["/gone"]).toBeUndefined();
    expect(setContext).toHaveBeenLastCalledWith("/gone", "/gone|3|", "");
  });

  it("a finished brief build refreshes the summary, so the project's tabs get the new context", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValueOnce(summary("/p", 1)).mockResolvedValueOnce(summary("/p", 2));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    projects.briefStatus({ root: "/p", state: "building", message: null });
    expect(projectsState().briefs["/p"].building).toBe(true);
    projects.briefStatus({ root: "/p", state: "done", message: null });
    await settle();
    expect(projectsState().briefs["/p"].building).toBe(false);
    expect(setContext).toHaveBeenLastCalledWith("/p", "/p|2|", "");
  });

  it("editing instructions updates the project's context and saves under that project", async () => {
    const { projects } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    const next = { default: "", current: "Focus.", templateId: null };
    projects.changeInstructions("/p", next);
    expect(setContext).toHaveBeenLastCalledWith("/p", "/p|1|Focus.", "Focus.");
    await projects.flush();
    expect(api.saveProjectInstructions).toHaveBeenCalledWith("/p", next);
  });

  it('reloading under "" (how events name no project) updates the tabs with no project', async () => {
    const { projects } = await fresh();
    api.getInstructions.mockResolvedValueOnce(instructionsView("")).mockResolvedValueOnce(instructionsView("Be brief."));
    projects.ensure(null);
    await settle();
    await projects.reloadInstructions("");
    expect(api.getInstructions).toHaveBeenLastCalledWith("");
    expect(setContext).toHaveBeenLastCalledWith(null, "|0|Be brief.", "Be brief.");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/lib/projects.test.ts`
Expected: FAIL, all 6 tests, each with `Error: Cannot find module './projects'` (the test imports the store per test, after `vi.resetModules()`).

- [ ] **Step 3: Create `src/lib/projects.ts`**

```ts
// Each project's data in this window, by root: its summary (the brief and
// whether it's current), its session instructions, and its brief build. Every
// tab of a project shares them. A project's analysis context comes from both
// the summary and the instructions, so it reaches tabs.ts only once both have
// loaded: a key computed earlier ("root|0|") would differ from the saved notes'
// and start a spurious run at launch.
import { useSyncExternalStore } from "react";
import { activityLine } from "./brief";
import { onBriefStatus } from "./events";
import { api, errorMessage, type ProjectInstructions, type ProjectSummary, type Template } from "./ipc";
import { EMPTY_INSTRUCTIONS, analysisContextKey } from "./instructions";
import { createSaver } from "./saver";
import { tabs } from "./tabs";

export interface BriefRun {
  building: boolean;
  /** The brief as it streams in; only the window that started the build receives it. */
  live: string;
  activity: string[];
  error: string | null;
}

export const IDLE_BRIEF: BriefRun = { building: false, live: "", activity: [], error: null };

export interface ProjectsState {
  summaries: Record<string, ProjectSummary>;
  /** A project that couldn't be read (moved, deleted, unreadable): the message to show. */
  summaryErrors: Record<string, string>;
  /** Keyed by root; "" is tabs with no project. */
  instructions: Record<string, ProjectInstructions>;
  templates: Template[];
  instructionsError: string | null;
  briefs: Record<string, BriefRun>;
}

let state: ProjectsState = {
  summaries: {},
  summaryErrors: {},
  instructions: {},
  templates: [],
  instructionsError: null,
  briefs: {},
};
const listeners = new Set<() => void>();
const loading = new Set<string>();
let started = false;

function set(patch: Partial<ProjectsState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

const key = (root: string | null) => root ?? "";

function without<T>(record: Record<string, T>, k: string): Record<string, T> {
  const next = { ...record };
  delete next[k];
  return next;
}

// Instruction edits save a moment after typing stops, each under the project it was typed in.
const saver = createSaver<ProjectInstructions>(
  (root, p) => api.saveProjectInstructions(root, p),
  500,
  (e) => set({ instructionsError: `Couldn't save instructions: ${errorMessage(e)}` }),
);

/**
 * Hands the project's context to its tabs once both halves have loaded. A
 * project that couldn't be read hands over only the instructions (a null key):
 * its tabs keep the context their saved notes ran with, so a missing folder
 * never re-runs them at launch, and the next pause analyses without code context.
 */
function pushContext(root: string | null) {
  const instructions = state.instructions[key(root)];
  if (!instructions) return;
  if (root !== null && state.summaryErrors[root] !== undefined) {
    tabs.setContext(root, null, instructions.current);
    return;
  }
  if (root !== null && !state.summaries[root]) return;
  const brief = root === null ? null : (state.summaries[root]?.brief ?? null);
  tabs.setContext(root, analysisContextKey(root ?? undefined, brief?.createdAt, instructions.current), instructions.current);
}

async function loadSummary(root: string): Promise<void> {
  try {
    const summary = await api.projectSummary(root);
    set({ summaries: { ...state.summaries, [root]: summary }, summaryErrors: without(state.summaryErrors, root) });
  } catch (e) {
    // The tab still opens with its draft and notes, and analyses without code context.
    set({ summaryErrors: { ...state.summaryErrors, [root]: errorMessage(e) } });
  }
  pushContext(root);
}

async function loadInstructions(root: string | null): Promise<void> {
  await saver.flush(); // a pending edit must not lose to the read below
  let project = EMPTY_INSTRUCTIONS;
  try {
    const view = await api.getInstructions(key(root));
    project = view.project;
    set({ templates: view.templates });
  } catch (e) {
    set({ instructionsError: `Couldn't load instructions: ${errorMessage(e)}` });
  }
  set({ instructions: { ...state.instructions, [key(root)]: project } });
  pushContext(root);
}

function once(id: string, load: () => Promise<void>) {
  if (loading.has(id)) return;
  loading.add(id);
  void load().finally(() => loading.delete(id));
}

/** Loads what a shown tab's project needs. A project that couldn't be read is retried each time. */
function ensure(root: string | null): void {
  if (root !== null && !state.summaries[root]) once(`s:${root}`, () => loadSummary(root));
  if (!state.instructions[key(root)]) once(`i:${key(root)}`, () => loadInstructions(root));
}

/** "Open a project…": scans the folder (replacing any loaded index) and returns its root. */
async function openFolder(path: string): Promise<string> {
  const summary = await api.openProject(path);
  set({ summaries: { ...state.summaries, [summary.root]: summary }, summaryErrors: without(state.summaryErrors, summary.root) });
  pushContext(summary.root);
  return summary.root;
}

function changeInstructions(root: string | null, next: ProjectInstructions): void {
  set({ instructions: { ...state.instructions, [key(root)]: next }, instructionsError: null });
  saver.schedule(key(root), next);
  pushContext(root); // re-reads the shown tab's draft, as editing instructions does today
}

async function changeTemplates(next: Template[]): Promise<void> {
  set({ templates: next });
  try {
    await api.saveTemplates(next);
    set({ instructionsError: null });
  } catch (e) {
    set({ instructionsError: `Couldn't save instructions: ${errorMessage(e)}` });
  }
}

async function restoreStarters(): Promise<void> {
  try {
    set({ templates: await api.restoreStarterTemplates(), instructionsError: null });
  } catch (e) {
    set({ instructionsError: `Couldn't save instructions: ${errorMessage(e)}` });
  }
}

function patchBrief(root: string, f: (b: BriefRun) => BriefRun) {
  set({ briefs: { ...state.briefs, [root]: f(state.briefs[root] ?? IDLE_BRIEF) } });
}

function buildBrief(root: string): void {
  if (state.briefs[root]?.building) return; // one build per project; the backend refuses a second too
  patchBrief(root, () => ({ building: true, live: "", activity: [], error: null }));
  api
    .buildBrief(root, (ev) => {
      if (ev.type === "text") {
        const text = ev.text;
        patchBrief(root, (b) => ({ ...b, live: b.live + text }));
      } else if (ev.type === "activity") {
        const line = activityLine(ev.label, ev.lane);
        patchBrief(root, (b) => ({ ...b, activity: [...b.activity, line] }));
      } else if (ev.type === "done") {
        patchBrief(root, (b) => ({ ...b, building: false }));
      } else if (ev.type === "error") {
        const message = ev.message;
        patchBrief(root, (b) => ({ ...b, building: false, error: message }));
      }
    })
    .catch((e) => patchBrief(root, (b) => ({ ...b, building: false, error: errorMessage(e) })));
}

/** `brief-status` from any window. On done, the rescanned summary carries the new brief to every tab of the project. */
function briefStatus(e: { root: string; state: string; message?: string | null }): void {
  if (e.state === "building") {
    patchBrief(e.root, (b) => (b.building ? b : { ...b, building: true, error: null }));
  } else if (e.state === "done") {
    patchBrief(e.root, (b) => ({ ...b, building: false }));
    void loadSummary(e.root);
  } else if (e.state === "error") {
    const message = e.message ?? "Couldn't build the project brief.";
    patchBrief(e.root, (b) => ({ ...b, building: false, error: b.error ?? message }));
  }
}

function start(): void {
  if (started) return; // StrictMode runs effects twice
  started = true;
  ensure(null); // the templates, and the instructions for tabs with no project
  void onBriefStatus((e) => briefStatus(e));
  if (typeof window !== "undefined") window.addEventListener("pagehide", () => void saver.flush());
}

export const projects = {
  start,
  ensure,
  openFolder,
  refreshSummary: loadSummary,
  // Events name the no-project session "" (as the instructions store does); here it is null.
  reloadInstructions: (root: string | null) => loadInstructions(root || null),
  changeInstructions,
  changeTemplates,
  restoreStarters,
  buildBrief,
  briefStatus,
  flush: () => saver.flush(),
};

export function projectsState(): ProjectsState {
  return state;
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export function useProjects(): ProjectsState {
  return useSyncExternalStore(subscribe, projectsState);
}
```

- [ ] **Step 4: Run the tests and the typecheck**

Run: `npx vitest run src/lib/projects.test.ts && npm test && npm run typecheck`
Expected: the 6 new tests PASS, the suite passes, and there are no type errors. Phase 1's `BriefStatusEvent` (`message: string | null`, `state` a string union) is assignable to `briefStatus`'s parameter, so `onBriefStatus((e) => briefStatus(e))` needs no mapping.

- [ ] **Step 5: Commit**

```bash
git add src/lib/projects.ts src/lib/projects.test.ts
git commit -m "feat(ui): per-window project store: summaries, instructions and brief builds by root

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 8: The sidebar: `Menu`, `TabRow`, `ClosedTabs` and `Sidebar`

The components are presentational over the stores and rules from Tasks 1 to 7, which carry the tests. This task is verified by the typecheck and the suite; Task 9 wires the sidebar in and runs the manual checks.

**Files:**
- Modify: `src/components/ui.tsx` (the `mini` button variant; `Menu`)
- Create: `src/components/TabRow.tsx`, `src/components/ClosedTabs.tsx`, `src/components/Sidebar.tsx`

**Interfaces:**
- Consumes:
  - `RULE`, `RuleState` (Task 1); `ruleState`, `statusLine`, `anyThinking` (Task 1)
  - `groupTabs`, `groupKey`, `TabGroup`, `rowTitle`, `renameResult`, `unreadableNotice` (Task 2)
  - `tabs`, `sessionFor`, `useWorkspace`, `WorkspaceState` (Task 6)
  - Phase 1: `useTabSession(s: TabSession | null): TabView | null` (`src/lib/useTabSession.ts`); `tabTitle(t: { title: string | null; firstLine: string }): string` (`src/lib/tabTitle.ts`); `TabSummary` (`{ id: string; root: string | null; title: string | null; firstLine: string; updatedAt: number; closedAt: number | null }`, `src/lib/ipc.ts`)
  - Unchanged in `ui.tsx`: `cx(...parts)`, `inputClass`, `Button` (variants `default | primary | ghost | link` before this task)
- Produces:
  - `ui.tsx`: `Button` variant `"mini"` (compact, `text-xs`); `export interface MenuItem { label: string; onSelect: () => void; disabled?: boolean }`; `export function Menu(props: { at: { x: number; y: number }; items: MenuItem[]; label: string; onClose: () => void })`. `Menu` renders data items only (no children, no submenus) and closes itself before running the chosen item's `onSelect`. Phase 3 adds its *Move to* entries to `TabRow`'s `items` array, so they must be `MenuItem`s ("Move to new window", then one per target window).
  - `TabRow({ tab: TabSummary; active: boolean })`
  - `ClosedTabs({ closed: TabSummary[] })`
  - `Sidebar` props:
    ```ts
    interface Props {
      hidden: boolean;            // collapsed with ⌘\; kept mounted so folded groups and dismissed notices survive
      opening: boolean;           // "Open a project…" reads "Reading project…"
      providerLabel: string;
      footer?: ReactNode;         // rendered above the provider label; Phase 4 passes <UsageFooter/>
      onOpenProject: () => void;
      onShowBrief: (root: string) => void;
      onBuildBrief: (root: string) => void;
      onOpenSettings: () => void;
    }
    ```

- [ ] **Step 1: Add the `mini` variant and `Menu` to `src/components/ui.tsx`**

Change the first import line to:

```ts
import { useEffect, useRef, type ButtonHTMLAttributes, type ReactNode } from "react";
```

Replace the `Variant` type, the `VARIANTS` map and `Button` with the version below. The font size moves from the base classes into each variant, so `mini` can be smaller without two font sizes fighting in one class list:

```tsx
type Variant = "default" | "primary" | "ghost" | "link" | "mini";

const VARIANTS: Record<Variant, string> = {
  default: "rounded-md border border-line bg-page px-3 py-1.5 text-[13px] hover:border-faint",
  // text-page, not white: the dark-mode accent is light enough to need dark text.
  primary: "rounded-md border border-accent bg-accent px-3 py-1.5 text-[13px] font-medium text-page hover:opacity-90",
  ghost: "rounded-md px-2.5 py-1.5 text-[13px] text-muted hover:bg-subtle hover:text-ink",
  link: "text-[13px] text-accent underline underline-offset-2",
  // Compact, for sidebar rows and group headers.
  mini: "rounded px-1.5 py-0.5 text-xs text-muted hover:bg-subtle hover:text-ink",
};

export function Button({ variant = "default", className, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex cursor-pointer items-center gap-2 transition-colors disabled:cursor-default disabled:opacity-50",
        VARIANTS[variant],
        className,
      )}
      {...props}
    />
  );
}
```

Append at the end of the file:

```tsx
export interface MenuItem {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
}

/**
 * A small popup menu at a point: where you right-clicked, or under a ⋯ button.
 * Choosing an item, Esc, a click outside, or the window losing focus closes
 * it; arrow keys move between items. Its Esc never reaches the drawers' handler.
 */
export function Menu({ at, items, label, onClose }: { at: { x: number; y: number }; items: MenuItem[]; label: string; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    const buttons = () => [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    buttons()[0]?.focus();
    const outside = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close.current();
    };
    const keys = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close.current();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const list = buttons();
        const i = list.indexOf(document.activeElement as HTMLButtonElement);
        const down = e.key === "ArrowDown";
        const next = i === -1 ? (down ? 0 : list.length - 1) : (i + (down ? 1 : -1) + list.length) % list.length;
        list[next]?.focus();
      }
    };
    const dismiss = () => close.current();
    window.addEventListener("mousedown", outside);
    window.addEventListener("keydown", keys, true);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("mousedown", outside);
      window.removeEventListener("keydown", keys, true);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
    };
  }, []);

  // Keep it on screen near the edges.
  const style = {
    left: Math.max(4, Math.min(at.x, window.innerWidth - 224)),
    top: Math.max(4, Math.min(at.y, window.innerHeight - 12 - items.length * 32)),
  };
  return (
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      style={style}
      className="fixed z-50 grid min-w-44 gap-0.5 rounded-md border border-line bg-page p-1 shadow-[0_8px_24px_rgb(0_0_0/0.12)]"
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          className="cursor-pointer rounded px-2.5 py-1.5 text-left text-[13px] text-ink hover:bg-subtle focus-visible:bg-subtle focus-visible:outline-none disabled:cursor-default disabled:opacity-50"
          onClick={() => {
            close.current();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: Create `src/components/TabRow.tsx`**

```tsx
import { useRef, useState, type MouseEvent } from "react";
import type { TabSummary } from "../lib/ipc";
import { renameResult, rowTitle } from "../lib/tabList";
import { sessionFor, tabs } from "../lib/tabs";
import { ruleState, statusLine } from "../lib/tabStatus";
import { useTabSession } from "../lib/useTabSession";
import { Menu, RULE, cx, inputClass } from "./ui";

/**
 * One tab: a 2 × 28 px rule in the same states as the rule between the panes,
 * the title, and a status line. Double-click or F2 renames; right-click (or
 * the context-menu key) opens Rename and Close.
 */
export function TabRow({ tab, active }: { tab: TabSummary; active: boolean }) {
  // Null until the tab is first shown: the rule is faint and there's no status line.
  const view = useTabSession(sessionFor(tab.id) ?? null);
  const [editing, setEditing] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const cancelled = useRef(false);
  const live = view?.draft ?? null;
  const title = rowTitle(tab, live);
  const auto = rowTitle({ ...tab, title: null }, live);
  const status = statusLine(view);

  const startRename = () => {
    cancelled.current = false;
    setEditing(title);
  };
  const finishRename = () => {
    const input = editing;
    setEditing(null);
    if (cancelled.current || input === null) return;
    const next = renameResult(input, tab.title, auto);
    if (next !== undefined) void tabs.rename(tab.id, next);
  };
  const openMenu = (e: MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    // The context-menu key reports 0,0: open under the row instead.
    const r = e.currentTarget.getBoundingClientRect();
    setMenu(e.clientX === 0 && e.clientY === 0 ? { x: r.left + 12, y: r.bottom } : { x: e.clientX, y: e.clientY });
  };

  const rule = (
    <span className="relative h-7 w-0.5 shrink-0 bg-line" aria-hidden>
      <span className={cx("absolute inset-0 origin-top", RULE[ruleState(view)])} />
    </span>
  );

  return (
    <li>
      {editing !== null ? (
        <div className="flex items-center gap-2.5 rounded-md bg-subtle px-2 py-1">
          {rule}
          <input
            autoFocus
            aria-label="Tab name"
            className={inputClass}
            value={editing}
            onChange={(e) => setEditing(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              else if (e.key === "Escape") {
                e.stopPropagation(); // Esc cancels the rename; it doesn't close a drawer
                cancelled.current = true;
                e.currentTarget.blur();
              }
            }}
            onBlur={finishRename}
          />
        </div>
      ) : (
        <button
          type="button"
          aria-current={active ? "true" : undefined}
          className={cx("flex w-full cursor-pointer items-center gap-2.5 rounded-md px-2 py-1 text-left", active ? "bg-subtle" : "hover:bg-subtle")}
          onClick={() => void tabs.switchTo(tab.id)}
          onDoubleClick={startRename}
          onKeyDown={(e) => {
            if (e.key === "F2") startRename();
          }}
          onContextMenu={openMenu}
        >
          {rule}
          <span className="grid min-w-0 flex-1">
            <span className="truncate text-[13px] text-ink">{title}</span>
            {status && <span className={cx("truncate text-xs", status.fault ? "text-fault" : "text-muted")}>{status.text}</span>}
          </span>
        </button>
      )}
      {menu && (
        <Menu
          at={menu}
          label={`${title}: tab actions`}
          onClose={() => setMenu(null)}
          items={[
            { label: "Rename", onSelect: startRename },
            { label: "Close", onSelect: () => void tabs.close(tab.id) },
          ]}
        />
      )}
    </li>
  );
}
```

- [ ] **Step 3: Create `src/components/ClosedTabs.tsx`**

```tsx
import { useState } from "react";
import type { TabSummary } from "../lib/ipc";
import { groupKey, groupTabs } from "../lib/tabList";
import { tabs } from "../lib/tabs";
import { tabTitle } from "../lib/tabTitle";
import { Button, cx } from "./ui";

/**
 * "Recently closed (n)": shared by every window, newest first, grouped by
 * project. Delete asks inline ("Delete for good?"), never with confirm().
 */
export function ClosedTabs({ closed }: { closed: TabSummary[] }) {
  const [open, setOpen] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  if (closed.length === 0) return null;

  return (
    <section className="mt-4" aria-label="Recently closed">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-left text-xs text-muted hover:text-ink"
      >
        <span aria-hidden className={cx("text-faint transition-transform motion-reduce:transition-none", open && "rotate-90")}>
          ▸
        </span>
        Recently closed ({closed.length})
      </button>
      {open &&
        groupTabs(closed).map((g) => (
          <div key={groupKey(g.root)} className="mt-2 grid gap-0.5">
            <p className="truncate px-2 text-xs font-medium text-faint" title={g.root ?? undefined}>
              {g.name}
            </p>
            <ul className="grid gap-0.5">
              {g.tabs.map((t) => (
                <li key={t.id} className="flex items-center gap-1 px-2">
                  {deleting === t.id ? (
                    <>
                      <span className="min-w-0 flex-1 truncate text-xs text-ink">Delete for good?</span>
                      <Button
                        variant="mini"
                        onClick={() => {
                          setDeleting(null);
                          void tabs.remove(t.id);
                        }}
                      >
                        Yes
                      </Button>
                      <Button variant="mini" onClick={() => setDeleting(null)}>
                        No
                      </Button>
                    </>
                  ) : (
                    <>
                      <span className="min-w-0 flex-1 truncate text-xs text-muted" title={tabTitle(t)}>
                        {tabTitle(t)}
                      </span>
                      <Button variant="mini" onClick={() => void tabs.reopen(t.id)}>
                        Reopen
                      </Button>
                      <Button variant="mini" onClick={() => setDeleting(t.id)}>
                        Delete
                      </Button>
                    </>
                  )}
                </li>
              ))}
            </ul>
          </div>
        ))}
    </section>
  );
}
```

- [ ] **Step 4: Create `src/components/Sidebar.tsx`**

```tsx
import { useCallback, useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import type { TabSummary } from "../lib/ipc";
import { groupKey, groupTabs, unreadableNotice, type TabGroup } from "../lib/tabList";
import { sessionFor, tabs, useWorkspace, type WorkspaceState } from "../lib/tabs";
import { anyThinking } from "../lib/tabStatus";
import { ClosedTabs } from "./ClosedTabs";
import { TabRow } from "./TabRow";
import { Button, Menu, cx } from "./ui";

interface Props {
  /** Collapsed with ⌘\: kept mounted, so folded groups and dismissed notices survive. */
  hidden: boolean;
  opening: boolean;
  providerLabel: string;
  /** Rendered above the provider label. Phase 4 passes the usage line. */
  footer?: ReactNode;
  onOpenProject: () => void;
  onShowBrief: (root: string) => void;
  onBuildBrief: (root: string) => void;
  onOpenSettings: () => void;
}

/** This window's tabs, grouped by project in order of first appearance, then Recently closed. */
export function Sidebar(p: Props) {
  const ws = useWorkspace();
  const [folded, setFolded] = useState<string[]>([]);
  const [dismissed, setDismissed] = useState<string[]>([]);
  const groups = groupTabs(ws.view?.tabs ?? []);
  const notices = (ws.view?.unreadable ?? []).filter((path) => !dismissed.includes(path));

  // Showing a tab in a folded group (⌃Tab, reopening it) unfolds the group.
  const activeRoot = ws.view?.tabs.find((t) => t.id === ws.activeId)?.root;
  useEffect(() => {
    if (activeRoot === undefined) return;
    const k = groupKey(activeRoot);
    setFolded((f) => (f.includes(k) ? f.filter((x) => x !== k) : f));
  }, [ws.activeId, activeRoot]);

  const toggle = (k: string) => setFolded((f) => (f.includes(k) ? f.filter((x) => x !== k) : [...f, k]));

  return (
    <nav aria-label="Tabs" className={cx(p.hidden ? "hidden" : "flex", "min-h-0 flex-col border-r border-line bg-panel")}>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-3">
        {ws.view && groups.length === 0 && <p className="px-2 pb-2 text-xs text-faint">No tabs in this window.</p>}
        {groups.map((g) => (
          <Group
            key={groupKey(g.root)}
            group={g}
            ws={ws}
            folded={folded.includes(groupKey(g.root))}
            onFold={() => toggle(groupKey(g.root))}
            onShowBrief={p.onShowBrief}
            onBuildBrief={p.onBuildBrief}
          />
        ))}
        <Button variant="ghost" className="mt-1 w-full" onClick={p.onOpenProject} disabled={p.opening}>
          {p.opening ? "Reading project…" : "Open a project…"}
        </Button>
        <ClosedTabs closed={ws.view?.closed ?? []} />
      </div>

      <footer className="grid shrink-0 gap-1.5 border-t border-line px-2 py-2">
        {notices.map((path) => (
          <p key={path} className="flex items-start gap-2 px-2 text-xs text-muted" role="status">
            <span className="min-w-0 flex-1 break-words" title={path}>
              {unreadableNotice(path)}
            </span>
            <Button variant="mini" onClick={() => setDismissed((d) => [...d, path])}>
              OK
            </Button>
          </p>
        ))}
        {ws.saveError && (
          <p className="px-2 text-xs text-fault" role="alert">
            {ws.saveError}
          </p>
        )}
        {ws.error && (
          <p className="px-2 text-xs text-fault" role="alert">
            {ws.error}
          </p>
        )}
        {p.footer}
        <Button variant="ghost" className="w-full min-w-0" onClick={p.onOpenSettings} title="Model and settings">
          <span className="truncate">{p.providerLabel}</span>
        </Button>
      </footer>
    </nav>
  );
}

interface GroupProps {
  group: TabGroup<TabSummary>;
  ws: WorkspaceState;
  folded: boolean;
  onFold: () => void;
  onShowBrief: (root: string) => void;
  onBuildBrief: (root: string) => void;
}

/** A project's header (name, ⋯ menu, +) and its tabs. "No project" has no ⋯ menu. */
function Group({ group, ws, folded, onFold, onShowBrief, onBuildBrief }: GroupProps) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const root = group.root;
  return (
    <section className="mb-3" aria-label={group.name}>
      <div className="flex items-center gap-0.5">
        <button
          type="button"
          aria-expanded={!folded}
          onClick={onFold}
          title={root ?? undefined}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-left text-xs font-semibold text-muted hover:text-ink"
        >
          <span aria-hidden className={cx("text-faint transition-transform motion-reduce:transition-none", !folded && "rotate-90")}>
            ▸
          </span>
          <span className="truncate">{group.name}</span>
          {folded && <FoldedDot ids={group.tabs.map((t) => t.id)} ws={ws} />}
        </button>
        {root !== null && (
          <Button
            variant="mini"
            aria-label={`${group.name}: project actions`}
            aria-haspopup="menu"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setMenu({ x: r.left, y: r.bottom + 4 });
            }}
          >
            ⋯
          </Button>
        )}
        <Button variant="mini" aria-label={`New tab in ${group.name}`} title="New tab" onClick={() => void tabs.newTab(root)}>
          +
        </Button>
      </div>
      {!folded && (
        <ul className="mt-0.5 grid gap-0.5">
          {group.tabs.map((t) => (
            <TabRow key={t.id} tab={t} active={t.id === ws.activeId} />
          ))}
        </ul>
      )}
      {menu && root !== null && (
        <Menu
          at={menu}
          label={`${group.name}: project actions`}
          onClose={() => setMenu(null)}
          items={[
            { label: "Project brief…", onSelect: () => onShowBrief(root) },
            { label: "Rebuild brief", onSelect: () => onBuildBrief(root) },
          ]}
        />
      )}
    </section>
  );
}

/** A folded group pulses while any of its tabs is analysing; the hidden text says so to screen readers. */
function FoldedDot({ ids, ws }: { ids: string[]; ws: WorkspaceState }) {
  const key = ids.join(" ");
  // Re-subscribes when the workspace changes, which is when sessions are created.
  const subscribe = useCallback(
    (fn: () => void) => {
      const offs = key.split(" ").map((id) => sessionFor(id)?.subscribe(fn));
      return () => offs.forEach((off) => off?.());
    },
    [key, ws],
  );
  const thinking = useSyncExternalStore(subscribe, () => anyThinking(key.split(" ").map((id) => sessionFor(id)?.getState())));
  if (!thinking) return null;
  return (
    <span className="ml-auto size-1.5 shrink-0 rounded-full bg-accent animate-pulse-dot motion-reduce:animate-none" title="Analysing…">
      <span className="sr-only">Analysing…</span>
    </span>
  );
}
```

- [ ] **Step 5: Typecheck and test**

Run: `npm run typecheck && npm test`
Expected: no type errors (the new components aren't rendered yet, so the app itself is unchanged) and every test passes. `Button`'s existing callers render as before: each variant still sets `text-[13px]`.

- [ ] **Step 6: Commit**

```bash
git add src/components/ui.tsx src/components/TabRow.tsx src/components/ClosedTabs.tsx src/components/Sidebar.tsx
git commit -m "feat(ui): sidebar with project groups, tab rows, Recently closed, and a shared Menu

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

---

### Task 9: The three-column window: history in the notes, the new top bar, `App.tsx`, and the docs

`NotesPane`'s props, the top bar and `App.tsx` change together, so they land in one task to keep the typecheck green. Phase 1's `NotesPane` and `App.tsx` are replaced whole: their props and wiring were for a single tab.

**Files:**
- Replace: `src/components/NotesPane.tsx`, `src/components/TopBar.tsx`, `src/App.tsx`
- Modify: `src/components/BriefDrawer.tsx` (a `projectName` prop in the header)
- Modify: `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes:
  - `TabView`, `TabSession` (Task 4); `historyPosition`, `snapshotAnalysis`, `snapshotTime` (Task 3); `ruleState` (Task 1); `RULE`, `RuleState`, `Button`, `Eyebrow`, `cx` (Task 1, Task 8)
  - `useActions` (Task 5); `tabs`, `sessionFor`, `useWorkspace` (Task 6); `projects`, `useProjects`, `IDLE_BRIEF` (Task 7); `Sidebar` (Task 8); `projectName`, `rowTitle`, `windowTitle`, `readCollapsed`, `writeCollapsed` (Task 2)
  - Phase 1: `useTabSession(s: TabSession | null): TabView | null`; `currentWindowId(): string`; `api.getSettings(): Promise<Settings>`, `api.saveSettings(s: Settings): Promise<void>` and `api.warmUp(): Promise<void>` (unchanged)
  - Unchanged components and helpers:
    - `DraftPane({ value: string; onChange: (v: string) => void; hasProject: boolean })`
    - `InstructionsDrawer({ instructions: ProjectInstructions; templates: Template[]; localModel: boolean; error: string | null; onChange: (next: ProjectInstructions) => void; onClose: () => void; children?: ReactNode })`
    - `TemplateList({ templates: Template[]; current: string; onSave: (next: Template[]) => void; onRestore: () => void })`
    - `SettingsDialog({ settings: Settings; onSave: (s: Settings) => Promise<void>; onClose: () => void })`, a native `<dialog>` opened with `showModal()`
    - `BriefDrawer` props before Step 3: `{ brief: StoredBrief | null; status: BriefStatus | null; building: boolean; liveText: string; activity: string[]; error: string | null; onBuild: () => void; onClose: () => void }`
    - `providerLabel(s: Settings | null): string`; `instructionsLabel(p: ProjectInstructions, templates: Template[]): string`; `EMPTY_INSTRUCTIONS`
    - `VERDICT`, `checkLocation`, `checkSummary`, `plainClaim` (`src/lib/checks.ts`); `SECTIONS`, `parseSections`, `SectionKey` (`src/lib/sections.ts`); `MiniMarkdown({ text, className? })`
- Produces:
  - `NotesPane` props: `{ view: TabView; hasProject: boolean; briefMissing: boolean; onBuildBrief: () => void; onRerun: () => void; onView: (index: number) => void }`
  - `TopBar` props: `{ tab: TopBarTab | null; providerLabel: string; sidebarOpen: boolean; onToggleSidebar: () => void; onShowBrief: () => void; onShowInstructions: () => void; onOpenSettings: () => void }`, with `export interface TopBarTab { projectName: string | null; briefStatus: BriefStatus | null; briefBuilding: boolean; instructionsLabel: string }`
  - `BriefDrawer` gains `projectName: string`
  - `App.tsx`: binds every action except `new-window` and `close-window` through one `useActions({...})` call, which Phase 3 extends; renders `<Sidebar … />` without `footer`, which Phase 4 adds.

- [ ] **Step 1: Replace `src/components/NotesPane.tsx`**

```tsx
import { useState } from "react";
import type { CheckResult } from "../lib/ipc";
import { VERDICT, checkLocation, checkSummary, plainClaim } from "../lib/checks";
import { historyPosition, snapshotAnalysis, snapshotTime } from "../lib/history";
import { SECTIONS, parseSections, type SectionKey } from "../lib/sections";
import type { TabView } from "../lib/tabSession";
import { ruleState } from "../lib/tabStatus";
import { MiniMarkdown } from "./MiniMarkdown";
import { Button, Eyebrow, RULE, cx, type RuleState } from "./ui";

interface Props {
  view: TabView;
  hasProject: boolean;
  briefMissing: boolean;
  onBuildBrief: () => void;
  onRerun: () => void;
  /** Shows analysis `index` of the history (0 = latest). */
  onView: (index: number) => void;
}

const DOT: Record<SectionKey, string> = {
  intent: "bg-muted",
  faults: "bg-fault",
  directions: "bg-accent",
  questions: "bg-question",
};

function statusText(v: TabView) {
  const a = v.analysis;
  if (a.status === "thinking") return "Thinking";
  if (a.status === "error") return "Couldn't think this through";
  if (v.stale) return "Your draft moved on. Pause to refresh.";
  if (a.status === "done") return "Up to date with your draft";
  return "Reading along";
}

/** The accent rule between the panes: solid when current, animated while thinking. Always the live state. */
function Rule({ state }: { state: RuleState }) {
  return (
    <div className="absolute inset-y-0 left-0 w-0.5 bg-line" aria-hidden>
      <div className={cx("absolute inset-0 origin-top", RULE[state])} />
    </div>
  );
}

const TONE = { accent: "text-accent", fault: "text-fault", faint: "text-faint" } as const;

function CheckRow({ check }: { check: CheckResult }) {
  const v = VERDICT[check.verdict];
  const where = checkLocation(check);
  return (
    <li className="grid grid-cols-[1rem_minmax(0,1fr)] gap-x-2 text-[13px] leading-snug">
      <span className={cx("font-semibold", TONE[v.tone])} aria-hidden>
        {v.mark}
      </span>
      <span className="min-w-0">
        <span className="block truncate" title={plainClaim(check.claim)}>
          {plainClaim(check.claim)}
        </span>
        <span className="text-xs text-muted">
          {v.label}
          {where && (
            <>
              {" · "}
              <code className="font-mono text-[0.85em]">{where}</code>
            </>
          )}
        </span>
      </span>
    </li>
  );
}

/**
 * `‹ 2 of 5 ›` and the time of the analysis on screen. Hidden with fewer than
 * two, and over the live notes while a run is under way (history[0] isn't
 * them yet). Over an older analysis it stays, so a run that starts while you
 * browse never strands you there.
 */
function HistoryHeader({ view, onView }: { view: TabView; onView: (index: number) => void }) {
  const pos = historyPosition(view.viewing, view.history.length);
  if (!pos.show || (view.viewing === 0 && view.analysis.status === "thinking")) return null;
  // The latest is the live state: it has history[0]'s time once that analysis is what's on screen.
  const shown = view.viewing > 0 ? view.history[view.viewing] : view.analysis.status === "done" ? view.history[0] : undefined;
  return (
    <nav className="mb-4 flex items-center gap-1 text-xs tabular-nums text-faint" aria-label="Earlier analyses">
      <Button variant="mini" aria-label="Earlier analysis" disabled={pos.earlier === null} onClick={() => pos.earlier !== null && onView(pos.earlier)}>
        ‹
      </Button>
      <span aria-live="polite">{pos.label}</span>
      <Button variant="mini" aria-label="Later analysis" disabled={pos.later === null} onClick={() => pos.later !== null && onView(pos.later)}>
        ›
      </Button>
      {shown && <span className="ml-1">{snapshotTime(shown.at)}</span>}
    </nav>
  );
}

/** Over an older analysis: the banner, and the draft it read, collapsed by default. */
function EarlierDraft({ draft }: { draft: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mb-5 grid gap-2 text-xs text-muted">
      <p>
        Notes on an earlier draft.{" "}
        <Button variant="link" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "Hide that draft" : "Show that draft"}
        </Button>
      </p>
      {open && <p className="whitespace-pre-wrap border-l-2 border-line pl-3 font-serif text-[15px] leading-relaxed text-muted">{draft}</p>}
    </div>
  );
}

export function NotesPane({ view, hasProject, briefMissing, onBuildBrief, onRerun, onView }: Props) {
  const older = view.viewing > 0 ? view.history[view.viewing] : undefined;
  // An older analysis is read-only: its notes, checks and notices, never a retry.
  const a = older ? snapshotAnalysis(older) : view.analysis;
  const stale = older ? false : view.stale;
  const sections = parseSections(a.markdown);

  return (
    <aside className="relative flex min-h-0 flex-col bg-panel" aria-label="Notes">
      <Rule state={ruleState(view)} />
      <div className="min-h-0 flex-1 overflow-y-auto px-7 pt-7 pb-10">
        <HistoryHeader view={view} onView={onView} />
        {older ? (
          <EarlierDraft key={older.id} draft={older.draft} />
        ) : (
          <p className="mb-5 text-xs text-faint" aria-live="polite">
            {statusText(view)}
          </p>
        )}
        {a.notices.map((n, i) => (
          <p key={i} className="mb-2 text-xs text-muted">
            {n}
          </p>
        ))}
        {a.partial && a.status === "done" && (
          <p className="mb-4 text-xs text-muted">
            Some sections are missing.
            {!older && (
              <>
                {" "}
                <Button variant="link" onClick={onRerun}>
                  Try again
                </Button>
              </>
            )}
          </p>
        )}

        {!older && !hasProject && <p className="mb-4 text-[13px] text-muted">This tab has no project, so notes won't know your code.</p>}
        {!older && hasProject && briefMissing && (
          <p className="mb-4 text-[13px] text-muted">
            Notes are sharper with a project brief.{" "}
            <Button variant="link" onClick={onBuildBrief}>
              Build it now
            </Button>
          </p>
        )}

        {a.status === "error" && (
          <div className="grid justify-items-start gap-2 text-[13px] text-fault" role="alert">
            <p className="whitespace-pre-wrap">{a.error}</p>
            <Button onClick={onRerun}>Try again</Button>
          </div>
        )}

        {a.status === "idle" && (
          <div className="max-w-[46ch] space-y-2.5 text-[13px] leading-relaxed text-muted">
            <p>Start writing on the left. When you pause, notes appear here:</p>
            <p>
              what you seem to be after, faults against how the project actually works, directions worth trying, and the
              questions that matter most.
            </p>
          </div>
        )}

        {a.status !== "idle" && a.status !== "error" && (
          <div className={cx("grid gap-6 transition-opacity motion-reduce:transition-none", stale && "opacity-60")}>
            {SECTIONS.map(({ key, title }) =>
              sections[key] !== undefined ? (
                <section key={key} className="grid gap-1.5">
                  <h2>
                    <Eyebrow dot={DOT[key]}>{title}</Eyebrow>
                  </h2>
                  <MiniMarkdown text={sections[key]!} />
                </section>
              ) : null,
            )}
            {a.status === "thinking" && !a.markdown && <p className="text-[13px] text-muted">Reading your draft against the project…</p>}
            {a.checking && (
              <section className="grid gap-1.5">
                <h2>
                  <Eyebrow dot="bg-ink">Checked in code</Eyebrow>
                </h2>
                <p className="text-xs text-faint">{checkSummary(a.checking, a.checks.length, a.checksStopped)}</p>
                <ul className="grid gap-2">
                  {a.checks.map((c, i) => (
                    <CheckRow key={i} check={c} />
                  ))}
                </ul>
              </section>
            )}
          </div>
        )}
      </div>
    </aside>
  );
}
```

- [ ] **Step 2: Replace `src/components/TopBar.tsx`**

*Open a project* moves to the sidebar. The bar shows the active tab's project, its brief status, the Instructions button and the provider label; with no active tab, only the app name (and the sidebar toggle, so a collapsed sidebar can always come back).

```tsx
import type { BriefStatus } from "../lib/ipc";
import { Button, cx } from "./ui";

/** What the top bar shows about the active tab. */
export interface TopBarTab {
  /** Null for a tab with no project. */
  projectName: string | null;
  /** Null until the project's summary has loaded. */
  briefStatus: BriefStatus | null;
  briefBuilding: boolean;
  instructionsLabel: string;
}

interface Props {
  tab: TopBarTab | null;
  providerLabel: string;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onShowBrief: () => void;
  onShowInstructions: () => void;
  onOpenSettings: () => void;
}

const BRIEF_LABEL: Record<BriefStatus, string> = {
  fresh: "Project brief is current",
  stale: "Files changed since the brief",
  missing: "No project brief yet",
};

const BRIEF_DOT: Record<BriefStatus | "building", string> = {
  fresh: "bg-accent",
  stale: "bg-question",
  missing: "bg-faint",
  building: "bg-accent animate-pulse-dot motion-reduce:animate-none",
};

export function TopBar(p: Props) {
  const t = p.tab;
  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
      <Button
        variant="mini"
        aria-label={p.sidebarOpen ? "Hide sidebar" : "Show sidebar"}
        aria-expanded={p.sidebarOpen}
        title="Show or hide the sidebar"
        onClick={p.onToggleSidebar}
      >
        <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" aria-hidden>
          <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
          <path d="M6 2.5v11" />
        </svg>
      </Button>
      <span className="mr-1 text-[15px] font-semibold tracking-tight" aria-hidden>
        LiveBrainstorm
      </span>

      {t && (
        <>
          <span className={cx("min-w-0 truncate text-[13px] font-medium", t.projectName === null && "text-muted")}>
            {t.projectName ?? "No project"}
          </span>

          {t.projectName !== null && (t.briefStatus !== null || t.briefBuilding) && (
            <Button variant="ghost" onClick={p.onShowBrief}>
              <span className={cx("size-2 rounded-full", BRIEF_DOT[t.briefBuilding ? "building" : (t.briefStatus ?? "missing")])} aria-hidden />
              {t.briefBuilding ? "Building project brief…" : BRIEF_LABEL[t.briefStatus ?? "missing"]}
            </Button>
          )}

          <Button variant="ghost" className="min-w-0" onClick={p.onShowInstructions} title="Session instructions">
            <span className="max-w-64 truncate">Instructions: {t.instructionsLabel}</span>
          </Button>

          <span className="flex-1" />
          <Button variant="ghost" onClick={p.onOpenSettings} title="Model and settings">
            {p.providerLabel}
          </Button>
        </>
      )}
    </header>
  );
}
```

- [ ] **Step 3: Name the project in the brief drawer**

The drawer can now open for any project in the sidebar (⋯ → *Project brief…*), so its header names it. In `src/components/BriefDrawer.tsx`, add `projectName: string;` as the first field of `interface Props`, and replace the header's heading:

```tsx
        <h2 className="flex-1 text-[15px] font-semibold">Project brief</h2>
```

with:

```tsx
        <h2 className="min-w-0 flex-1 truncate text-[15px] font-semibold">
          Project brief <span className="font-normal text-muted">· {p.projectName}</span>
        </h2>
```

- [ ] **Step 4: Replace `src/App.tsx`**

```tsx
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type Settings } from "./lib/ipc";
import { providerLabel } from "./lib/providerOptions";
import { EMPTY_INSTRUCTIONS, instructionsLabel } from "./lib/instructions";
import { useActions } from "./lib/actions";
import { IDLE_BRIEF, projects, useProjects } from "./lib/projects";
import { projectName, readCollapsed, rowTitle, windowTitle, writeCollapsed } from "./lib/tabList";
import { sessionFor, tabs, useWorkspace } from "./lib/tabs";
import { useTabSession } from "./lib/useTabSession";
import { currentWindowId } from "./lib/windowId";
import { TopBar } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { InstructionsDrawer } from "./components/InstructionsDrawer";
import { TemplateList } from "./components/TemplateList";
import { SettingsDialog } from "./components/SettingsDialog";
import { Button, cx } from "./components/ui";

export default function App() {
  const ws = useWorkspace();
  const pj = useProjects();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [showInstructions, setShowInstructions] = useState(false);
  /** The project whose brief drawer is open; any project in the sidebar can open it. */
  const [briefRoot, setBriefRoot] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(() => readCollapsed(currentWindowId(), window.innerWidth));

  const session = ws.activeId ? (sessionFor(ws.activeId) ?? null) : null;
  const view = useTabSession(session);
  const root = session?.root ?? null;
  const summary = root !== null ? (pj.summaries[root] ?? null) : null;
  const briefRun = root !== null ? (pj.briefs[root] ?? IDLE_BRIEF) : IDLE_BRIEF;
  const instructions = pj.instructions[root ?? ""] ?? EMPTY_INSTRUCTIONS;
  const activeSummary = ws.view?.tabs.find((t) => t.id === ws.activeId) ?? null;
  const title = activeSummary ? rowTitle(activeSummary, view?.draft ?? null) : null;

  useEffect(() => {
    void api.getSettings().then((s) => {
      setSettings(s);
      tabs.setDebounce(s.debounceMs);
    });
    void api.warmUp();
    projects.start();
    void tabs.load();
  }, []);

  // A shown tab's project loads on first show; its context reaches the tab once summary and instructions are in.
  useEffect(() => {
    if (session) projects.ensure(session.root);
  }, [session]);

  useEffect(() => {
    document.title = windowTitle(title);
  }, [title]);

  // Esc closes either drawer. The settings <dialog> handles its own Esc.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showSettings) {
        setBriefRoot(null);
        setShowInstructions(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  const toggleSidebar = () => {
    const next = !collapsed;
    writeCollapsed(currentWindowId(), next);
    setCollapsed(next);
  };

  // "Open a project…": a new tab on the chosen folder.
  const openFolder = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir !== "string") return;
    setOpening(true);
    setOpenError(null);
    try {
      await tabs.newTab(await projects.openFolder(dir));
    } catch (e) {
      setOpenError(errorMessage(e));
    } finally {
      setOpening(false);
    }
  };

  const showBrief = (r: string) => {
    setShowInstructions(false);
    projects.ensure(r);
    setBriefRoot((current) => (current === r ? null : r));
  };

  const buildBrief = (r: string) => {
    setShowInstructions(false);
    setBriefRoot(r);
    projects.ensure(r);
    projects.buildBrief(r);
  };

  // Phase 3 adds new-window and close-window, and routes its menu events here.
  useActions({
    "new-tab": () => void tabs.newTab(root),
    "open-project": () => void openFolder(),
    "reopen-tab": () => {
      const last = ws.view?.closed[0];
      if (last) void tabs.reopen(last.id);
    },
    "close-tab": () => {
      if (ws.activeId) void tabs.close(ws.activeId);
    },
    "next-tab": () => tabs.next(1),
    "prev-tab": () => tabs.next(-1),
    "toggle-sidebar": toggleSidebar,
    settings: () => setShowSettings(true),
  });

  const saveSettings = async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
    tabs.setDebounce(s.debounceMs);
  };

  // A folder that couldn't be opened or read: shown under the top bar, as a Precondition message.
  const alert = openError ?? (root !== null ? (pj.summaryErrors[root] ?? null) : null);
  const drawerSummary = briefRoot !== null ? (pj.summaries[briefRoot] ?? null) : null;
  const drawerRun = briefRoot !== null ? (pj.briefs[briefRoot] ?? IDLE_BRIEF) : IDLE_BRIEF;

  return (
    <div className="flex h-full flex-col">
      <TopBar
        tab={
          session
            ? {
                projectName: root === null ? null : projectName(root),
                briefStatus: summary?.briefStatus ?? null,
                briefBuilding: briefRun.building,
                instructionsLabel: instructionsLabel(instructions, pj.templates),
              }
            : null
        }
        providerLabel={providerLabel(settings)}
        sidebarOpen={!collapsed}
        onToggleSidebar={toggleSidebar}
        onShowBrief={() => {
          if (root !== null) showBrief(root);
        }}
        onShowInstructions={() => {
          setBriefRoot(null);
          setShowInstructions((v) => !v);
        }}
        onOpenSettings={() => setShowSettings(true)}
      />
      {alert && (
        <p className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-fault" role="alert">
          {alert}
        </p>
      )}

      {/* Sidebar, draft and notes. The draft and notes panes scroll on their own. */}
      <main
        className={cx(
          "relative grid min-h-0 flex-1",
          collapsed ? "grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]" : "grid-cols-[248px_minmax(0,3fr)_minmax(320px,2fr)]",
        )}
      >
        <Sidebar
          hidden={collapsed}
          opening={opening}
          providerLabel={providerLabel(settings)}
          onOpenProject={() => void openFolder()}
          onShowBrief={showBrief}
          onBuildBrief={buildBrief}
          onOpenSettings={() => setShowSettings(true)}
        />
        {session && view ? (
          <>
            {/* Keyed by tab, so the textarea's undo history and the notes' local state never cross tabs.
                Sibling keys must differ, hence the prefixes. */}
            <DraftPane
              key={`draft:${session.id}`}
              value={view.draft}
              onChange={(text) => session.setDraft(text)}
              hasProject={root !== null}
            />
            <NotesPane
              key={`notes:${session.id}`}
              view={view}
              hasProject={root !== null}
              briefMissing={summary !== null && !summary.brief && !briefRun.building}
              onBuildBrief={() => {
                if (root !== null) buildBrief(root);
              }}
              onRerun={() => session.rerun()}
              onView={(i) => session.view(i)}
            />
          </>
        ) : (
          <section className="col-span-2 grid place-items-center bg-page" aria-label="No tab open">
            {!ws.view && ws.error && (
              <p className="text-[13px] text-fault" role="alert">
                {ws.error}
              </p>
            )}
            {ws.view && (
              <div className="grid justify-items-center gap-3 text-center">
                <p className="text-[13px] text-muted">No tab is open in this window.</p>
                <div className="flex gap-2">
                  <Button variant="primary" onClick={() => void openFolder()} disabled={opening}>
                    {opening ? "Reading project…" : "Open a project…"}
                  </Button>
                  <Button variant="ghost" onClick={() => void tabs.newTab(null)}>
                    New tab without a project
                  </Button>
                </div>
              </div>
            )}
          </section>
        )}
        {briefRoot !== null && (
          <BriefDrawer
            projectName={projectName(briefRoot)}
            brief={drawerSummary?.brief ?? null}
            status={drawerSummary?.briefStatus ?? null}
            building={drawerRun.building}
            liveText={drawerRun.live}
            activity={drawerRun.activity}
            error={drawerRun.error}
            onBuild={() => buildBrief(briefRoot)}
            onClose={() => setBriefRoot(null)}
          />
        )}
        {showInstructions && session && (
          <InstructionsDrawer
            key={root ?? ""}
            instructions={instructions}
            templates={pj.templates}
            localModel={settings?.provider.kind === "local"}
            error={pj.instructionsError}
            onChange={(next) => projects.changeInstructions(root, next)}
            onClose={() => setShowInstructions(false)}
          >
            <TemplateList
              templates={pj.templates}
              current={instructions.current}
              onSave={(next) => void projects.changeTemplates(next)}
              onRestore={() => void projects.restoreStarters()}
            />
          </InstructionsDrawer>
        )}
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
```

- [ ] **Step 5: Remove what only Phase 1's single-pane `App.tsx` used**

Phase 1's `src/lib/tabContext.ts` held the shown tab's context rule; `projects.ts` (Task 7) replaces it, and after Step 4 only its own test imports it. Run:
```bash
for n in $(git show HEAD:src/App.tsx | grep -o 'from "\./lib/[^"]*"' | sed 's#.*/##; s#"##' | sort -u); do
  printf '%s: ' "$n"; grep -rlE "from \"(\./|\.\./lib/|\./lib/)$n\"" src | grep -v '\.test\.ts$' | tr '\n' ' '; echo
done
```
It lists each `./lib/…` module Phase 1's `App.tsx` imported, and the non-test files that still import it. Expected: every module has at least one importer except `tabContext`, which has none. Then:
```bash
git rm src/lib/tabContext.ts src/lib/tabContext.test.ts
grep -rn "tabContext" src
grep -rn "importLegacyDrafts\|livebrainstorm\.draft\.\|livebrainstorm\.lastProject" src --include='*.ts' --include='*.tsx' | grep -v '\.test\.ts'
```
Expected: `git rm` succeeds; the first `grep` prints nothing (exit status 1); the second prints only `src/lib/ipc.ts` (the `api` wrapper), `src/lib/legacyDrafts.ts` (the old keys, read once for the import) and `src/lib/tabs.ts` (the call). If the loop shows another module with no importer, `git rm` it and its test too.

- [ ] **Step 6: Typecheck, test, and check the Rust side is untouched**

Run: `npm run typecheck && npm test && cargo test -p livebrainstorm-core && cargo check -p livebrainstorm && cargo clippy --workspace --all-targets`
Expected: no type errors, every vitest and core test passes, the shell compiles, and clippy prints no warnings. `git diff --stat HEAD -- crates src-tauri` prints nothing.

- [ ] **Step 7: Docs: AGENTS.md and README.md**

Phase 1 has already edited AGENTS.md (the Architecture rows, live analysis steps 1, 2 and 4, the brief paragraph, persistence, contracts, ids), so find each passage below by its text, not by line number. Phase 3 later inserts a bullet right after `- **Settings**` in "### UI and styling", so leave that bullet where it is.

**AGENTS.md, "## Architecture".** Replace the table row that starts with `| UI |` (Phase 1's version ends with "each tab's live analysis in a `TabSession` (`src/lib/tabSession.ts`) outside React") with:

```markdown
| UI | `src/` (React 19 + Vite, no state library) | Sidebar of tabs, draft, notes, brief drawer, settings. This window's tabs live in `src/lib/tabs.ts` and its projects' summaries, instructions and brief builds in `src/lib/projects.ts` (module stores read with `useSyncExternalStore`); each tab's live analysis is a `TabSession` (`src/lib/tabSession.ts`) outside React. `App.tsx` holds only settings and which drawer is open |
```

**AGENTS.md, "### UI and styling".** Replace the `- **Shared primitives**` bullet with:

```markdown
- **Shared primitives** live in `src/components/ui.tsx`: `Button` (with a compact `mini` variant for sidebar rows), `Field`, `inputClass`, `Eyebrow`, `Menu` (a popup menu at a point; Esc, a click outside or the window losing focus closes it), `RULE` / `RuleState`, and the `cx` class joiner. Use them instead of repeating class lists.
```

Replace the `- **Layout:**` bullet with:

```markdown
- **Layout:** `main` is a three-column grid, `[sidebar 248px | draft 3fr | notes minmax(320px, 2fr)]`. The draft and notes panes are flex columns with `min-h-0` and their own `overflow-y-auto`. The draft `<textarea>` fills its pane, and percentage padding centres a 40rem text column, so the textarea and the word-count footer line up. ⌘\\ hides the sidebar (`display: none`, so the grid drops to two columns). A window narrower than 1,100 px starts with it hidden, unless the user chose otherwise in that window (`readCollapsed` / `writeCollapsed` in `src/lib/tabList.ts`).
```

Replace the `- **Rule animation:**` bullet with:

```markdown
- **Rule animation:** the accent rule between the panes (`Rule` in `NotesPane.tsx`) and the 2 × 28 px rule on each sidebar tab row share the state classes in `RULE` (`ui.tsx`): solid when the notes are current, animated (`animate-fill`) while thinking, dashed when the draft has moved on, faint when idle. `ruleState` and `statusLine` in `src/lib/tabStatus.ts` derive both from a tab's `TabView`. Under `prefers-reduced-motion` the rule is static (`motion-reduce:animate-none`, `opacity-50`), and the row's status line ("Analysing…", "Draft moved on", "Notes current", or the error in `text-fault`) carries the state. A folded project group shows the `animate-pulse-dot` dot while any of its tabs is analysing.
```

Insert these four bullets after it, before `- **Settings**`:

```markdown
- **Sidebar** (`Sidebar.tsx`, `TabRow.tsx`, `ClosedTabs.tsx`): this window's tabs, grouped by project in order of first appearance (`groupTabs` in `src/lib/tabList.ts`). Tabs without a project sit under "No project", and two projects with the same folder name also show their parent folder. ⌃Tab / ⌃⇧Tab follow this visual order, not the stored list, and closing the shown tab shows the next one in it (the previous one when it was last). A tab's title is its name, else the first line of its draft, else "Untitled"; double-click or F2 renames it, and a blank name returns to the automatic title. The context menu has Rename and Close. *Recently closed (n)* is shared by every window and grouped by project; Delete asks inline ("Delete for good?"), never with `confirm()`. The footer shows draft-save errors, "Couldn't read a tab; kept it as <file>.bad" notices, an optional `footer` node and the provider label.
- **Tab state lives outside React.** `src/lib/tabs.ts` is this window's store (`useSyncExternalStore`, like `downloads.ts`): the `WindowView` from `list_workspace`, the shown tab, and one `TabSession` (`src/lib/tabSession.ts`) per tab shown since launch, so a hidden tab keeps its pause timer and its run. It refetches on every `workspace-changed`. Draft edits save through one `createSaver` keyed by tab id (500 ms), flushed on a switch, before closing a tab and on `pagehide`. `src/lib/projects.ts` caches each project's summary, instructions and brief build for the window, and hands each project's `analysisContextKey` to its tabs (`tabs.setContext`) only once both the summary and the instructions have loaded: a key computed earlier (`root|0|`) would differ from the saved notes' and start a run at launch. A context change on a hidden tab only sets `contextDirty`, and `activate()` runs it when the tab is shown, so a rebuilt brief or edited instructions re-run only the tab on screen.
- **History:** each tab keeps its last 5 analyses. `TabSession.view(index)` browses them (`viewing` 0 is the live state, k ≥ 1 is `history[k]`). `done` refetches the history with `get_tab`, and `started` goes back to the latest. `NotesPane` shows `‹ 2 of 5 ›` and the time (`historyPosition` and `snapshotTime` in `src/lib/history.ts`; hidden with fewer than two), and renders an older analysis read-only through `snapshotAnalysis`, under "Notes on an earlier draft.", with its draft behind *Show that draft*.
- **Shortcuts** go through `useActions` (`src/lib/actions.ts`), one `keydown` listener per window: ⌘T new tab on the shown tab's project, ⌘O open a project, ⌘⇧T reopen the last closed tab, ⌘W close the tab, ⌃Tab / ⌃⇧Tab next and previous tab, ⌘\\ sidebar, ⌘, settings. Ctrl replaces ⌘ on Windows and Linux, and on macOS Ctrl+letter keeps its text-editing meaning (⌃T transposes). A key runs only the handlers `App.tsx` passes, calls `preventDefault()` only then, and does nothing while a modal `<dialog>` is open.
```

**AGENTS.md, "### Live analysis flow".** Replace step 1 (Phase 1's version begins ``1. `TabSession` (`src/lib/tabSession.ts`), one per tab and outside React``) with:

```markdown
1. Each tab's `TabSession` (`src/lib/tabSession.ts`) waits for a typing pause (`debounceMs`, default 1200), whether or not the tab is shown. It needs at least 24 characters after whitespace normalization and skips the call if the normalized draft hasn't changed. Its context key (`analysisContextKey` in `src/lib/instructions.ts`: the root, the brief's `createdAt` and the session instructions) changes when the project, brief or instructions do. Then the shown tab re-reads the same draft without the gate, and a hidden tab waits until it's shown. Every `analyze` call carries the tab id, the current instructions and the context key, which the saved snapshot keeps. Relaunching starts no run by itself: a restored tab whose draft matches its latest analysis doesn't, and the first context a tab with no saved analysis receives isn't a change. A project that can't be read hands its tabs only the instructions (`setContext(null, …)`), so they keep the context their saved notes ran with.
```

**AGENTS.md, "### Persistence".** Add at the end of the list:

```markdown
- Sidebar: hidden or shown per window, in `localStorage` (`livebrainstorm.sidebar.<windowId>`: `"1"` hidden, `"0"` shown), read and written inside `try`/`catch`. Without a stored choice, a window narrower than 1,100 px starts with it hidden.
```

**AGENTS.md, "## Cross-language contracts".** Add after the `- **Check events:**` line:

```markdown
- **Saved checks:** `Snapshot.checksTotal` is the `count` of the last `checking` event (the claims sent to Jev) and `checksFaults` its `total` (all Faults), as `Snapshot::apply_check` records them. `snapshotAnalysis` in `src/lib/history.ts` reads fewer checks than that, without `checksStopped`, as a run an edit cut off, and shows "Checks stopped when you edited." Change both together.
```

**README.md, "### Session instructions".** Replace:

```markdown
Click **Instructions** in the top bar to steer the notes for the project you have open, for example
```

with:

```markdown
Click **Instructions** in the top bar to steer the notes for the shown tab's project (every tab of a project shares them), for example
```

**README.md, a new subsection.** Insert this after the "### Session instructions" paragraph, before "### Requirements for the Claude subscription option". Phase 3 adds "### Windows" after it.

```markdown
### Tabs

The sidebar lists your tabs, grouped by project. Each tab has its own draft, notes and analysis, and tabs analyse at the same time: typing in one never stops another, and a tab you switch away from still gets its notes when you pause. A rule beside each tab shows what it's doing (animated while analysing, solid when its notes are current, dashed when the draft has moved on), and a line under its title says the same in words. **Open a project…** opens a new tab on a folder, **+** beside a project adds another tab to it, and **⋯** shows or rebuilds that project's brief, which all its tabs share. Double-click a tab to rename it. Closed tabs wait under *Recently closed*, where you can reopen or delete them at any later launch. Each tab keeps its last five analyses: ‹ and › above the notes show an earlier one and the draft it read. Drafts, notes and history survive restarts, and relaunching doesn't re-analyse a tab whose notes are current.

Shortcuts: ⌘T new tab, ⌘W close tab, ⌘⇧T reopen the last closed tab, ⌘O open a project, ⌃Tab and ⌃⇧Tab next and previous tab, ⌘\\ show or hide the sidebar, ⌘, settings. On Windows and Linux, use Ctrl instead of ⌘.
```

**README.md, "## How it works".** In the **Live analysis:** paragraph (Phase 1 leaves README.md unchanged), replace this sentence:

```markdown
A new request aborts the previous one in Rust (which kills the `claude` process too), and stale replies are dropped by request id.
```

replace it with:

```markdown
A new request aborts that tab's previous one in Rust (which kills its `claude` process too), never another tab's, and stale replies are dropped by request id.
```

Run: `npm run typecheck && npm test`
Expected: still green (docs only).

- [ ] **Step 8: Manual checklist (the user runs `npm run tauri dev`)**

The agent shell has Node 20.11, which Vite 7 can't run, so hand this list to the user. Commit (Step 9) without waiting for it; each failure goes back to the task named in brackets as a follow-up commit.

1. **Two tabs analysing at once** [Tasks 4, 6]: open two projects (*Open a project…* twice), and add a second tab to one of them with its **+**. Type a few sentences in one tab, switch to another before you pause, and type there too. Both rows show "Analysing…" with animated rules at the same time, neither run stops the other, and each tab ends with its own notes. Repeat with two tabs of the same project.
2. **Background pause** [Tasks 4, 6]: type a long sentence in tab A and click tab B within half a second. A's row turns to "Analysing…" while B is shown, then "Notes current". Switch back to A: its notes are there, and its draft has every character you typed (the save landed on A, not on B).
3. **Restart restores with no runs** [Tasks 4, 6, 7]: with several tabs whose rows say "Notes current", including one on a large project, plus a new tab (⌘T) where you typed a long sentence and quit within a second, so it was never analysed,, quit (⌘Q) and relaunch. The sidebar, drafts, notes and history come back, and for at least 10 seconds no row shows "Analysing…", even while the large project's summary loads. (Phase 4's usage footer will also show no new runs.)
4. **Brief and instructions with several tabs** [Tasks 4, 7]: with two tabs of one project, choose ⋯ → *Rebuild brief*. When it finishes, only the shown tab re-runs; the other shows "Analysing…" only once you switch to it. Edit that project's instructions: the same.
5. **Recently closed** [Tasks 6, 8]:
   - Close a tab with ⌘W, and another from its context menu (right-click → Close). Each time, the next tab in the sidebar shows (the previous one when it was last).
   - *Recently closed (2)* lists both, grouped by project. *Reopen* one: it returns at the end of its group, shown, with its draft and notes. ⌘⇧T reopens the other.
   - Close one again and choose *Delete*: "Delete for good?" appears inline. *No* keeps it; *Yes* removes it.
   - Close every tab: the window stays open and empty, offering *Open a project…*. Quit and relaunch: the closed tabs are still under *Recently closed*.
6. **History arrows** [Tasks 3, 4, 9]:
   - In one tab, change the idea and pause until three analyses have finished. The notes header shows "‹ 3 of 3 ›" and a time, and › is disabled.
   - ‹ shows "2 of 3": that analysis's notes, checks and notices under "Notes on an earlier draft.", and *Show that draft* expands the draft it read. ‹ again reaches "1 of 3", where ‹ is disabled.
   - Type and pause while viewing "1 of 3": the arrows stay, and as soon as new notes start streaming the view jumps back to the latest.
   - A tab with one analysis shows no arrows.
7. **Reduced motion** [Tasks 1, 8]: turn on System Settings → Accessibility → Display → Reduce motion. While a tab analyses, its row's rule and the rule between the panes are static and faded, and the row says "Analysing…". Fold that tab's project group: its header shows a dot that doesn't pulse.
8. **Rename** [Tasks 2, 8]: double-click a tab, type a name and press Enter: the row keeps it. Esc while renaming changes nothing. Rename it to blank: the row follows the draft's first line again.
9. **Shortcuts** [Tasks 5, 9]:
   - ⌘T opens a tab on the shown tab's project. ⌘W closes the tab and never the window. ⌃Tab and ⌃⇧Tab step through tabs in sidebar order, across groups, wrapping at the ends.
   - ⌘\\ hides and shows the sidebar, and the choice survives a relaunch. ⌘, opens Settings, and with Settings open ⌘W does nothing.
   - In the draft, ⌘C, ⌘V, ⌘Z and ⌃T (transpose) still work.
10. **A project folder that moved** [Task 7]: quit, rename a project's folder, relaunch, and show one of its tabs. Its draft and notes show and nothing re-runs (`pgrep -fl "claude -p"` prints nothing), the line under the top bar says the folder can't be read, and typing still gets notes, without code context.

- [ ] **Step 9: Commit**

Step 5's `git rm` already staged the deletions; `git add -u src` picks up any other tracked change under `src/`.

```bash
git add src/App.tsx src/components/NotesPane.tsx src/components/TopBar.tsx src/components/BriefDrawer.tsx AGENTS.md README.md
git add -u src
git status --short   # nothing left unstaged under src/
git commit -m "feat(ui): three-column window: tabs sidebar, history in the notes, a new top bar; docs

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01UsvRGgyYsgKYJj31W5vJ47"
```

