// TabSession behaviour that only matters with several tabs: hidden tabs keep
// analysing, context changes wait for the tab to be shown, and the history
// can be browsed and stays current.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot, Tab, UiEvent } from "./ipc";
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
  nextSnapshot: 2,
  history: [snap()],
  ...over,
});

interface Call {
  draft: string;
  previousDraft: string | null;
  instructions: string;
  context: string;
  emit: (e: UiEvent) => void;
}

function harness(t: Tab, getTab?: TabSessionDeps["getTab"]) {
  const calls: Call[] = [];
  const deps: TabSessionDeps = {
    analyze: (_id, draft, previousDraft, instructions, context, onEvent) => {
      calls.push({ draft, previousDraft, instructions, context, emit: onEvent });
      return Promise.resolve();
    },
    saveDraft: vi.fn(),
    debounceMs: () => 1000,
    getTab,
  };
  return { s: new TabSession(t, deps), calls };
}

/** Lets promise callbacks run; fake timers don't touch microtasks. */
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("background tabs", () => {
  it("a background tab's pause fires after you switch away", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    s.deactivate();
    vi.advanceTimersByTime(1000);
    expect(calls.map((c) => c.draft)).toEqual([DRAFT]);
  });

  it("a context change on a background tab waits for activate()", () => {
    const { s, calls } = harness(tab());
    s.deactivate();
    s.setContext("/p|2|", "Focus on retries.");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().contextDirty).toBe(true);
    s.activate();
    expect(s.getState().contextDirty).toBe(false);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null, instructions: "Focus on retries.", context: "/p|2|" });
  });

  it("activate() skips the re-run when the draft is too short", () => {
    const { s, calls } = harness(tab({ draft: "too short", history: [snap({ draft: "too short" })] }));
    s.deactivate();
    s.setContext("/p|2|", "");
    s.activate();
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
  });

  it("on the shown tab, a new context re-reads the same draft with no gate", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|2|", "");
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].previousDraft).toBeNull();
  });

  it("a background run reads the new context, so showing the tab adds no second run", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setContext("/p|1|", "");
    s.setDraft(DRAFT);
    s.deactivate();
    s.setContext("/p|2|", "");
    expect(s.getState().contextDirty).toBe(true);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].context).toBe("/p|2|");
    expect(s.getState().contextDirty).toBe(false);
    s.activate();
    vi.advanceTimersByTime(5000);
    expect(calls).toHaveLength(1);
  });
});

describe("restore", () => {
  it("a tab with no saved analysis takes its first context without a run", () => {
    const { s, calls } = harness(tab({ history: [] })); // a long draft that was never analysed
    s.setContext("/p|1|", "Focus.");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ previousDraft: null, instructions: "Focus.", context: "/p|1|" });
  });

  it("a project that can't be read keeps the saved context and starts no run", () => {
    const { s, calls } = harness(tab()); // saved notes that ran with "/p|1|"
    s.setContext(null, "Focus.");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().analysis.status).toBe("done");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ instructions: "Focus.", context: "/p|1|" });
  });

  it("starts no run when the draft matches the latest analysis", () => {
    const { s, calls } = harness(tab({ draft: `  ${DRAFT}\n` }));
    s.setContext("/p|1|", "");
    vi.advanceTimersByTime(5000);
    expect(calls).toEqual([]);
    expect(s.getState().analysis.status).toBe("done");
    expect(s.getState().stale).toBe(false);
  });

  it("a partial latest analysis is never the gate's comparison point", () => {
    const { s, calls } = harness(tab({ history: [snap({ partial: true })] }));
    s.setContext("/p|1|", "");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBeNull();
  });

  it("a complete latest analysis is", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(calls[0].previousDraft).toBe(DRAFT);
  });
});

describe("history", () => {
  const three = [snap({ id: 3 }), snap({ id: 2, draft: "an earlier draft of the idea" }), snap({ id: 1, draft: "the first draft of the idea" })];

  it("view() clamps to the history, and stays at 0 with fewer than two", () => {
    const one = harness(tab()).s;
    one.view(1);
    expect(one.getState().viewing).toBe(0);
    const { s } = harness(tab({ history: three }));
    s.view(2);
    expect(s.getState().viewing).toBe(2);
    s.view(9);
    expect(s.getState().viewing).toBe(2);
    s.view(-1);
    expect(s.getState().viewing).toBe(0);
  });

  it("started goes back to the latest", () => {
    const { s, calls } = harness(tab({ history: three }));
    s.view(2);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    expect(s.getState().viewing).toBe(2); // the gate may still skip: keep browsing until a run starts
    calls[0].emit({ type: "started" });
    expect(s.getState().viewing).toBe(0);
  });

  it("done refetches the history, which now starts with this analysis", async () => {
    const fresh = [snap({ id: 4, draft: `${DRAFT}, and alert after three` }), ...three];
    const getTab = vi.fn(async () => tab({ history: fresh }));
    const { s, calls } = harness(tab({ history: three }), getTab);
    s.setDraft(`${DRAFT}, and alert after three`);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "started" });
    calls[0].emit({ type: "done", text: "## Intent\n- alert" });
    await settle();
    expect(getTab).toHaveBeenCalledWith("t-1");
    expect(s.getState().history.map((h) => h.id)).toEqual([4, 3, 2, 1]);
  });

  it("skips and errors never refetch", async () => {
    const getTab = vi.fn(async () => tab());
    const { s, calls } = harness(tab(), getTab);
    s.setDraft(`${DRAFT}, reworded`);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "skipped" });
    s.setDraft(`${DRAFT}, reworded again with more`);
    vi.advanceTimersByTime(1000);
    calls[1].emit({ type: "error", message: "Claude Code isn't installed." });
    await settle();
    expect(getTab).not.toHaveBeenCalled();
  });

  it("a slow history fetch never replaces a newer one", async () => {
    const resolvers: ((t: Tab) => void)[] = [];
    const getTab = vi.fn(() => new Promise<Tab>((resolve) => resolvers.push(resolve)));
    const { s, calls } = harness(tab({ history: three }), getTab);
    s.setDraft(`${DRAFT}, first change to it`);
    vi.advanceTimersByTime(1000);
    calls[0].emit({ type: "done", text: "## Intent\n- one" });
    s.setDraft(`${DRAFT}, second change to it`);
    vi.advanceTimersByTime(1000);
    calls[1].emit({ type: "done", text: "## Intent\n- two" });
    resolvers[1](tab({ history: [snap({ id: 5 }), snap({ id: 4 })] }));
    await settle();
    resolvers[0](tab({ history: [snap({ id: 4 })] }));
    await settle();
    expect(s.getState().history.map((h) => h.id)).toEqual([5, 4]);
  });
});

describe("requests", () => {
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

  it("dispose stops the pause and drops late events", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    s.setDraft(`${DRAFT}, more`);
    s.dispose();
    vi.advanceTimersByTime(5000);
    expect(calls).toHaveLength(1);
    calls[0].emit({ type: "done", text: "## Intent\n- late" });
    expect(s.getState().analysis.status).toBe("thinking");
  });
});
