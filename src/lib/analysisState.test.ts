import { describe, expect, it } from "vitest";
import { applyEvent, initialAnalysis, lastAnalyzedAfter, previousDraftFor, startAnalysis, type Analyzed } from "./analysisState";
import { checkSummary } from "./checks";
import type { UiEvent } from "./ipc";

const run = (events: UiEvent[]) => events.reduce(applyEvent, startAnalysis(initialAnalysis, "draft"));
const check: UiEvent = { type: "check", claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("applyEvent", () => {
  it("streams text, then completes", () => {
    const s = run([{ type: "started" }, { type: "text", text: "## Intent\n" }, { type: "text", text: "- a" }, { type: "done", text: "" }]);
    expect(s.status).toBe("done");
    expect(s.markdown).toBe("## Intent\n- a");
  });

  it("keeps applying checks after done", () => {
    const s = run([{ type: "done", text: "## Faults\n- `x` is wrong" }, { type: "checking", count: 1, total: 2 }, check]);
    expect(s.status).toBe("done");
    expect(s.checking).toEqual({ count: 1, total: 2 });
    expect(s.checks).toEqual([{ claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 }]);
  });

  it("a notice never turns the notes into an error", () => {
    const s = run([{ type: "done", text: "## Intent\n- a" }, { type: "notice", message: "Couldn't check" }]);
    expect(s.status).toBe("done");
    expect(s.markdown).toBe("## Intent\n- a");
    expect(s.notice).toBe("Couldn't check");
  });

  it("keeps the old notes visible until the new run starts", () => {
    const before = run([{ type: "done", text: "## Intent\n- old" }, { type: "checking", count: 1, total: 1 }, check, { type: "notice", message: "x" }]);
    const pending = startAnalysis(before, "new draft");
    expect(pending.status).toBe("thinking");
    expect(pending.markdown).toBe("## Intent\n- old");
    const started = applyEvent(pending, { type: "started" });
    expect(started.markdown).toBe("");
    expect(started.checks).toEqual([]);
    expect(started.checking).toBeNull();
    expect(started.notice).toBeNull();
  });

  it("a skipped pause keeps the notes and shows them as current", () => {
    const before = run([{ type: "done", text: "## Intent\n- a" }]);
    const s = applyEvent(startAnalysis(before, "draft, reworded"), { type: "skipped" });
    expect(s.status).toBe("done");
    expect(s.markdown).toBe("## Intent\n- a");
    expect(s.analyzedDraft).toBe("draft, reworded");
  });
});

describe("previous draft for the gate", () => {
  it("moves only when an analysis completes", () => {
    let last: Analyzed | null = null;
    last = lastAnalyzedAfter(last, { type: "done", text: "" }, "v1", "A");
    expect(last).toEqual({ draft: "v1", context: "A" });
    last = lastAnalyzedAfter(last, { type: "skipped" }, "v1, reworded", "A");
    last = lastAnalyzedAfter(last, { type: "skipped" }, "v1, reworded again", "A");
    expect(previousDraftFor(last, false, "A")).toBe("v1");
  });

  it("try again bypasses the gate", () => {
    const last: Analyzed = { draft: "v1", context: "A" };
    expect(previousDraftFor(last, true, "A")).toBeNull();
    expect(previousDraftFor(last, false, "A")).toBe("v1");
    expect(previousDraftFor(null, false, "A")).toBeNull();
  });

  it("once a run has started, the next pause always re-analyses", () => {
    // v1 done, v2 starts streaming (old notes cleared), then the user reverts to v1.
    let last: Analyzed | null = lastAnalyzedAfter(null, { type: "done", text: "" }, "v1", "A");
    last = lastAnalyzedAfter(last, { type: "started" }, "v2", "A");
    expect(previousDraftFor(last, false, "A")).toBeNull();
  });

  it("an analysis that finishes for another project or brief never gates this one", () => {
    const last = lastAnalyzedAfter(null, { type: "done", text: "" }, "draft", "project-A|brief-1");
    expect(previousDraftFor(last, false, "project-A|brief-2")).toBeNull();
  });
});

describe("checks interrupted by a skipped pause", () => {
  it("stop claiming to be checking", () => {
    const s = applyEvent(
      run([{ type: "done", text: "## Faults\n- `x`" }, { type: "checking", count: 4, total: 4 }, check]),
      { type: "skipped" },
    );
    expect(s.status).toBe("done");
    expect(s.notice).toBe("Checks stopped when you edited.");
    expect(checkSummary(s.checking!, s.checks.length, !!s.notice)).toBe("1 of 4 faults checked");
  });

  it("finished checks are left alone", () => {
    const s = applyEvent(run([{ type: "done", text: "" }, { type: "checking", count: 1, total: 1 }, check]), { type: "skipped" });
    expect(s.notice).toBeNull();
  });
});
