import { describe, expect, it } from "vitest";
import { initialAnalysis, type AnalysisState } from "./analysisState";
import { anyThinking, ruleState, statusLine } from "./tabStatus";

const tab = (status: AnalysisState["status"], stale = false, error: string | null = null) => ({
  analysis: { ...initialAnalysis, status, error },
  stale,
});

describe("ruleState", () => {
  it("follows the rule between the panes", () => {
    expect(ruleState(null)).toBe("idle");
    expect(ruleState(tab("idle"))).toBe("idle");
    expect(ruleState(tab("thinking", true))).toBe("thinking");
    expect(ruleState(tab("done", true))).toBe("stale");
    expect(ruleState(tab("done"))).toBe("fresh");
    expect(ruleState(tab("error"))).toBe("idle");
  });
});

describe("statusLine", () => {
  it("says what the tab is doing", () => {
    expect(statusLine(tab("thinking"))).toEqual({ text: "Analysing…", fault: false });
    expect(statusLine(tab("done", true))).toEqual({ text: "Draft moved on", fault: false });
    expect(statusLine(tab("done"))).toEqual({ text: "Notes current", fault: false });
    expect(statusLine(tab("idle"))).toBeNull();
    expect(statusLine(null)).toBeNull();
  });

  it("shows an error's first line as a fault", () => {
    expect(statusLine(tab("error", false, "\nClaude Code isn't installed.\nInstall it, then try again."))).toEqual({
      text: "Claude Code isn't installed.",
      fault: true,
    });
    expect(statusLine(tab("error"))).toEqual({ text: "Couldn't think this through", fault: true });
  });
});

describe("anyThinking", () => {
  it("is true when any tab in a group is analysing", () => {
    expect(anyThinking([null, tab("done"), tab("thinking")])).toBe(true);
    expect(anyThinking([undefined, tab("done", true)])).toBe(false);
    expect(anyThinking([])).toBe(false);
  });
});
