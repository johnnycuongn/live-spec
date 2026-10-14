// The usage ledger keeps hourly UTC buckets. Days are the user's own, so
// grouping by day happens here, on the user's clock, not in Rust.
import type { UsageBucket, UsageKind } from "./ipc";

const HOUR = 3_600_000;
const DAY = 86_400_000;

/** How many local days the usage panel covers, today included. */
export const DAYS_SHOWN = 30;

/** Minutes east of UTC at instant `ms`: +330 in India, -420 in California in summer. */
export type Offset = (ms: number) => number;

/** The user's own offset, daylight saving included. */
export const localOffset: Offset = (ms) => -new Date(ms).getTimezoneOffset();

export interface UsageTotals {
  /** Input + cache write + cache read + output. */
  tokens: number;
  /** Estimates included; 0 for runs with no known price. */
  costUsd: number;
  estimatedCostUsd: number;
  runs: number;
  estimatedRuns: number;
  unpricedRuns: number;
}

/** One project and kind within a day ("shop-api · notes"), all models merged. */
export interface UsageLine extends UsageTotals {
  root: string | null;
  kind: UsageKind;
}

export interface DayUsage extends UsageTotals {
  /** The local calendar date, e.g. "2026-10-14". */
  day: string;
  lines: UsageLine[];
}

const ZERO: UsageTotals = { tokens: 0, costUsd: 0, estimatedCostUsd: 0, runs: 0, estimatedRuns: 0, unpricedRuns: 0 };

function plus(a: UsageTotals, b: UsageTotals): UsageTotals {
  return {
    tokens: a.tokens + b.tokens,
    costUsd: a.costUsd + b.costUsd,
    estimatedCostUsd: a.estimatedCostUsd + b.estimatedCostUsd,
    runs: a.runs + b.runs,
    estimatedRuns: a.estimatedRuns + b.estimatedRuns,
    unpricedRuns: a.unpricedRuns + b.unpricedRuns,
  };
}

export function bucketTotals(b: UsageBucket): UsageTotals {
  return {
    tokens: b.input + b.cacheWrite + b.cacheRead + b.output,
    costUsd: b.costUsd,
    estimatedCostUsd: b.estimatedCostUsd,
    runs: b.runs,
    estimatedRuns: b.estimatedRuns,
    unpricedRuns: b.unpricedRuns,
  };
}

export function sumTotals(items: UsageTotals[]): UsageTotals {
  return items.reduce(plus, ZERO);
}

/** Days since the epoch on the user's calendar. A bucket belongs to the day its hour starts in. */
function localDay(ms: number, offset: Offset): number {
  return Math.floor((ms + offset(ms) * 60_000) / DAY);
}

const isoDay = (day: number) => new Date(day * DAY).toISOString().slice(0, 10);

/** The first hour to ask `usage_report` for, covering `days` local days up to today. One hour early, so a daylight-saving change never cuts off the first local hour; the helpers below filter exactly. */
export function sinceHourFor(now: number, days: number, offset: Offset = localOffset): number {
  const midnight = (localDay(now, offset) - (days - 1)) * DAY - offset(now) * 60_000;
  return Math.floor(midnight / HOUR) - 1;
}

/** The usage panel's rows: every one of the last `DAYS_SHOWN` local days, today first, including days with no runs
 *  (all zeros, no lines). Each day is split by project and kind, largest first. */
export function groupByLocalDay(buckets: UsageBucket[], now: number, offset: Offset = localOffset): DayUsage[] {
  const today = localDay(now, offset);
  const days = new Map<number, Map<string, UsageLine>>();
  for (const b of buckets) {
    const day = localDay(b.hour * HOUR, offset);
    if (day > today || day <= today - DAYS_SHOWN) continue;
    const lines = days.get(day) ?? new Map<string, UsageLine>();
    days.set(day, lines);
    const key = JSON.stringify([b.root, b.kind]);
    const line = lines.get(key) ?? { root: b.root, kind: b.kind, ...ZERO };
    lines.set(key, { root: line.root, kind: line.kind, ...plus(line, bucketTotals(b)) });
  }
  return Array.from({ length: DAYS_SHOWN }, (_, back) => {
    const day = today - back;
    const list = [...(days.get(day)?.values() ?? [])].sort((x, y) => y.tokens - x.tokens);
    return { day: isoDay(day), ...sumTotals(list), lines: list };
  });
}

/** Today's totals on the user's clock, for the sidebar footer. */
export function todayTotals(buckets: UsageBucket[], now: number, offset: Offset = localOffset): UsageTotals {
  const today = localDay(now, offset);
  return sumTotals(buckets.filter((b) => localDay(b.hour * HOUR, offset) === today).map(bucketTotals));
}

/** "950", "1.2k", "182k", "1.2M". */
export function formatTokens(n: number): string {
  if (n < 1000) return String(Math.round(n));
  const [value, unit]: [number, string] = n < 999_500 ? [n / 1000, "k"] : [n / 1_000_000, "M"];
  const text = value < 9.95 ? value.toFixed(1).replace(/\.0$/, "") : String(Math.round(value));
  return text + unit;
}

/** "$0.41", "~$0.41" for an estimate, and four places below a cent ("$0.0005"), where one run usually costs. */
export function formatCost(usd: number, estimated: boolean): string {
  const text = usd === 0 ? "$0.00" : usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
  return estimated ? `~${text}` : text;
}

/** The cost to show: "—" when no run had a known price, "~" when estimates are part of it. */
export function costText(t: UsageTotals): string {
  if (t.runs > 0 && t.unpricedRuns === t.runs) return "—";
  return formatCost(t.costUsd, t.estimatedCostUsd > 0);
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** What's behind a cost: "incl. ~$0.03 from 12 cancelled runs", "price unknown for 2 runs". */
export function usageNotes(t: UsageTotals): string[] {
  const notes: string[] = [];
  if (t.estimatedRuns > 0) notes.push(`incl. ${formatCost(t.estimatedCostUsd, true)} from ${plural(t.estimatedRuns, "cancelled run")}`);
  if (t.unpricedRuns > 0) notes.push(`price unknown for ${plural(t.unpricedRuns, "run")}`);
  return notes;
}

/** A usage panel row's cells. A row with no runs (a quiet day) shows "—" in every column. */
export function rowCells(t: UsageTotals): { tokens: string; cost: string; runs: string } {
  if (t.runs === 0) return { tokens: "—", cost: "—", runs: "—" };
  return { tokens: formatTokens(t.tokens), cost: costText(t), runs: String(t.runs) };
}

/** "Today · 182k tokens · ~$0.41". */
export function footerText(t: UsageTotals): string {
  return `Today · ${formatTokens(t.tokens)} tokens · ${costText(t)}`;
}

/** The project's folder name; "No project" for tabs without one. */
export function projectName(root: string | null): string {
  if (root === null) return "No project";
  return root.split(/[\\/]/).filter(Boolean).pop() ?? root;
}

/** "shop-api · notes". */
export function lineLabel(l: UsageLine): string {
  return `${projectName(l.root)} · ${l.kind}`;
}

/** "Wed 14 Oct" in the user's locale. `day` is already a local date, so it's formatted as UTC. */
export function formatDay(day: string): string {
  return new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", timeZone: "UTC" });
}
