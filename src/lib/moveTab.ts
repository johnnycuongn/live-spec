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
