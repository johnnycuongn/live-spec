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
