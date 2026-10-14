import { describe, expect, it } from "vitest";
import type { ProjectSummary } from "./ipc";
import { tabContext } from "./tabContext";

const summary = (createdAt: number | null): ProjectSummary => ({
  name: "p",
  root: "/p",
  indexedFiles: 1,
  totalFiles: 1,
  briefStatus: createdAt ? "fresh" : "missing",
  brief: createdAt ? { root: "/p", fingerprint: "f", text: "brief", createdAt, source: "test" } : null,
});

describe("tabContext", () => {
  it("waits for both the project summary and the instructions", () => {
    expect(tabContext("/p", { root: "/p", summary: null, error: null }, { root: "/p", current: "Focus." })).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, null)).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, { root: "/p", current: " Focus. " })).toEqual({
      key: "/p|7|Focus.",
      instructions: " Focus. ",
    });
  });

  it("never mixes in another project's data", () => {
    expect(tabContext("/p", { root: "/q", summary: summary(7), error: null }, { root: "/p", current: "" })).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, { root: "/q", current: "" })).toBeNull();
    expect(tabContext("/p", { root: "/p", summary: summary(7), error: null }, { root: "", current: "" })).toBeNull();
  });

  it("a project that couldn't be read analyses without its brief", () => {
    const gone = { root: "/p", summary: null, error: "That folder no longer exists. Open it again from its new place." };
    expect(tabContext("/p", gone, { root: "/p", current: "" })).toEqual({ key: "/p|0|", instructions: "" });
  });

  it("a tab with no project needs only the no-project instructions", () => {
    expect(tabContext(null, null, { root: "", current: "Be brief." })).toEqual({ key: "|0|Be brief.", instructions: "Be brief." });
    expect(tabContext(null, null, { root: "/p", current: "" })).toBeNull();
  });

  it("a project with no brief yet uses 0, as a saved analysis did", () => {
    expect(tabContext("/p", { root: "/p", summary: summary(null), error: null }, { root: "/p", current: "" })).toEqual({
      key: "/p|0|",
      instructions: "",
    });
  });
});
