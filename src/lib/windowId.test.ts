import { describe, expect, it, vi } from "vitest";

const getCurrentWindow = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow }));

import { currentWindowId } from "./windowId";

describe("currentWindowId", () => {
  it("is the window's label when it is a workspace id", () => {
    getCurrentWindow.mockReturnValue({ label: "w-3" });
    expect(currentWindowId()).toBe("w-3");
  });

  it("is w-1 for the static main window, and outside Tauri", () => {
    getCurrentWindow.mockReturnValue({ label: "main" });
    expect(currentWindowId()).toBe("w-1");
    getCurrentWindow.mockImplementation(() => {
      throw new TypeError("no Tauri internals");
    });
    expect(currentWindowId()).toBe("w-1");
  });
});
