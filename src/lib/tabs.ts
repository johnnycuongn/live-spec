// This window's tabs: the workspace view from the backend, which tab is
// shown, and one TabSession per tab shown since launch. Sessions live here,
// outside React, so a hidden tab keeps its pause timer and its run. A
// per-window module store read through useSyncExternalStore, like downloads.ts.
import { useSyncExternalStore } from "react";
import { onWorkspaceChanged } from "./events";
import { api, errorMessage, type Tab, type WindowView } from "./ipc";
import { browserStorage, importLegacy } from "./legacyDrafts";
import { moveTabTo } from "./moveTab";
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
let wired = false;
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
      if (state.saveError && saver.owed() === 0) set({ saveError: null }); // another tab may still be owed
    }),
  500,
  (e) => set({ saveError: `Couldn't save your draft: ${errorMessage(e)}` }),
);

const deps: TabSessionDeps = {
  analyze: (tabId, draft, previousDraft, instructions, sections, context, onEvent) =>
    api.analyze(tabId, draft, previousDraft, instructions, sections, context, onEvent),
  saveDraft: (tabId, draft) => saver.schedule(tabId, draft),
  debounceMs: () => debounce,
  getTab: (tabId) => api.getTab(tabId),
};

const rootKey = (root: string | null) => root ?? "";

/** The tab's session, created on first show with its project's context, if known yet. */
function sessionOf(tab: Tab): TabSession {
  let s = sessions.get(tab.id);
  if (!s) {
    s = new TabSession(tab, deps, { awaitContext: true }); // no run until the window hands over its context
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
  const shown = state.activeId;
  if (shown === null || !view.tabs.some((t) => t.id === shown)) {
    let target = view.active;
    if (!target && view.tabs.length > 0) {
      // Tabs but no active one (the backend lost track): show the first, as the single-pane window did.
      const id = view.tabs[0].id;
      await api.activateTab(state.windowId, id);
      target = await api.getTab(id);
      if (n !== refreshing) return;
    }
    const next = target ? sessionOf(target) : null;
    if ((next?.id ?? null) !== shown) show(next);
  }
}

async function load(): Promise<void> {
  if (loaded) return; // StrictMode runs effects twice; there is one store per window
  loaded = true;
  set({ windowId: currentWindowId() });
  if (!wired) {
    wired = true;
    // Closing the window doesn't unmount React, so flush when the page goes away.
    if (typeof window !== "undefined") window.addEventListener("pagehide", () => void saver.flush());
    // Any session change, in this window or another, refetches this window's list and the shared closed list.
    void onWorkspaceChanged(() => void refresh().catch(() => {}));
  }
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
    loaded = false; // a failed first load is retried by the next load()
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
    // A tab opened from the shown tab's project starts with its sections; elsewhere, the default set.
    const shown = state.activeId !== null ? sessions.get(state.activeId) : undefined;
    const sections = shown && shown.root === root ? shown.getState().sections : [];
    const tab = await api.createTab(state.windowId, root, sections); // appended to the window and made active
    show(sessionOf(tab));
    await refresh();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

/** Saves the tab's pending draft before it leaves this window's session (closed or moved), and
 *  says so when it can't: the stored copy would be older than the text on screen. Only this
 *  tab's save counts: another tab's owed save does not stop a healthy tab from closing. */
async function draftSaved(tabId: string, action: string): Promise<boolean> {
  if (await saver.flush(tabId)) return true;
  set({ error: `Couldn't save your draft, so the tab was not ${action}.` });
  return false;
}

async function close(tabId: string): Promise<void> {
  const wasShown = tabId === state.activeId;
  const next = wasShown ? afterClose(order(), tabId) : null;
  set({ error: null });
  if (!(await draftSaved(tabId, "closed"))) return; // before switching: a failed save leaves everything as it is
  // Show the next tab first. Closing the shown tab makes the backend activate its neighbour in the
  // *stored* order, and the workspace-changed refresh would show that tab for a moment.
  if (next) await switchTo(next);
  await saver.flush(tabId);
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

/**
 * Moves a tab to window `to`, or to a new window (null). The pending draft is saved first,
 * and a failed move leaves the tab here. Moving the shown tab shows the next one in the
 * sidebar's order, as closing does. The shell stops the tab's run; the other window shows it.
 */
async function move(tabId: string, to: string | null): Promise<void> {
  const wasShown = tabId === state.activeId;
  const next = wasShown ? afterClose(order(), tabId) : null;
  set({ error: null });
  if (!(await draftSaved(tabId, "moved"))) return;
  // As closing does: show the next tab first, so the backend's stored-order pick never flashes.
  if (next) await switchTo(next);
  try {
    await moveTabTo(tabId, to, {
      flush: (id) => saver.flush(id),
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

/** Saves the tab's section choice, then applies it to its session (which re-runs the shown tab). */
async function setSections(tabId: string, ids: string[]): Promise<void> {
  set({ error: null });
  try {
    await api.setTabSections(tabId, ids);
    sessions.get(tabId)?.setSections(ids);
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}

export const tabs = {
  load,
  switchTo,
  newTab,
  close,
  move,
  reopen,
  remove,
  rename,
  setSections,
  next,
  refresh,
  setContext,
  setDebounce: (ms: number) => {
    debounce = ms;
  },
  /** Saves pending draft edits now, and retries failed ones: every tab's before closing this
   *  window, or just `tabId`'s. False if any (that tab's) save failed. */
  flush: (tabId?: string) => saver.flush(tabId),
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
