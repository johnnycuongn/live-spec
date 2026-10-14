// Actions for tabs and windows. The native app menu emits the same ids as `menu`
// events and owns their shortcuts; keydown handles only the keys it doesn't own.
import { useEffect, useRef } from "react";
import { onMenu } from "./events";

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
export function dispatchMenu(
  id: string,
  handlers: Partial<Record<ActionId, () => void>>,
  modalOpen: boolean,
): ActionId | null {
  const action = menuAction(id);
  if (action === null || modalOpen) return null;
  handlers[action]?.();
  return action;
}

/**
 * Binds the actions: this window's app-menu items, plus any keydown the menu doesn't own
 * (through `dispatchKey`, so auto-repeat is still swallowed). Neither runs while a modal
 * dialog (Settings) is open, so ⌘W never closes a tab behind it.
 */
export function useActions(handlers: Partial<Record<ActionId, () => void>>): void {
  const current = useRef(handlers);
  current.current = handlers;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (keydownAction(e) === null || document.querySelector("dialog[open]")) return;
      dispatchKey(e, current.current, IS_MAC);
    };
    window.addEventListener("keydown", onKey);
    const menu = onMenu(({ id }) => {
      dispatchMenu(id, current.current, document.querySelector("dialog[open]") !== null);
    });
    return () => {
      window.removeEventListener("keydown", onKey);
      void menu.then((unlisten) => unlisten());
    };
  }, []);
}
