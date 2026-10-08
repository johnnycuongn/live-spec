import { describe, expect, it } from "vitest";
import { applyEvent, startAnalysis } from "./analysisState";
import type { UiEvent } from "./ipc";

const run = (events: UiEvent[]) => events.reduce(applyEvent, startAnalysis("draft"));
const check: UiEvent = { type: "check", claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("applyEvent", () => {
  it("streams text, then completes", () => {
    const s = run([{ type: "text", text: "## Intent\n" }, { type: "text", text: "- a" }, { type: "done", text: "" }]);
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

  it("a new run drops the previous checks and notice", () => {
    const before = run([{ type: "checking", count: 1, total: 1 }, check, { type: "notice", message: "x" }]);
    expect(before.checks).toHaveLength(1);
    const next = startAnalysis("new draft");
    expect(next.checks).toEqual([]);
    expect(next.checking).toBeNull();
    expect(next.notice).toBeNull();
    expect(next.status).toBe("thinking");
  });
});
