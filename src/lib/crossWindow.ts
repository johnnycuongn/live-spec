// Settings, instructions and templates are shared by every window, and the last save
// wins. Another window's save arrives as an event, and this window refetches what it shows.
import { useEffect, useRef } from "react";
import { onInstructionsChanged, onSettingsChanged, type InstructionsChangedEvent } from "./events";

export interface CrossWindowDeps {
  /** This window's id (`currentWindowId()`), to recognise its own saves. */
  windowId: string;
  /** The roots whose instructions this window has loaded ("" is the no-project instructions). */
  cachedRoots: () => Iterable<string>;
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

/** Refetches what another window saved. The refetch itself saves this window's pending edit
 *  first (`projects.reloadInstructions`), so the last save really is the last. */
export async function refreshInstructions(e: InstructionsChangedEvent, d: CrossWindowDeps): Promise<void> {
  const roots = rootsToRefetch(e, d.windowId, d.cachedRoots());
  if (roots.length === 0) return;
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
