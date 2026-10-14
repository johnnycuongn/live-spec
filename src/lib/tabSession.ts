// One tab's live analysis, kept outside React so its pause timer and its run
// don't depend on a component staying mounted. Like a colleague reading over
// your shoulder, it waits until you pause, then comments; if you keep going,
// the last notes stay visible but marked out of date. If the edit didn't
// change the idea, the backend skips (Jev gate) and the notes stay. Old
// responses are dropped by request id; the backend also aborts the tab's
// superseded run (its task slots are keyed by tab id).
import {
  applyEvent,
  initialAnalysis,
  lastAnalyzedAfter,
  previousDraftFor,
  startAnalysis,
  type AnalysisState,
  type Analyzed,
} from "./analysisState";
import { errorMessage, type Snapshot, type Tab, type UiEvent } from "./ipc";

export const MIN_CHARS = 24;
export const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/** What a run's fact-check shows when an edit cut it off, live or restored. */
const CHECKS_CUT_OFF = "Checks stopped when you edited.";

export interface TabSessionDeps {
  analyze: (
    tabId: string,
    draft: string,
    previousDraft: string | null,
    instructions: string,
    context: string,
    onEvent: (e: UiEvent) => void,
  ) => Promise<void>;
  /** The caller wires createSaver (500 ms) keyed by tab id. */
  saveDraft: (tabId: string, draft: string) => void;
  debounceMs: () => number;
}

/** What React renders. A new object only when something changed (useSyncExternalStore). */
export interface TabView {
  draft: string;
  analysis: AnalysisState;
  tooShort: boolean;
  stale: boolean;
  /** A context change waiting for the tab to be shown. Always false while one tab is shown. */
  contextDirty: boolean;
  /** Which saved analysis is shown; 0 is the latest. */
  viewing: number;
  /** Newest first, at most 5 (sessions::HISTORY_LEN). */
  history: Snapshot[];
}

/**
 * A saved analysis as the notes pane shows it. Fewer checks than
 * `checksTotal`, without `checksStopped`, means an edit cut the run off: it
 * reads as stopped, as it did live.
 */
function fromSnapshot(s: Snapshot): AnalysisState {
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

export class TabSession {
  readonly id: string;
  readonly root: string | null;
  private readonly deps: TabSessionDeps;
  private listeners = new Set<() => void>();
  private state!: TabView;
  private draft = "";
  private analysis: AnalysisState = initialAnalysis;
  private history: Snapshot[] = [];
  private context = "";
  private instructions = "";
  private lastSent = "";
  private lastAnalyzed: Analyzed | null = null;
  private disposed = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private requestId = 0;

  constructor(tab: Tab, deps: TabSessionDeps) {
    this.id = tab.id;
    this.root = tab.root;
    this.deps = deps;
    this.restore(tab);
  }

  getState(): TabView {
    return this.state;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  /**
   * Starts from the tab's latest analysis. A draft that matches it starts no
   * run, and restoring never starts one by itself; partial notes are never
   * the gate's comparison point.
   */
  restore(tab: Tab): void {
    this.clearTimer();
    this.requestId++; // events from a run before the restore no longer apply
    const latest = tab.history[0];
    this.draft = tab.draft;
    this.history = tab.history;
    this.analysis = latest ? fromSnapshot(latest) : initialAnalysis;
    this.context = latest?.context ?? "";
    this.lastSent = latest && normalize(latest.draft) === normalize(tab.draft) ? normalize(tab.draft) : "";
    this.lastAnalyzed = latest && !latest.partial ? { draft: latest.draft, context: latest.context } : null;
    this.emit();
  }

  setDraft(text: string): void {
    if (text === this.draft || this.disposed) return;
    this.draft = text;
    this.deps.saveDraft(this.id, text);
    this.schedule();
    this.emit();
  }

  /**
   * The window calls this once it has loaded the tab's project summary and
   * instructions. A new key (another brief, other instructions) re-reads the
   * same draft, past the gate.
   */
  setContext(key: string, instructions: string): void {
    this.instructions = instructions;
    if (key === this.context || this.disposed) return;
    this.context = key;
    this.lastSent = "";
    this.lastAnalyzed = null;
    this.schedule();
  }

  /** The window shows one tab at a time, so nothing waits for it: setContext always re-reads. */
  activate(): void {}

  /** Leaves any timer or run alone: a hidden tab keeps analysing. */
  deactivate(): void {}

  /** "Try again": always re-analyses, past the gate. */
  rerun(): void {
    if (!this.disposed) this.run(this.draft, true);
  }

  /** Clears the timer and drops late events. close_tab cancels the backend run. */
  dispose(): void {
    this.disposed = true;
    this.clearTimer();
    this.requestId++;
    this.listeners.clear();
  }

  private schedule() {
    this.clearTimer();
    const norm = normalize(this.draft);
    if (norm.length < MIN_CHARS || norm === this.lastSent) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.run(this.draft, false);
    }, this.deps.debounceMs());
  }

  private clearTimer() {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private run(text: string, rerun: boolean) {
    this.clearTimer();
    const id = ++this.requestId;
    const ctx = this.context;
    this.lastSent = normalize(text);
    this.analysis = startAnalysis(this.analysis, text);
    this.emit();
    const mine = () => id === this.requestId;
    this.deps
      .analyze(this.id, text, previousDraftFor(this.lastAnalyzed, rerun, ctx), this.instructions, ctx, (ev) => {
        if (!mine()) return;
        this.lastAnalyzed = lastAnalyzedAfter(this.lastAnalyzed, ev, text, ctx);
        this.analysis = applyEvent(this.analysis, ev);
        this.emit();
      })
      .catch((e) => {
        if (!mine()) return;
        this.analysis = { ...this.analysis, status: "error", error: errorMessage(e) };
        this.emit();
      });
  }

  private emit() {
    const norm = normalize(this.draft);
    this.state = {
      draft: this.draft,
      analysis: this.analysis,
      tooShort: norm.length < MIN_CHARS,
      stale: this.analysis.status !== "idle" && normalize(this.analysis.analyzedDraft) !== norm,
      contextDirty: false,
      viewing: 0,
      history: this.history,
    };
    this.listeners.forEach((l) => l());
  }
}
