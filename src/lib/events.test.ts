import { beforeEach, describe, expect, it, vi } from "vitest";

const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { onBriefStatus, onWorkspaceChanged } from "./events";

/** Calls the handler the n-th `listen` was given, as Tauri would. */
const deliver = (n: number, payload: unknown) => listen.mock.calls[n][1]({ event: listen.mock.calls[n][0], id: 1, payload });

beforeEach(() => {
  listen.mockReset();
  listen.mockResolvedValue(() => {});
});

describe("events", () => {
  it("brief-status hands its payload to the handler", async () => {
    const got: unknown[] = [];
    const unlisten = await onBriefStatus((e) => got.push(e));
    expect(typeof unlisten).toBe("function");
    expect(listen.mock.calls[0][0]).toBe("brief-status");
    deliver(0, { root: "/p", state: "done", message: null });
    expect(got).toEqual([{ root: "/p", state: "done", message: null }]);
  });

  it("workspace-changed hands its payload to the handler", async () => {
    const got: unknown[] = [];
    await onWorkspaceChanged((e) => got.push(e));
    expect(listen.mock.calls[0][0]).toBe("workspace-changed");
    deliver(0, { windows: ["w-1"] });
    expect(got).toEqual([{ windows: ["w-1"] }]);
  });
});
