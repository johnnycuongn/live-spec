// Pure state transitions for the live analysis, testable without React.
// Check events may arrive after `done`: the notes are complete before
// "Checked in code" fills in.
import type { CheckResult, UiEvent } from "./ipc";

export type AnalysisStatus = "idle" | "thinking" | "done" | "error";

export interface AnalysisState {
  status: AnalysisStatus;
  markdown: string;
  error: string | null;
  analyzedDraft: string;
  checking: { count: number; total: number } | null;
  checks: CheckResult[];
  notice: string | null;
}

export const initialAnalysis: AnalysisState = {
  status: "idle",
  markdown: "",
  error: null,
  analyzedDraft: "",
  checking: null,
  checks: [],
  notice: null,
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
    case "text":
      return { ...s, markdown: s.markdown + ev.text };
    case "done":
      return { ...s, status: "done", markdown: ev.text || s.markdown };
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
      return { ...s, notice: ev.message };
    case "started":
      return { ...s, markdown: "", checks: [], checking: null, notice: null, error: null };
    case "skipped": {
      // The skip aborted any fact-checks still running for these notes.
      const unfinished = s.checking !== null && s.checks.length < s.checking.count;
      return { ...s, status: "done", notice: unfinished ? "Checks stopped when you edited." : s.notice };
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
 * has started, the notes it replaced are gone, so nothing is left to keep.
 */
export const lastAnalyzedAfter = (last: Analyzed | null, ev: UiEvent, sent: string, context: string): Analyzed | null =>
  ev.type === "done" ? { draft: sent, context } : ev.type === "started" ? null : last;

/**
 * What to send as `previousDraft`. "Try again" always re-analyses, and notes
 * from another project or brief never count as current.
 */
export const previousDraftFor = (last: Analyzed | null, rerun: boolean, context: string): string | null =>
  rerun || !last || last.context !== context ? null : last.draft;
