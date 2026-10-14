import { beforeEach, describe, expect, it, vi } from "vitest";

const win = vi.hoisted(() => ({ setTitle: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));

import { setWindowTitle } from "./windowTitle";

beforeEach(() => {
  win.setTitle.mockReset();
  vi.stubGlobal("document", { title: "" }); // vitest runs in node: there is no DOM
});

describe("setWindowTitle", () => {
  it("sets the native title, which Tauri doesn't copy from document.title", async () => {
    win.setTitle.mockResolvedValue(undefined);
    await setWindowTitle("LiveBrainstorm — Checkout retry");
    expect(win.setTitle).toHaveBeenCalledWith("LiveBrainstorm — Checkout retry");
    expect(document.title).toBe("LiveBrainstorm — Checkout retry");
  });

  it("a refused title is logged, never thrown", async () => {
    win.setTitle.mockRejectedValue("core:window:allow-set-title not allowed");
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(setWindowTitle("LiveBrainstorm")).resolves.toBeUndefined();
    expect(log).toHaveBeenCalledTimes(1);
    log.mockRestore();
  });
});
