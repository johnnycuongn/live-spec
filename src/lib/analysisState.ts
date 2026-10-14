// Pure state transitions for the live analysis, testable without React.
// Check events may arrive before or after `done`: the fact-check starts when
// the Faults lane finishes, independently of the other lanes.
import type { CheckResult, UiEvent } from "./ipc";
import { appendLane, joinLanes } from "./lanes";

export type AnalysisStatus = "idle" | "thinking" | "done" | "error";

export interface AnalysisState {
  status: AnalysisStatus;
  lanes: string[];
  markdown: string;
  error: string | null;
  analyzedDraft: string;
  checking: { count: number; total: number } | null;
  checks: CheckResult[];
  /** Every non-fatal message for the notes on screen, oldest first. */
  notices: string[];
  /** The fact-check ended early (it failed, or an edit skipped past it). */
  checksStopped: boolean;
  /** Some sections couldn't be written; offer a retry. */
  partial: boolean;
}

export const initialAnalysis: AnalysisState = {
  status: "idle",
  lanes: [],
  markdown: "",
  error: null,
  analyzedDraft: "",
  checking: null,
  checks: [],
  notices: [],
  checksStopped: false,
  partial: false,
};

/** A new pause: the old notes stay visible until the backend says `started`. */
export const startAnalysis = (prev: AnalysisState, draft: string): AnalysisState => ({
  ...prev,
  status: "thinking",
  error: null,
  analyzedDraft: draft,
});

export function applyEvent(s: AnalysisState, ev: UiEvent): AnalysisState {
  switch (ev.type) {
    case "text": {
      const lanes = appendLane(s.lanes, ev.lane, ev.text);
      return { ...s, lanes, markdown: joinLanes(lanes) };
    }
    case "done":
      return { ...s, status: "done", markdown: ev.text || s.markdown, partial: ev.partial ?? false };
    case "error":
      return { ...s, status: "error", error: ev.message };
    case "checking":
      return { ...s, checking: { count: ev.count, total: ev.total } };
    case "check":
      return {
        ...s,
        checks: [...s.checks, { claim: ev.claim, verdict: ev.verdict, confidence: ev.confidence, path: ev.path, line: ev.line }],
      };
    case "notice":
      return { ...s, notices: [...s.notices, ev.message] };
    case "checksStopped":
      return { ...s, notices: [...s.notices, ev.message], checksStopped: true };
    case "started":
      return { ...s, lanes: [], markdown: "", checks: [], checking: null, notices: [], checksStopped: false, partial: false, error: null };
    case "skipped": {
      // The skip aborted any fact-checks still running for these notes.
      const unfinished = s.checking !== null && s.checks.length < s.checking.count;
      return unfinished
        ? { ...s, status: "done", notices: [...s.notices, "Checks stopped when you edited."], checksStopped: true }
        : { ...s, status: "done" };
    }
    default:
      return s;
  }
}

/** The last analysis that actually completed, and the context (project and brief) it ran in. */
export interface Analyzed {
  draft: string;
  context: string;
}

/**
 * The draft the gate compares against: the last one actually analysed.
 * Skips never move it, so small edits add up to a real change. Once a run
 * has started, the notes it replaced are gone, so nothing is left to keep;
 * and partly written notes never count, so the next pause fills the gaps.
 */
export const lastAnalyzedAfter = (last: Analyzed | null, ev: UiEvent, sent: string, context: string): Analyzed | null => {
  if (ev.type === "done") return ev.partial ? null : { draft: sent, context };
  return ev.type === "started" ? null : last;
};

/**
 * What to send as `previousDraft`. "Try again" always re-analyses, and notes
 * from another project or brief never count as current.
 */
export const previousDraftFor = (last: Analyzed | null, rerun: boolean, context: string): string | null =>
  rerun || !last || last.context !== context ? null : last.draft;
