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
