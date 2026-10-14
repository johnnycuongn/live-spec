// One tab's live analysis, kept outside React: a timer inside a hook would
// die when you switch tabs. Like a colleague reading over your shoulder, it
// waits until you pause, then comments; if you keep going, the last notes stay
// visible but marked out of date. The pause fires whether or not the tab is
// shown. Old responses are dropped by request id; the backend also aborts a
// tab's superseded run (its task slots are keyed by tab id).
import {
  applyEvent,
  initialAnalysis,
  lastAnalyzedAfter,
  previousDraftFor,
  startAnalysis,
  type AnalysisState,
  type Analyzed,
} from "./analysisState";
import { snapshotAnalysis } from "./history";
import { errorMessage, type Snapshot, type Tab, type UiEvent } from "./ipc";

export const MIN_CHARS = 24;
export const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

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
  /** Refetches the tab after `done`, for its history. Without it the history stays restore's copy. */
  getTab?: (tabId: string) => Promise<Tab>;
}

/** What React renders. A new object only when something changed (useSyncExternalStore). */
export interface TabView {
  draft: string;
  analysis: AnalysisState;
  tooShort: boolean;
  stale: boolean;
  contextDirty: boolean;
  /** 0 is the live state; k >= 1 is history[k]. */
  viewing: number;
  /** Newest first, at most 5 (sessions::HISTORY_LEN). */
  history: Snapshot[];
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
  private viewing = 0;
  private context = "";
  private instructions = "";
  private lastSent = "";
  private lastAnalyzed: Analyzed | null = null;
  private contextDirty = false;
  /** A session is created for the tab being shown; tabs.ts deactivates it when another tab is shown. */
  private active = true;
  private disposed = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private requestId = 0;
  private historyRequest = 0;

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
   * run; partial notes are never the gate's comparison point.
   */
  restore(tab: Tab): void {
    this.clearTimer();
    this.requestId++; // events from a run before the restore no longer apply
    const latest = tab.history[0];
    this.draft = tab.draft;
    this.history = tab.history;
    this.viewing = 0;
    this.analysis = latest ? snapshotAnalysis(latest) : initialAnalysis;
    this.context = latest?.context ?? "";
    this.lastSent = latest && normalize(latest.draft) === normalize(tab.draft) ? normalize(tab.draft) : "";
    this.lastAnalyzed = latest && !latest.partial ? { draft: latest.draft, context: latest.context } : null;
    this.contextDirty = false;
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
   * instructions. A new key on the shown tab re-reads the same draft with no
   * gate. On a hidden tab it waits for activate(), so a brief rebuilt under
   * ten open tabs re-runs only the one you're looking at. The first key a tab
   * with no saved analysis gets is not a change: it starts no run. A null key
   * means the project couldn't be read: the tab keeps the context its saved
   * notes ran with, so a missing folder never re-runs them, and the next pause
   * still reads the new instructions.
   */
  setContext(key: string | null, instructions: string): void {
    this.instructions = instructions;
    if (key === null || key === this.context || this.disposed) return;
    const first = this.context === "";
    this.context = key;
    // A tab with no saved analysis has no context until the window hands it one. That first
    // context isn't a change, so relaunching never starts a run by itself; a pending pause
    // reads it when it fires.
    if (first) return;
    if (this.active) this.resetAndSchedule();
    else this.contextDirty = true;
    this.emit();
  }

  activate(): void {
    if (this.active || this.disposed) return;
    this.active = true;
    if (!this.contextDirty) return;
    this.contextDirty = false;
    this.resetAndSchedule(); // schedule() skips drafts under MIN_CHARS
    this.emit();
  }

  /** Leaves any timer or run alone: a hidden tab keeps analysing. */
  deactivate(): void {
    this.active = false;
  }

  /** "Try again": always re-analyses, past the gate. */
  rerun(): void {
    if (!this.disposed) this.run(this.draft, true);
  }

  /** Clears the timer and drops late events. close_tab and move_tab cancel the backend run. */
  dispose(): void {
    this.disposed = true;
    this.clearTimer();
    this.requestId++;
    this.historyRequest++;
    this.listeners.clear();
  }

  /** Shows analysis `index` (0 = latest), clamped to the history; 0 with fewer than two. */
  view(index: number): void {
    const max = this.history.length < 2 ? 0 : this.history.length - 1;
    const next = Number.isFinite(index) ? Math.min(Math.max(Math.trunc(index), 0), max) : 0;
    if (next === this.viewing) return;
    this.viewing = next;
    this.emit();
  }

  private resetAndSchedule() {
    this.lastSent = "";
    this.lastAnalyzed = null;
    this.schedule();
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
    this.contextDirty = false; // this run reads the current context
    this.analysis = startAnalysis(this.analysis, text);
    this.emit();
    const mine = () => id === this.requestId;
    this.deps
      .analyze(this.id, text, previousDraftFor(this.lastAnalyzed, rerun, ctx), this.instructions, ctx, (ev) => {
        if (!mine()) return;
        this.lastAnalyzed = lastAnalyzedAfter(this.lastAnalyzed, ev, text, ctx);
        if (ev.type === "started") this.viewing = 0; // live streaming always shows the latest
        this.analysis = applyEvent(this.analysis, ev);
        this.emit();
        if (ev.type === "done") this.refreshHistory();
      })
      .catch((e) => {
        if (!mine()) return;
        this.analysis = { ...this.analysis, status: "error", error: errorMessage(e) };
        this.emit();
      });
  }

  /** `analyze` records the snapshot before it sends `done`, so this fetch includes it. */
  private refreshHistory() {
    const getTab = this.deps.getTab;
    if (!getTab) return;
    const n = ++this.historyRequest;
    getTab(this.id).then(
      (tab) => {
        if (n !== this.historyRequest) return; // a newer fetch, or dispose, won
        this.history = tab.history;
        this.viewing = Math.min(this.viewing, Math.max(tab.history.length - 1, 0));
        this.emit();
      },
      () => {}, // the notes on screen are right; the next `done` refetches
    );
  }

  private emit() {
    const norm = normalize(this.draft);
    this.state = {
      draft: this.draft,
      analysis: this.analysis,
      tooShort: norm.length < MIN_CHARS,
      stale: this.analysis.status !== "idle" && normalize(this.analysis.analyzedDraft) !== norm,
      contextDirty: this.contextDirty,
      viewing: this.viewing,
      history: this.history,
    };
    this.listeners.forEach((l) => l());
  }
}
