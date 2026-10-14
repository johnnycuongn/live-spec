import { beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (e: { payload: unknown }) => void;
const tauri = vi.hoisted(() => ({
  globalListen: vi.fn(async (_event: string, _handler: Handler) => () => {}),
  windowListen: vi.fn(async (_event: string, _handler: Handler) => () => {}),
}));
vi.mock("@tauri-apps/api/event", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/event")>()),
  listen: tauri.globalListen,
}));
vi.mock("@tauri-apps/api/window", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/window")>()),
  getCurrentWindow: () => ({ label: "w-2", listen: tauri.windowListen }),
}));

import { onMenu } from "./events";

const names = (listen: typeof tauri.globalListen) => listen.mock.calls.map(([name]) => name);

beforeEach(() => {
  tauri.globalListen.mockClear();
  tauri.windowListen.mockClear();
});

describe("cross-window events", () => {
  it("menu items are heard by this window only, never through a global listener", async () => {
    const seen: string[] = [];
    await onMenu((e) => seen.push(e.id));
    expect(names(tauri.windowListen)).toEqual(["menu"]);
    expect(names(tauri.globalListen)).not.toContain("menu");
    tauri.windowListen.mock.calls[0][1]({ payload: { id: "close-tab" } });
    expect(seen).toEqual(["close-tab"]);
  });
});
