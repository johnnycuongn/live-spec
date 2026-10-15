// TabSession on the one tab the window shows: the pause, the gate's
// comparison point, restoring saved notes, and dropping superseded events.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CheckResult, Snapshot, Tab, UiEvent } from "./ipc";
import { TabSession, type TabSessionDeps } from "./tabSession";

const DRAFT = "Retry failed checkout payments with backoff in the worker";

const snap = (over: Partial<Snapshot> = {}): Snapshot => ({
  id: 1,
  at: 1760403600,
  draft: DRAFT,
  context: "/p|1|",
  markdown: "## Intent\n- retry",
  partial: false,
  notices: [],
  checks: [],
  checksTotal: null,
  checksFaults: null,
  checksStopped: false,
  ...over,
});

const tab = (over: Partial<Tab> = {}): Tab => ({
  id: "t-1",
  root: "/p",
  title: null,
  createdAt: 1,
  updatedAt: 1,
  closedAt: null,
  draft: DRAFT,
  sections: [],
  nextSnapshot: 2,
  history: [snap()],
  ...over,
});

interface Call {
  tabId: string;
  draft: string;
  previousDraft: string | null;
  instructions: string;
  sections: string[];
  context: string;
  emit: (e: UiEvent) => void;
}

function harness(t: Tab, analyze?: TabSessionDeps["analyze"]) {
  const calls: Call[] = [];
  const saveDraft = vi.fn();
  const deps: TabSessionDeps = {
    analyze:
      analyze ??
      ((tabId, draft, previousDraft, instructions, sections, context, onEvent) => {
        calls.push({ tabId, draft, previousDraft, instructions, sections, context, emit: onEvent });
        return Promise.resolve();
      }),
    saveDraft,
    debounceMs: () => 1000,
  };
  return { s: new TabSession(t, deps), calls, saveDraft };
}

/** Lets promise callbacks run; fake timers don't touch microtasks. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("the pause", () => {
  it("analyses a moment after typing stops, with the tab, its instructions and its context", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setContext("/p|1|", "Focus on retries.");
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(999);
    expect(calls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ tabId: "t-1", draft: DRAFT, previousDraft: null, instructions: "Focus on retries.", sections: [], context: "/p|1|" });
    expect(s.getState().analysis.status).toBe("thinking");
  });

  it("saves every edit, and nothing when the text is unchanged", () => {
    const { s, saveDraft } = harness(tab({ draft: "", history: [] }));
    s.setDraft("a");
    s.setDraft("ab");
    s.setDraft("ab");
    expect(saveDraft.mock.calls).toEqual([
      ["t-1", "a"],
      ["t-1", "ab"],
    ]);
  });

  it("a draft under 24 characters never runs", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft("a short idea");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().tooShort).toBe(true);
  });

  it("whitespace-only edits don't re-send the same draft", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    s.setDraft(`${DRAFT}  \n`);
    vi.advanceTimersByTime(5000);
    expect(calls).toHaveLength(1);
  });

  it("dispose stops a pending pause", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    s.dispose();
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
  });
});

describe("restore", () => {
  it("shows the latest analysis and starts no run when the draft matches it", () => {
    const { s, calls } = harness(tab({ draft: `  ${DRAFT}\n` }));
    s.setContext("/p|1|", "");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    const v = s.getState();
    expect(v.analysis.status).toBe("done");
    expect(v.analysis.markdown).toBe("## Intent\n- retry");
    expect(v.stale).toBe(false);
    expect(v.viewing).toBe(0);
    expect(v.history.map((h) => h.id)).toEqual([1]);
  });

  it("an edit since the latest analysis shows as stale, and the next pause compares against it", () => {
    const { s, calls } = harness(tab({ draft: `${DRAFT}, and alert` }));
    s.setContext("/p|1|", "");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]); // relaunching never starts a run by itself
    expect(s.getState().stale).toBe(true);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBe(DRAFT);
  });

  it("a partial latest analysis is never the gate's comparison point", () => {
    const { s, calls } = harness(tab({ history: [snap({ partial: true })] }));
    s.setContext("/p|1|", "");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBeNull();
  });

  it("a new context re-reads the same draft, past the gate", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|2|", "Focus on retries.");
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null, context: "/p|2|", instructions: "Focus on retries." });
  });

  it("checks an edit cut off read as stopped, as they did live", () => {
    const check: CheckResult = { claim: "`retry` is missing", verdict: "confirmed", confidence: 0.9, path: "src/pay.ts", line: 3 };
    const { s } = harness(tab({ history: [snap({ checks: [check], checksTotal: 3, checksFaults: 5 })] }));
    const a = s.getState().analysis;
    expect(a.checking).toEqual({ count: 3, total: 5 }); // "1 of 5 faults checked", as it read live
    expect(a.checks).toEqual([check]);
    expect(a.checksStopped).toBe(true);
    expect(a.notices).toEqual(["Checks stopped when you edited."]);
  });

  it("starts idle with no saved analysis", () => {
    const { s } = harness(tab({ draft: "", history: [] }));
    expect(s.getState().analysis.status).toBe("idle");
    expect(s.getState().history).toEqual([]);
  });
});

describe("runs", () => {
  it("started clears the old notes, text streams in, and done is current", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "started" });
    calls[0].emit({ type: "text", text: "## Intent\n- a", lane: 0 });
    expect(s.getState().analysis.markdown).toBe("## Intent\n- a");
    calls[0].emit({ type: "done", text: "## Intent\n- a" });
    expect(s.getState().analysis.status).toBe("done");
    expect(s.getState().stale).toBe(false);
  });

  it("drops events from a superseded request", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(`${DRAFT}, version one`);
    vi.advanceTimersByTime(1000);
    s.setDraft(`${DRAFT}, version two`);
    vi.advanceTimersByTime(1000);
    calls[1].emit({ type: "started" });
    calls[0].emit({ type: "text", text: "## Intent\n- stale", lane: 0 });
    expect(s.getState().analysis.markdown).toBe("");
  });

  it("try again re-analyses the same draft, past the gate", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    s.rerun();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null });
  });

  it("a failed call shows its error", async () => {
    const { s } = harness(tab({ draft: "", history: [] }), () => Promise.reject(new Error("Claude Code isn't installed.")));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    await settle();
    expect(s.getState().analysis).toMatchObject({ status: "error", error: "Claude Code isn't installed." });
  });

  it("subscribers hear each change, with a new state object", () => {
    const { s } = harness(tab({ draft: "", history: [] }));
    const before = s.getState();
    const heard = vi.fn();
    const stop = s.subscribe(heard);
    s.setDraft("a");
    expect(heard).toHaveBeenCalledTimes(1);
    expect(s.getState()).not.toBe(before);
    expect(s.getState()).toBe(s.getState());
    stop();
    s.setDraft("ab");
    expect(heard).toHaveBeenCalledTimes(1);
  });
});

describe("sections", () => {
  it("a run carries the tab's stored sections", () => {
    const { s, calls } = harness(tab({ draft: "", history: [], sections: ["conflicts", "questions"] }));
    s.setContext("/p|1|", "");
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    expect(calls[0]).toMatchObject({ sections: ["conflicts", "questions"], context: "/p|1||conflicts,questions" });
    expect(s.getState().sections).toEqual(["conflicts", "questions"]);
  });

  it("changing the sections re-reads the same draft past the gate, with the new key", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    expect(calls).toEqual([]); // the saved notes match the draft
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null, sections: ["gaps"], context: "/p|1||gaps" });
    expect(s.getState().sections).toEqual(["gaps"]);
  });

  it("back to the default set re-runs with the bare project key", () => {
    const { s, calls } = harness(tab({ sections: ["gaps"], history: [snap({ context: "/p|1||gaps" })] }));
    s.setContext("/p|1|", "");
    expect(calls).toEqual([]); // the saved notes ran with these sections
    s.setSections([]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].context).toBe("/p|1|");
  });

  it("the same sections again change nothing", () => {
    const { s, calls } = harness(tab({ sections: ["gaps"], history: [snap({ context: "/p|1||gaps" })] }));
    s.setContext("/p|1|", "");
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toEqual([]);
  });

  it("on a hidden tab the change waits until the tab is shown", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    s.deactivate();
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toEqual([]);
    expect(s.getState().contextDirty).toBe(true);
    s.activate();
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].context).toBe("/p|1||gaps");
  });

  it("setSections during a run supersedes it", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setContext("/p|1|", "");
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    expect(s.getState().analysis.status).toBe("thinking");
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(2);
    calls[0].emit({ type: "text", text: "## Intent\n- stale", lane: 0 });
    expect(s.getState().analysis.markdown).toBe(""); // the first run's events are dropped
    calls[1].emit({ type: "started" });
    calls[1].emit({ type: "text", text: "## Gaps\n- g", lane: 0 });
    expect(s.getState().analysis.markdown).toBe("## Gaps\n- g");
  });

  it("without a project key (the folder couldn't be read) a change still re-runs with the new sections", () => {
    const { s, calls } = harness(tab());
    s.setContext(null, "");
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ sections: ["gaps"], context: "/p|1|" });
  });
});
