import { describe, expect, it } from "vitest";
import type { UsageBucket } from "./ipc";
import {
  costText,
  footerText,
  formatCost,
  formatTokens,
  groupByLocalDay,
  lineLabel,
  projectName,
  rowCells,
  sinceHourFor,
  todayTotals,
  usageNotes,
  type DayUsage,
  type UsageTotals,
} from "./usage";

const HOUR = 3_600_000;
const UTC = () => 0;
const PDT = () => -420; // California in summer
const IST = () => 330; // India: a half-hour offset
const NOW = Date.parse("2026-10-14T20:00:00Z");

const bucket = (iso: string, fields: Partial<UsageBucket> = {}): UsageBucket => ({
  hour: Date.parse(iso) / HOUR,
  root: "/Users/me/code/shop-api",
  kind: "notes",
  model: "claude-haiku-5-5",
  input: 100,
  cacheWrite: 0,
  cacheRead: 0,
  output: 50,
  costUsd: 0.01,
  runs: 1,
  estimatedRuns: 0,
  estimatedCostUsd: 0,
  unpricedRuns: 0,
  ...fields,
});

const totals = (fields: Partial<UsageTotals>): UsageTotals => ({
  tokens: 0,
  costUsd: 0,
  estimatedCostUsd: 0,
  runs: 0,
  estimatedRuns: 0,
  unpricedRuns: 0,
  ...fields,
});

/** The days that had runs; the panel lists the quiet days too. */
const busy = (days: DayUsage[]) => days.filter((d) => d.runs > 0);

describe("groupByLocalDay", () => {
  it("puts each hour on the user's calendar day, not UTC's", () => {
    const b = [bucket("2026-10-14T05:00:00Z"), bucket("2026-10-14T08:00:00Z")];
    expect(busy(groupByLocalDay(b, NOW, UTC)).map((d) => d.day)).toEqual(["2026-10-14"]);
    // 05:00Z is 22:00 the evening before in California; 08:00Z is 01:00 the same day.
    expect(busy(groupByLocalDay(b, NOW, PDT)).map((d) => [d.day, d.runs])).toEqual([
      ["2026-10-14", 1],
      ["2026-10-13", 1],
    ]);
  });

  it("handles half-hour offsets", () => {
    // 20:00Z is 01:30 the next morning in India, where it's already the 15th at NOW.
    const days = groupByLocalDay([bucket("2026-10-13T20:00:00Z")], NOW, IST);
    expect(days[0].day).toBe("2026-10-15");
    expect(busy(days).map((d) => d.day)).toEqual(["2026-10-14"]);
  });

  it("splits a day by project and kind, merging models, largest first", () => {
    const [today, ...rest] = groupByLocalDay(
      [
        bucket("2026-10-14T09:00:00Z", { input: 10, output: 10 }),
        bucket("2026-10-14T10:00:00Z", { model: "claude-sonnet-5-5", input: 1000, cacheRead: 500, output: 100, costUsd: 0.2 }),
        bucket("2026-10-14T10:00:00Z", { kind: "brief", input: 5000, output: 900, costUsd: 0.5 }),
        bucket("2026-10-14T11:00:00Z", { root: null, input: 1, output: 1, costUsd: 0 }),
      ],
      NOW,
      UTC,
    );
    expect(today.day).toBe("2026-10-14");
    expect(busy(rest)).toEqual([]);
    expect(today.lines.map((l) => [l.root, l.kind, l.tokens, l.runs])).toEqual([
      ["/Users/me/code/shop-api", "brief", 5900, 1],
      ["/Users/me/code/shop-api", "notes", 1620, 2],
      [null, "notes", 2, 1],
    ]);
    expect(today.tokens).toBe(7522);
    expect(today.runs).toBe(4);
    expect(today.costUsd).toBeCloseTo(0.71, 10);
  });

  it("lists every one of the last 30 local days, today first, quiet days included", () => {
    const days = groupByLocalDay(
      [
        bucket("2026-09-14T12:00:00Z"), // 30 days back: dropped
        bucket("2026-09-15T12:00:00Z"), // 29 days back: kept
        bucket("2026-10-12T12:00:00Z"),
        bucket("2026-10-15T12:00:00Z"), // after today (a clock change): dropped
      ],
      NOW,
      UTC,
    );
    expect(days).toHaveLength(30);
    expect([days[0].day, days[1].day, days[29].day]).toEqual(["2026-10-14", "2026-10-13", "2026-09-15"]);
    expect(busy(days).map((d) => d.day)).toEqual(["2026-10-12", "2026-09-15"]);
    expect(days[1]).toEqual({ day: "2026-10-13", ...totals({}), lines: [] });
    expect(groupByLocalDay([], NOW, UTC).every((d) => d.runs === 0 && d.lines.length === 0)).toBe(true);
  });
});

describe("todayTotals", () => {
  it("sums today's local hours, cache tokens included", () => {
    const b = [
      bucket("2026-10-14T05:00:00Z", { input: 1200, cacheWrite: 22000, cacheRead: 90000, output: 3100, estimatedRuns: 1 }),
      bucket("2026-10-14T08:00:00Z", { input: 100, output: 0 }),
    ];
    expect(todayTotals(b, NOW, UTC)).toMatchObject({ tokens: 116400, runs: 2, estimatedRuns: 1 });
    expect(todayTotals(b, NOW, PDT)).toMatchObject({ tokens: 100, runs: 1, estimatedRuns: 0 });
    expect(todayTotals([], NOW, UTC)).toEqual(totals({}));
  });
});

describe("sinceHourFor", () => {
  it("starts an hour before local midnight, so the first local hour is never missed", () => {
    expect(sinceHourFor(NOW, 1, UTC)).toBe(Date.parse("2026-10-13T23:00:00Z") / HOUR);
    expect(sinceHourFor(NOW, 1, PDT)).toBe(Date.parse("2026-10-14T06:00:00Z") / HOUR);
    expect(sinceHourFor(NOW, 1, IST)).toBe(Date.parse("2026-10-14T17:00:00Z") / HOUR);
    expect(sinceHourFor(NOW, 30, UTC)).toBe(Date.parse("2026-09-14T23:00:00Z") / HOUR);
  });
});

describe("formatting", () => {
  it("formats token counts like 182k and 1.2M", () => {
    const cases: [number, string][] = [
      [0, "0"],
      [950, "950"],
      [1000, "1k"],
      [1234, "1.2k"],
      [9960, "10k"],
      [182_400, "182k"],
      [999_499, "999k"],
      [999_500, "1M"],
      [1_234_567, "1.2M"],
      [12_345_678, "12M"],
    ];
    for (const [n, text] of cases) expect(formatTokens(n), String(n)).toBe(text);
  });

  it("formats costs, with ~ for estimates and four places below a cent", () => {
    expect(formatCost(0.41, false)).toBe("$0.41");
    expect(formatCost(0.41, true)).toBe("~$0.41");
    expect(formatCost(0, false)).toBe("$0.00");
    expect(formatCost(0.00046765, false)).toBe("$0.0005");
    expect(formatCost(12.3456, false)).toBe("$12.35");
  });

  it("shows a dash when no run has a price", () => {
    expect(costText(totals({ runs: 3, unpricedRuns: 3 }))).toBe("—");
    expect(costText(totals({ runs: 3, unpricedRuns: 1, costUsd: 0.2 }))).toBe("$0.20");
    expect(costText(totals({ runs: 3, costUsd: 0.41, estimatedRuns: 1, estimatedCostUsd: 0.03 }))).toBe("~$0.41");
    expect(costText(totals({}))).toBe("$0.00");
  });

  it("shows a dash in every column of a row with no runs", () => {
    expect(rowCells(totals({}))).toEqual({ tokens: "—", cost: "—", runs: "—" });
    expect(rowCells(totals({ tokens: 182_400, costUsd: 0.41, runs: 14 }))).toEqual({ tokens: "182k", cost: "$0.41", runs: "14" });
    expect(rowCells(totals({ tokens: 900, runs: 2, unpricedRuns: 2 }))).toEqual({ tokens: "900", cost: "—", runs: "2" });
  });

  it("writes the footer line", () => {
    const t = totals({ tokens: 182_400, costUsd: 0.41, estimatedCostUsd: 0.03, runs: 14, estimatedRuns: 3 });
    expect(footerText(t)).toBe("Today · 182k tokens · ~$0.41");
  });

  it("labels estimates and unpriced runs", () => {
    expect(usageNotes(totals({ runs: 20, estimatedRuns: 12, estimatedCostUsd: 0.03 }))).toEqual(["incl. ~$0.03 from 12 cancelled runs"]);
    expect(usageNotes(totals({ runs: 3, estimatedRuns: 1, estimatedCostUsd: 0.0004, unpricedRuns: 2 }))).toEqual([
      "incl. ~$0.0004 from 1 cancelled run",
      "price unknown for 2 runs",
    ]);
    expect(usageNotes(totals({ runs: 5, costUsd: 1 }))).toEqual([]);
  });

  it("names projects by their folder", () => {
    expect(projectName("/Users/me/code/shop-api")).toBe("shop-api");
    expect(projectName("/Users/me/code/shop-api/")).toBe("shop-api");
    expect(projectName("C:\\code\\billing")).toBe("billing");
    expect(projectName(null)).toBe("No project");
    expect(lineLabel({ ...totals({}), root: "/code/shop-api", kind: "brief" })).toBe("shop-api · brief");
  });
});
