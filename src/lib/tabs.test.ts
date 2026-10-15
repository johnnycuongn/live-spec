import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Tab, TabSummary, WindowView } from "./ipc";

const api = vi.hoisted(() => ({
  listWorkspace: vi.fn(),
  getTab: vi.fn(),
  createTab: vi.fn(),
  saveDraft: vi.fn(),
  renameTab: vi.fn(),
  activateTab: vi.fn(),
  closeTab: vi.fn(),
  reopenTab: vi.fn(),
  deleteTab: vi.fn(),
  importLegacyDrafts: vi.fn(),
  moveTab: vi.fn(),
  analyze: vi.fn(),
}));
vi.mock("./ipc", () => ({
  api,
  errorMessage: (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e)),
}));
vi.mock("./windowId", () => ({ currentWindowId: () => "w-1" }));
vi.mock("./events", () => ({ onWorkspaceChanged: vi.fn() }));

const summary = (id: string, root: string | null): TabSummary => ({ id, root, title: null, firstLine: "", updatedAt: 0, closedAt: null });
const full = (id: string, root: string | null, draft = ""): Tab => ({
  id,
  root,
  title: null,
  createdAt: 0,
  updatedAt: 0,
  closedAt: null,
  draft,
  nextSnapshot: 1,
  history: [],
});
const view = (tabs: TabSummary[], active: Tab | null, over: Partial<WindowView> = {}): WindowView => ({
  window: "w-1",
  tabs,
  active,
  closed: [],
  fresh: false,
  unreadable: [],
  ...over,
});

// Stored order a1, b1, a2; the sidebar shows a1, a2 | b1.
const stored = [summary("a1", "/a"), summary("b1", "/b"), summary("a2", "/a")];

/** A fresh store per test: the module keeps one window's state. */
async function loadStore(first: WindowView) {
  vi.resetModules();
  api.listWorkspace.mockResolvedValue(first);
  const mod = await import("./tabs");
  await mod.tabs.load();
  return mod;
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.unstubAllGlobals();
  for (const f of [api.saveDraft, api.renameTab, api.activateTab, api.closeTab, api.reopenTab, api.deleteTab, api.importLegacyDrafts, api.analyze]) {
    f.mockResolvedValue(undefined);
  }
});

describe("tabs store", () => {
  it("closing the active tab shows the next one in the sidebar's order", async () => {
    const { tabs, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    expect(workspaceState().activeId).toBe("a1");
    api.getTab.mockResolvedValue(full("a2", "/a"));
    api.listWorkspace.mockResolvedValue(view(stored.slice(1), full("a2", "/a")));
    await tabs.close("a1");
    expect(api.closeTab).toHaveBeenCalledWith("a1");
    expect(api.getTab).toHaveBeenCalledWith("a2"); // the stored order would pick b1
    expect(api.activateTab).toHaveBeenCalledWith("w-1", "a2");
    // a2 is active before a1 closes, so the backend never moves `active` to b1.
    expect(api.activateTab.mock.invocationCallOrder[0]).toBeLessThan(api.closeTab.mock.invocationCallOrder[0]);
    expect(workspaceState().activeId).toBe("a2");
  });

  it("a draft that can't be saved keeps the tab open and says so", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    api.saveDraft.mockRejectedValue("disk full");
    await tabs.close("a1");
    expect(api.closeTab).not.toHaveBeenCalled();
    expect(sessionFor("a1")).toBeDefined();
    expect(workspaceState().activeId).toBe("a1");
    expect(workspaceState().error).toMatch(/draft/i);
  });

  it("closing the last tab leaves the window empty", async () => {
    const { tabs, workspaceState } = await loadStore(view([summary("a1", "/a")], full("a1", "/a")));
    api.listWorkspace.mockResolvedValue(view([], null));
    await tabs.close("a1");
    expect(workspaceState().activeId).toBeNull();
    expect(workspaceState().view?.tabs).toEqual([]);
  });

  it("switching flushes the pending save of the tab you left", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    expect(api.saveDraft).not.toHaveBeenCalled();
    api.getTab.mockResolvedValue(full("b1", "/b"));
    await tabs.switchTo("b1");
    expect(api.saveDraft).toHaveBeenCalledWith("a1", "typed in a1");
    expect(api.saveDraft.mock.invocationCallOrder[0]).toBeLessThan(api.getTab.mock.invocationCallOrder[0]);
  });

  it("flush saves the pending draft now", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    expect(api.saveDraft).not.toHaveBeenCalled();
    await tabs.flush();
    expect(api.saveDraft).toHaveBeenCalledWith("a1", "typed in a1");
  });

  it("next() follows the sidebar's order", async () => {
    const { tabs, workspaceState } = await loadStore(view(stored, full("a2", "/a")));
    api.getTab.mockResolvedValue(full("b1", "/b"));
    tabs.next(1); // the stored order would wrap to a1
    await vi.waitFor(() => expect(workspaceState().activeId).toBe("b1"));
  });

  it("a project's context applies to its tabs, including ones shown later", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    tabs.setDebounce(0);
    tabs.setContext("/a", "/a|1|Focus.", "Focus.");
    api.getTab.mockResolvedValue(full("a2", "/a"));
    await tabs.switchTo("a2");
    sessionFor("a2")!.setDraft("Retry failed checkout payments with backoff");
    await vi.waitFor(() => expect(api.analyze).toHaveBeenCalled());
    expect(api.analyze.mock.calls[0].slice(0, 5)).toEqual([
      "a2",
      "Retry failed checkout payments with backoff",
      null,
      "Focus.",
      "/a|1|Focus.",
    ]);
  });

  it("moving the shown tab saves its draft first, then shows the next tab in the sidebar's order", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    api.moveTab.mockResolvedValue("w-2");
    api.getTab.mockResolvedValue(full("a2", "/a"));
    api.listWorkspace.mockResolvedValue(view(stored.slice(1), full("b1", "/b")));
    await tabs.move("a1", null);
    expect(api.saveDraft).toHaveBeenCalledWith("a1", "typed in a1");
    expect(api.saveDraft.mock.invocationCallOrder[0]).toBeLessThan(api.moveTab.mock.invocationCallOrder[0]);
    expect(api.moveTab).toHaveBeenCalledWith("a1", null);
    expect(sessionFor("a1")).toBeUndefined();
    expect(api.getTab).toHaveBeenCalledWith("a2"); // the backend's pick, b1, follows the stored order
    expect(workspaceState().activeId).toBe("a2");
  });

  it("a failed move keeps the tab here and says why", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    api.getTab.mockResolvedValue(full("a2", "/a"));
    api.moveTab.mockRejectedValue("This window was closed.");
    await tabs.move("a1", "w-9");
    expect(sessionFor("a1")).toBeDefined();
    expect(workspaceState().activeId).toBe("a1");
    expect(workspaceState().error).toBe("This window was closed.");
  });

  it("a draft that can't be saved keeps the tab here and says so", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    api.saveDraft.mockRejectedValue("disk full");
    await tabs.move("a1", "w-2");
    expect(api.moveTab).not.toHaveBeenCalled();
    expect(sessionFor("a1")).toBeDefined();
    expect(workspaceState().activeId).toBe("a1");
    expect(workspaceState().error).toMatch(/draft/i);
  });

  it("another tab's owed save does not block closing or moving this one", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    api.saveDraft.mockImplementation((id: string) => (id === "a1" ? Promise.reject("disk full") : Promise.resolve()));
    await tabs.flush("a1"); // a1's save fails and stays owed
    expect(workspaceState().saveError).toMatch(/disk full/);
    await tabs.close("b1"); // b1 is healthy
    expect(api.closeTab).toHaveBeenCalledWith("b1");
    await tabs.move("a2", "w-2"); // so is a2
    expect(api.moveTab).toHaveBeenCalledWith("a2", "w-2");
    api.closeTab.mockClear();
    api.moveTab.mockClear();
    await tabs.close("a1"); // a1 itself is still refused
    expect(api.closeTab).not.toHaveBeenCalled();
    await tabs.move("a1", "w-2");
    expect(api.moveTab).not.toHaveBeenCalled();
    expect(workspaceState().error).toMatch(/draft/i);
  });

  it("a successful save keeps the save error while another tab's save is still owed", async () => {
    const { tabs, sessionFor, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    sessionFor("a1")!.setDraft("typed in a1");
    api.saveDraft.mockImplementation((id: string) => (id === "a1" ? Promise.reject("disk full") : Promise.resolve()));
    await tabs.flush("a1");
    api.getTab.mockResolvedValue(full("b1", "/b"));
    await tabs.switchTo("b1");
    sessionFor("b1")!.setDraft("typed in b1");
    await tabs.flush("b1"); // b1 saves fine
    expect(workspaceState().saveError).toMatch(/disk full/); // a1 is still owed
    api.saveDraft.mockResolvedValue(undefined);
    await tabs.flush(); // a1 retried and saved
    expect(workspaceState().saveError).toBeNull();
  });

  it("a tab moved here from another window is shown", async () => {
    const { tabs, workspaceState } = await loadStore(view(stored, full("a1", "/a")));
    api.listWorkspace.mockResolvedValue(view([...stored, summary("m1", "/m")], full("m1", "/m")));
    await tabs.refresh();
    expect(workspaceState().activeId).toBe("m1");
    // The backend's active tab alone is never followed: only a tab that just arrived.
    api.listWorkspace.mockResolvedValue(view([...stored, summary("m1", "/m")], full("a2", "/a")));
    await tabs.refresh();
    expect(workspaceState().activeId).toBe("m1");
  });
});

describe("first load and context", () => {
  it("a pause before the project's context arrives starts no run; the context then starts it", async () => {
    const { tabs, sessionFor } = await loadStore(view(stored, full("a1", "/a")));
    tabs.setDebounce(0);
    sessionFor("a1")!.setDraft("Retry failed checkout payments with backoff");
    await new Promise((r) => setTimeout(r, 10));
    expect(api.analyze).not.toHaveBeenCalled();
    tabs.setContext("/a", "/a|1|Focus.", "Focus.");
    expect(api.analyze).toHaveBeenCalledTimes(1);
    expect(api.analyze.mock.calls[0].slice(3, 5)).toEqual(["Focus.", "/a|1|Focus."]);
  });

  it("a failed first load is retried by the next load()", async () => {
    vi.resetModules();
    api.listWorkspace.mockRejectedValueOnce(new Error("backend not ready")).mockResolvedValue(view(stored, full("a1", "/a")));
    const { tabs, workspaceState } = await import("./tabs");
    await tabs.load();
    expect(workspaceState().error).toBe("backend not ready");
    expect(workspaceState().activeId).toBeNull();
    await tabs.load();
    expect(workspaceState().activeId).toBe("a1");
  });

  it("shows the first tab when the view lists tabs but none is active", async () => {
    api.getTab.mockResolvedValue(full("a1", "/a"));
    const { workspaceState } = await loadStore(view(stored, null));
    expect(api.activateTab).toHaveBeenCalledWith("w-1", "a1");
    expect(workspaceState().activeId).toBe("a1");
  });
});

describe("legacy drafts", () => {
  const storage = (entries: Record<string, string>) => {
    const m = new Map(Object.entries(entries));
    return {
      m,
      get length() {
        return m.size;
      },
      key: (i: number) => [...m.keys()][i] ?? null,
      getItem: (k: string) => m.get(k) ?? null,
      removeItem: (k: string) => void m.delete(k),
    };
  };

  // Collecting and removing the keys is Phase 1's legacyDrafts.test.ts; these pin the store's use of it.
  it("imports them once on a fresh workspace, then removes the keys", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea", "livebrainstorm.lastProject": "/a" });
    vi.stubGlobal("localStorage", s);
    api.listWorkspace
      .mockResolvedValueOnce(view([], null, { fresh: true }))
      .mockResolvedValue(view([summary("t-a", "/a")], full("t-a", "/a", "a idea")));
    vi.resetModules();
    const { tabs, workspaceState } = await import("./tabs");
    await tabs.load();
    expect(api.importLegacyDrafts).toHaveBeenCalledWith("w-1", [{ root: "/a", draft: "a idea" }], "/a");
    expect(s.m.size).toBe(0);
    expect(workspaceState().activeId).toBe("t-a");
  });

  it("keeps the keys when the import fails", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea" });
    vi.stubGlobal("localStorage", s);
    api.importLegacyDrafts.mockRejectedValue(new Error("disk full"));
    api.listWorkspace.mockResolvedValue(view([], null, { fresh: true }));
    vi.resetModules();
    const { tabs, workspaceState } = await import("./tabs");
    await tabs.load();
    expect(s.m.has("livebrainstorm.draft./a")).toBe(true);
    expect(workspaceState().error).toBe("Couldn't bring over your earlier drafts: disk full");
  });
});
