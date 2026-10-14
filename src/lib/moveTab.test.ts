import { describe, expect, it, vi } from "vitest";
import type { TabSummary, WindowSummary } from "./ipc";
import { moveMenuItems, moveTabTo, moveTargets } from "./moveTab";

describe("moveTabTo", () => {
  it("saves the pending draft first, and drops the tab here only after the move", async () => {
    const order: string[] = [];
    const deps = {
      flush: vi.fn(async () => void order.push("flush")),
      move: vi.fn(async (_tabId: string, _to: string | null) => {
        order.push("move");
        return "w-3";
      }),
      forget: vi.fn((_tabId: string) => void order.push("forget")),
    };
    await expect(moveTabTo("t-1", null, deps)).resolves.toBe("w-3");
    expect(order).toEqual(["flush", "move", "forget"]);
    expect(deps.move).toHaveBeenCalledWith("t-1", null);
    expect(deps.forget).toHaveBeenCalledWith("t-1");
  });

  it("waits for a slow save before moving", async () => {
    let saved!: () => void;
    const flush = () => new Promise<void>((resolve) => (saved = resolve));
    const move = vi.fn(async () => "w-2");
    const moving = moveTabTo("t-1", "w-2", { flush, move, forget: () => {} });
    await Promise.resolve();
    expect(move).not.toHaveBeenCalled();
    saved();
    await moving;
    expect(move).toHaveBeenCalledTimes(1);
  });

  it("a failed move keeps the tab in this window", async () => {
    const forget = vi.fn();
    const move = async (): Promise<string> => {
      throw "This tab was closed.";
    };
    await expect(moveTabTo("t-1", "w-2", { flush: async () => {}, move, forget })).rejects.toBe("This tab was closed.");
    expect(forget).not.toHaveBeenCalled();
  });
});

describe("moveTargets", () => {
  const tab = (id: string, title: string | null, firstLine: string): TabSummary => ({
    id,
    root: null,
    title,
    firstLine,
    updatedAt: 0,
    closedAt: null,
  });
  const win = (id: string, active: TabSummary | null, tabs = active ? 1 : 0): WindowSummary => ({ id, active, tabs });

  it("lists the other windows, named by their active tab", () => {
    const windows = [win("w-1", tab("t-1", "Sidebar tabs", "")), win("w-2", tab("t-2", null, "Checkout retry")), win("w-3", null)];
    expect(moveTargets(windows, "w-1")).toEqual([
      { id: "w-2", name: "Checkout retry" },
      { id: "w-3", name: "Empty window" },
    ]);
  });

  it("numbers repeated names, and offers nothing with one window", () => {
    const windows = [win("w-1", null), win("w-4", tab("t-4", null, "")), win("w-6", tab("t-6", null, "")), win("w-7", null)];
    expect(moveTargets(windows, "w-1").map((t) => t.name)).toEqual(["Untitled", "Untitled (2)", "Empty window"]);
    expect(moveTargets([win("w-1", null)], "w-1")).toEqual([]);
  });
});

describe("moveMenuItems", () => {
  it("offers a new window first, then each other window by name", () => {
    const moves: (string | null)[] = [];
    const items = moveMenuItems(
      [
        { id: "w-2", name: "Checkout retry" },
        { id: "w-3", name: "Empty window" },
      ],
      (to) => void moves.push(to),
    );
    expect(items.map((i) => i.label)).toEqual(["Move to new window", "Move to “Checkout retry”", "Move to “Empty window”"]);
    items.forEach((i) => i.onSelect());
    expect(moves).toEqual([null, "w-2", "w-3"]);
  });
});
