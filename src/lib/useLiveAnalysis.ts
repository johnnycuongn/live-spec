import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./ipc";
import {
  applyEvent,
  initialAnalysis,
  lastAnalyzedAfter,
  previousDraftFor,
  startAnalysis,
  type AnalysisState,
  type Analyzed,
} from "./analysisState";

const MIN_CHARS = 24;
const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/**
 * Re-analyzes the draft after the user pauses typing.
 *
 * Like a colleague reading over your shoulder: they wait until you stop
 * typing, then comment; if you keep going, their last comment stays visible
 * but marked as out of date. If the edit didn't change the idea, the backend
 * skips (Jev gate) and the notes stay. Old responses are dropped by request
 * id; the backend also aborts the superseded run, checks included.
 */
export function useLiveAnalysis(draft: string, debounceMs: number, contextKey: string, instructions: string) {
  const [state, setState] = useState<AnalysisState>(initialAnalysis);
  const requestId = useRef(0);
  const lastSent = useRef("");
  const lastAnalyzed = useRef<Analyzed | null>(null);
  // Read at call time, so each request remembers the context it ran in.
  const context = useRef(contextKey);
  context.current = contextKey;
  // Also read at call time: each request sends the instructions current when it starts.
  const instructionsRef = useRef(instructions);
  instructionsRef.current = instructions;

  const run = useCallback((text: string, rerun = false) => {
    const id = ++requestId.current;
    lastSent.current = normalize(text);
    const ctx = context.current;
    setState((s) => startAnalysis(s, text));
    const mine = () => id === requestId.current;

    api
      .analyze(text, previousDraftFor(lastAnalyzed.current, rerun, ctx), instructionsRef.current, (ev) => {
        if (!mine()) return;
        lastAnalyzed.current = lastAnalyzedAfter(lastAnalyzed.current, ev, text, ctx);
        setState((s) => applyEvent(s, ev));
      })
      .catch((e) => mine() && setState((s) => ({ ...s, status: "error", error: errorMessage(e) })));
  }, []);

  // New project or brief: the same draft deserves a fresh read, with no gate.
  useEffect(() => {
    lastSent.current = "";
    lastAnalyzed.current = null;
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
    rerun: () => run(draft, true),
  };
}
