import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./ipc";

const MIN_CHARS = 24;
const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

export type AnalysisStatus = "idle" | "thinking" | "done" | "error";

interface State {
  status: AnalysisStatus;
  markdown: string;
  error: string | null;
  analyzedDraft: string;
}

/**
 * Re-analyzes the draft after the user pauses typing.
 *
 * Like a colleague reading over your shoulder: they wait until you stop
 * typing, then comment; if you keep going, their last comment stays visible
 * but marked as out of date. Old responses are dropped by request id; the
 * backend also aborts the superseded model call.
 */
export function useLiveAnalysis(draft: string, debounceMs: number, contextKey: string) {
  const [state, setState] = useState<State>({ status: "idle", markdown: "", error: null, analyzedDraft: "" });
  const requestId = useRef(0);
  const lastSent = useRef("");

  const run = useCallback((text: string) => {
    const id = ++requestId.current;
    lastSent.current = normalize(text);
    setState({ status: "thinking", markdown: "", error: null, analyzedDraft: text });
    const mine = () => id === requestId.current;

    api
      .analyze(text, (ev) => {
        if (!mine()) return;
        if (ev.type === "text") setState((s) => ({ ...s, markdown: s.markdown + ev.text }));
        else if (ev.type === "done") setState((s) => ({ ...s, status: "done", markdown: ev.text || s.markdown }));
        else if (ev.type === "error") setState((s) => ({ ...s, status: "error", error: ev.message }));
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
