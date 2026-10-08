import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./ipc";
import { applyEvent, initialAnalysis, startAnalysis, type AnalysisState } from "./analysisState";

const MIN_CHARS = 24;
const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/**
 * Re-analyzes the draft after the user pauses typing.
 *
 * Like a colleague reading over your shoulder: they wait until you stop
 * typing, then comment; if you keep going, their last comment stays visible
 * but marked as out of date. Old responses are dropped by request id; the
 * backend also aborts the superseded model call (and its fact-checks).
 */
export function useLiveAnalysis(draft: string, debounceMs: number, contextKey: string) {
  const [state, setState] = useState<AnalysisState>(initialAnalysis);
  const requestId = useRef(0);
  const lastSent = useRef("");

  const run = useCallback((text: string) => {
    const id = ++requestId.current;
    lastSent.current = normalize(text);
    setState(startAnalysis(text));
    const mine = () => id === requestId.current;

    api
      .analyze(text, (ev) => {
        if (mine()) setState((s) => applyEvent(s, ev));
      })
      .catch((e) => mine() && setState((s) => ({ ...s, status: "error", error: errorMessage(e) })));
  }, []);

  // New project or brief: the same draft deserves a fresh read.
  useEffect(() => {
    lastSent.current = "";
  }, [contextKey]);

  useEffect(() => {
    const norm = normalize(draft);
    if (norm.length < MIN_CHARS || norm === lastSent.current) return;
    const timer = setTimeout(() => run(draft), debounceMs);
    return () => clearTimeout(timer);
  }, [draft, debounceMs, contextKey, run]);

  return {
    ...state,
    tooShort: normalize(draft).length < MIN_CHARS,
    stale: state.status !== "idle" && normalize(state.analyzedDraft) !== normalize(draft),
    rerun: () => run(draft),
  };
}
