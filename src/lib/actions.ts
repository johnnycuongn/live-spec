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

/** Holding a key auto-repeats it; only stepping through tabs should repeat. */
const REPEATABLE: ReadonlySet<ActionId> = new Set(["next-tab", "prev-tab"]);

type KeyDown = KeyPress & { repeat?: boolean; preventDefault: () => void };

/**
 * Runs the handler for a recognised key press and returns whether the key was
 * taken. An auto-repeat of anything but ⌃Tab is swallowed without running, so
 * holding ⌘W closes one tab, not all of them.
 */
export function dispatchKey(e: KeyDown, handlers: Partial<Record<ActionId, () => void>>, mac: boolean): boolean {
  const action = keyToActionFor(e, mac);
  const run = action ? handlers[action] : undefined;
  if (!action || !run) return false;
  e.preventDefault();
  if (!e.repeat || REPEATABLE.has(action)) run();
  return true;
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
      if (document.querySelector("dialog[open]")) return;
      dispatchKey(e, current.current, IS_MAC);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
