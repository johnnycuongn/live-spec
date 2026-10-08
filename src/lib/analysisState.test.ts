import { describe, expect, it } from "vitest";
import { applyEvent, initialAnalysis, lastAnalyzedAfter, previousDraftFor, startAnalysis } from "./analysisState";
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
    let last: string | null = null;
    last = lastAnalyzedAfter(last, { type: "started" }, "v1");
    expect(last).toBeNull();
    last = lastAnalyzedAfter(last, { type: "done", text: "" }, "v1");
    expect(last).toBe("v1");
    last = lastAnalyzedAfter(last, { type: "skipped" }, "v1, reworded");
    last = lastAnalyzedAfter(last, { type: "skipped" }, "v1, reworded again");
    expect(last).toBe("v1");
  });

  it("try again bypasses the gate", () => {
    expect(previousDraftFor("v1", true)).toBeNull();
    expect(previousDraftFor("v1", false)).toBe("v1");
    expect(previousDraftFor(null, false)).toBeNull();
  });
});
