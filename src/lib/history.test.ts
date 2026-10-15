import { describe, expect, it } from "vitest";
import { checkSummary } from "./checks";
import { historyPosition, snapshotAnalysis, snapshotTime } from "./history";
import type { CheckResult, Snapshot } from "./ipc";

const snap = (over: Partial<Snapshot> = {}): Snapshot => ({
  id: 3,
  at: 1760443320,
  draft: "the draft",
  context: "/p|1|",
  markdown: "## Intent\n- a",
  partial: false,
  notices: [],
  checks: [],
  checksTotal: null,
  checksFaults: null,
  checksStopped: false,
  ...over,
});
const check: CheckResult = { claim: "`x` is wrong", verdict: "confirmed", confidence: 0.9, path: "src/x.ts", line: 3 };

describe("snapshotAnalysis", () => {
  it("shows the saved notes as done", () => {
    const a = snapshotAnalysis(snap({ partial: true, notices: ["Couldn't write Questions."] }));
    expect(a.status).toBe("done");
    expect(a.markdown).toBe("## Intent\n- a");
    expect(a.analyzedDraft).toBe("the draft");
    expect(a.partial).toBe(true);
    expect(a.notices).toEqual(["Couldn't write Questions."]);
    expect(a.checking).toBeNull();
  });

  it("checks cut off by an edit read as stopped, as they did live", () => {
    const a = snapshotAnalysis(snap({ checks: [check], checksTotal: 3, checksFaults: 5 }));
    expect(a.checksStopped).toBe(true);
    expect(a.notices).toEqual(["Checks stopped when you edited."]);
    // Live, 3 of the 5 Conflicts named code; the saved notes read the same way.
    expect(checkSummary(a.checking!, a.checks.length, a.checksStopped)).toBe("1 of 5 conflicts checked");
  });

  it("finished or failed checks stay as they were", () => {
    const finished = snapshotAnalysis(snap({ checks: [check], checksTotal: 1 }));
    expect(finished.checksStopped).toBe(false);
    expect(finished.notices).toEqual([]);
    // A file saved without checksFaults counts against the claims.
    expect(checkSummary(finished.checking!, 1, false)).toBe("1 of 1 conflict checked");
    const failed = snapshotAnalysis(snap({ checksTotal: 2, checksStopped: true, notices: ["Jev is unavailable."] }));
    expect(failed.checksStopped).toBe(true);
    expect(failed.notices).toEqual(["Jev is unavailable."]);
  });
});

describe("historyPosition", () => {
  it("is hidden with fewer than two analyses", () => {
    expect(historyPosition(0, 0).show).toBe(false);
    expect(historyPosition(0, 1).show).toBe(false);
  });

  it("numbers chronologically, so the latest is n of n and ‹ goes back in time", () => {
    expect(historyPosition(0, 5)).toEqual({ show: true, label: "5 of 5", earlier: 1, later: null });
    expect(historyPosition(3, 5)).toEqual({ show: true, label: "2 of 5", earlier: 4, later: 2 });
    expect(historyPosition(4, 5)).toEqual({ show: true, label: "1 of 5", earlier: null, later: 3 });
    expect(historyPosition(9, 5).label).toBe("1 of 5");
  });
});

describe("snapshotTime", () => {
  it("adds the date only for another day", () => {
    const at = Date.UTC(2026, 9, 14, 12, 2) / 1000;
    const time = snapshotTime(at, new Date(at * 1000 + 60_000));
    const otherDay = snapshotTime(at, new Date(at * 1000 + 8 * 86_400_000));
    expect(time.length).toBeGreaterThan(0);
    expect(otherDay).not.toBe(time);
    expect(otherDay.endsWith(time)).toBe(true);
  });
});
