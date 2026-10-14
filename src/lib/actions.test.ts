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
