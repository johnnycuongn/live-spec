import { describe, expect, it } from "vitest";
import { applyEvent, initialAnalysis, lastAnalyzedAfter, previousDraftFor, startAnalysis, type Analyzed } from "./analysisState";
import { checkSummary } from "./checks";
import type { UiEvent } from "./ipc";

const run = (events: UiEvent[]) => events.reduce(applyEvent, startAnalysis(initialAnalysis, "draft"));
const check: UiEvent = { type: "check", claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("applyEvent", () => {
  it("streams text, then completes", () => {
    const s = run([{ type: "started" }, { type: "text", text: "## Intent\n", lane: 0 }, { type: "text", text: "- a", lane: 0 }, { type: "done", text: "" }]);
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
    expect(s.notices).toEqual(["Couldn't check"]);
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
    expect(started.notices).toEqual([]);
    expect(started.checksStopped).toBe(false);
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
    expect(s.notices).toEqual(["Checks stopped when you edited."]);
    expect(s.checksStopped).toBe(true);
    expect(checkSummary(s.checking!, s.checks.length, s.checksStopped)).toBe("1 of 4 conflicts checked");
  });

  it("finished checks are left alone", () => {
    const s = applyEvent(run([{ type: "done", text: "" }, { type: "checking", count: 1, total: 1 }, check]), { type: "skipped" });
    expect(s.notices).toEqual([]);
    expect(s.checksStopped).toBe(false);
  });
});

describe("parallel lanes", () => {
  it("builds the notes from lanes in lane order", () => {
    const s = run([
      { type: "started" },
      { type: "text", text: "## Directions\n- d", lane: 1 },
      { type: "text", text: "## Intent\n- i", lane: 0 },
    ]);
    expect(s.markdown).toBe("## Intent\n- i\n## Directions\n- d");
  });

  it("started clears the lane buffers too", () => {
    const s = run([{ type: "started" }, { type: "text", text: "## Intent\n- old", lane: 0 }, { type: "done", text: "## Intent\n- old" }]);
    const next = applyEvent(startAnalysis(s, "v2"), { type: "started" });
    expect(applyEvent(next, { type: "text", text: "## Faults\n- new", lane: 1 }).markdown).toBe("## Faults\n- new");
  });
});

describe("notices", () => {
  it("keeps every notice, not just the last", () => {
    const s = run([
      { type: "started" },
      { type: "notice", message: "Couldn't write Directions and Questions: rate limited" },
      { type: "checksStopped", message: "Couldn't check the notes against the code: timed out" },
    ]);
    expect(s.notices).toEqual([
      "Couldn't write Directions and Questions: rate limited",
      "Couldn't check the notes against the code: timed out",
    ]);
  });

  it("a lane notice never marks running checks as stopped", () => {
    const s = run([{ type: "checking", count: 3, total: 3 }, { type: "notice", message: "Couldn't write Questions: busy" }]);
    expect(s.checksStopped).toBe(false);
    expect(checkSummary(s.checking!, s.checks.length, s.checksStopped)).toBe("Checking 3 of 3 conflicts…");
  });

  it("a failed check stops the checks", () => {
    const s = run([{ type: "checking", count: 3, total: 3 }, { type: "checksStopped", message: "Couldn't check" }]);
    expect(s.checksStopped).toBe(true);
  });
});

describe("partly written notes", () => {
  it("offer a retry and never count as analysed", () => {
    const s = run([{ type: "started" }, { type: "done", text: "## Intent\n- i", partial: true }]);
    expect(s.partial).toBe(true);
    expect(lastAnalyzedAfter({ draft: "v0", context: "A" }, { type: "done", text: "", partial: true }, "v1", "A")).toBeNull();
    expect(lastAnalyzedAfter(null, { type: "done", text: "", partial: false }, "v1", "A")).toEqual({ draft: "v1", context: "A" });
  });

  it("a fresh run clears the retry offer", () => {
    const s = run([{ type: "done", text: "x", partial: true }, { type: "started" }]);
    expect(s.partial).toBe(false);
  });
});
