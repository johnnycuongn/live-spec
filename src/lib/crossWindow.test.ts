import { describe, expect, it } from "vitest";
import { refreshInstructions, rootsToRefetch, type CrossWindowDeps } from "./crossWindow";

describe("rootsToRefetch", () => {
  const cached = ["/a", "/b", ""];

  it("never refetches this window's own save", () => {
    expect(rootsToRefetch({ root: "/a", source: "w-1" }, "w-1", cached)).toEqual([]);
    expect(rootsToRefetch({ root: "", source: "w-1" }, "w-1", cached)).toEqual([]);
  });

  it("refetches the saved root only if this window has it", () => {
    expect(rootsToRefetch({ root: "/a", source: "w-2" }, "w-1", cached)).toEqual(["/a"]);
    expect(rootsToRefetch({ root: "/c", source: "w-2" }, "w-1", cached)).toEqual([]);
  });

  it("a templates change refetches every cached root", () => {
    expect(rootsToRefetch({ root: "", source: "w-2" }, "w-1", cached)).toEqual(["/a", "/b", ""]);
  });
});

describe("refreshInstructions", () => {
  const deps = (log: string[], cached: string[]): CrossWindowDeps => ({
    windowId: "w-1",
    cachedRoots: () => cached,
    flushInstructions: async () => void log.push("flush"),
    refetchInstructions: async (root) => void log.push(`refetch ${root}`),
    refetchSettings: async () => void log.push("settings"),
  });

  it("saves this window's pending edit before refetching", async () => {
    const log: string[] = [];
    await refreshInstructions({ root: "/a", source: "w-2" }, deps(log, ["/a", "/b"]));
    expect(log).toEqual(["flush", "refetch /a"]);
  });

  it("does nothing for its own save, or a root it doesn't show", async () => {
    const log: string[] = [];
    await refreshInstructions({ root: "/a", source: "w-1" }, deps(log, ["/a"]));
    await refreshInstructions({ root: "/z", source: "w-2" }, deps(log, ["/a"]));
    expect(log).toEqual([]);
  });
});
