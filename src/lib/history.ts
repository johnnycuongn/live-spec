// A tab's history of analyses (newest first, at most 5): an older snapshot
// shown as notes, and the `‹ 2 of 5 ›` header. Snapshot mirrors
// sessions::Snapshot in crates/livespec-core.
import type { AnalysisState } from "./analysisState";
import type { Snapshot } from "./ipc";

export const CHECKS_CUT_OFF = "Checks stopped when you edited.";

/**
 * A saved analysis as the notes pane shows it. Fewer checks than
 * `checksTotal`, without `checksStopped`, means an edit cut the run off: it
 * reads as stopped, as it did live. `checksFaults` keeps the summary's "of 5
 * faults"; a file saved before it existed counts against the claims instead.
 */
export function snapshotAnalysis(s: Snapshot): AnalysisState {
  const cutOff = !s.checksStopped && s.checksTotal !== null && s.checks.length < s.checksTotal;
  return {
    status: "done",
    lanes: [],
    markdown: s.markdown,
    error: null,
    analyzedDraft: s.draft,
    checking: s.checksTotal === null ? null : { count: s.checksTotal, total: s.checksFaults ?? s.checksTotal },
    checks: s.checks,
    notices: cutOff && !s.notices.includes(CHECKS_CUT_OFF) ? [...s.notices, CHECKS_CUT_OFF] : s.notices,
    checksStopped: s.checksStopped || cutOff,
    partial: s.partial,
  };
}

export interface HistoryPosition {
  show: boolean;
  label: string;
  /** The `viewing` index of the next older analysis, or null at the oldest. */
  earlier: number | null;
  /** The `viewing` index of the next newer analysis, or null at the latest. */
  later: number | null;
}

/**
 * `viewing` 0 is the latest. The label counts in time order, so the oldest is
 * "1 of n" and the latest "n of n", and ‹ goes back in time. Hidden with
 * fewer than two analyses.
 */
export function historyPosition(viewing: number, length: number): HistoryPosition {
  if (length < 2) return { show: false, label: "", earlier: null, later: null };
  const k = Math.min(Math.max(Math.trunc(viewing) || 0, 0), length - 1);
  return { show: true, label: `${length - k} of ${length}`, earlier: k < length - 1 ? k + 1 : null, later: k > 0 ? k - 1 : null };
}

/** "14:02" today; "Oct 12, 14:02" on another day. In the user's locale and time zone. */
export function snapshotTime(at: number, now: Date = new Date()): string {
  const d = new Date(at * 1000);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return time;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}
