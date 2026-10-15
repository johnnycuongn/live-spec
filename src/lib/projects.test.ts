import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstructionsView, ProjectSummary } from "./ipc";

const api = vi.hoisted(() => ({
  projectSummary: vi.fn(),
  getInstructions: vi.fn(),
  saveProjectInstructions: vi.fn(),
  saveTemplates: vi.fn(),
  restoreStarterTemplates: vi.fn(),
  openProject: vi.fn(),
  buildBrief: vi.fn(),
}));
const setContext = vi.hoisted(() => vi.fn());
vi.mock("./ipc", () => ({
  api,
  errorMessage: (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e)),
}));
vi.mock("./tabs", () => ({ tabs: { setContext } }));
vi.mock("./events", () => ({ onBriefStatus: vi.fn() }));

const summary = (root: string, createdAt: number | null): ProjectSummary => ({
  name: "p",
  root,
  indexedFiles: 1,
  totalFiles: 1,
  briefStatus: createdAt ? "fresh" : "missing",
  brief: createdAt ? { root, fingerprint: "f", text: "brief", createdAt, source: "test" } : null,
});
const instructionsView = (current: string): InstructionsView => ({ templates: [], project: { default: "", current, templateId: null } });

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

async function fresh() {
  vi.resetModules();
  return import("./projects");
}

beforeEach(() => {
  vi.resetAllMocks();
  api.saveProjectInstructions.mockResolvedValue(undefined);
});

describe("project contexts", () => {
  it("a project's context reaches its tabs only after both its summary and its instructions load", async () => {
    const { projects } = await fresh();
    const s = deferred<ProjectSummary>();
    const i = deferred<InstructionsView>();
    api.projectSummary.mockReturnValue(s.promise);
    api.getInstructions.mockReturnValue(i.promise);
    projects.ensure("/p");
    i.resolve(instructionsView("Focus."));
    await settle();
    expect(setContext).not.toHaveBeenCalled(); // "/p|0|Focus." would not match the saved notes' context
    s.resolve(summary("/p", 7));
    await settle();
    expect(setContext).toHaveBeenCalledTimes(1);
    expect(setContext).toHaveBeenCalledWith("/p", "/p|7|Focus.", "Focus.");
  });

  it("tabs with no project need only the instructions", async () => {
    const { projects } = await fresh();
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure(null);
    await settle();
    expect(api.projectSummary).not.toHaveBeenCalled();
    expect(setContext).toHaveBeenCalledWith(null, "|0|", "");
  });

  it("a project that can't be read hands its tabs only the instructions, and is retried when shown again", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockRejectedValueOnce("That folder no longer exists. Open it again from its new place.");
    api.projectSummary.mockResolvedValue(summary("/gone", 3));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/gone");
    await settle();
    expect(projectsState().summaryErrors["/gone"]).toBe("That folder no longer exists. Open it again from its new place.");
    expect(setContext).toHaveBeenLastCalledWith("/gone", null, ""); // keep the saved notes' context: no re-run
    projects.ensure("/gone");
    await settle();
    expect(projectsState().summaryErrors["/gone"]).toBeUndefined();
    expect(setContext).toHaveBeenLastCalledWith("/gone", "/gone|3|", "");
  });

  it("a finished brief build refreshes the summary, so the project's tabs get the new context", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValueOnce(summary("/p", 1)).mockResolvedValueOnce(summary("/p", 2));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    projects.briefStatus({ root: "/p", state: "building", message: null });
    expect(projectsState().briefs["/p"].building).toBe(true);
    projects.briefStatus({ root: "/p", state: "done", message: null });
    await settle();
    expect(projectsState().briefs["/p"].building).toBe(false);
    expect(setContext).toHaveBeenLastCalledWith("/p", "/p|2|", "");
  });

  it("editing instructions updates the project's context and saves under that project", async () => {
    const { projects } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    const next = { default: "", current: "Focus.", templateId: null };
    projects.changeInstructions("/p", next);
    expect(setContext).toHaveBeenLastCalledWith("/p", "/p|1|Focus.", "Focus.");
    await projects.flush();
    expect(api.saveProjectInstructions).toHaveBeenCalledWith("/p", next);
  });

  it('reloading under "" (how events name no project) updates the tabs with no project', async () => {
    const { projects } = await fresh();
    api.getInstructions.mockResolvedValueOnce(instructionsView("")).mockResolvedValueOnce(instructionsView("Be brief."));
    projects.ensure(null);
    await settle();
    await projects.reloadInstructions("");
    expect(api.getInstructions).toHaveBeenLastCalledWith("");
    expect(setContext).toHaveBeenLastCalledWith(null, "|0|Be brief.", "Be brief.");
  });
});

describe("instructions refetched while typing", () => {
  it("a keystroke during the refetch is kept, not replaced by the older fetched copy", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValueOnce(instructionsView("")).mockResolvedValueOnce({
      templates: [{ id: "t", name: "T", text: "new template", sections: [] }],
      project: { default: "", current: "from the other window", templateId: null },
    });
    projects.ensure("/p");
    await settle();
    const fetched = deferred<InstructionsView>();
    api.getInstructions.mockReset();
    api.getInstructions.mockReturnValue(fetched.promise);
    const reloading = projects.reloadInstructions("/p");
    await settle();
    const typed = { default: "", current: "typed meanwhile", templateId: null };
    projects.changeInstructions("/p", typed);
    fetched.resolve({
      templates: [{ id: "t", name: "T", text: "new template", sections: [] }],
      project: { default: "", current: "from the other window", templateId: null },
    });
    await reloading;
    expect(projectsState().instructions["/p"]).toEqual(typed);
    expect(projectsState().templates).toEqual([{ id: "t", name: "T", text: "new template", sections: [] }]); // templates still apply
    await projects.flush();
    expect(api.saveProjectInstructions).toHaveBeenLastCalledWith("/p", typed);
  });

  it("an edit made while the pending save flushes is kept too", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    const saving = deferred<void>();
    api.saveProjectInstructions.mockReturnValue(saving.promise);
    projects.changeInstructions("/p", { default: "", current: "first", templateId: null });
    const reloading = projects.reloadInstructions("/p"); // flushes "first", which is still saving
    await settle();
    const typed = { default: "", current: "first, then more", templateId: null };
    projects.changeInstructions("/p", typed);
    saving.resolve();
    await reloading;
    expect(projectsState().instructions["/p"]).toEqual(typed);
  });

  it("without an edit in between, the fetched copy is applied", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValueOnce(instructionsView("")).mockResolvedValueOnce(instructionsView("Other window."));
    projects.ensure("/p");
    await settle();
    await projects.reloadInstructions("/p");
    expect(projectsState().instructions["/p"].current).toBe("Other window.");
  });
});

describe("load failures and ordering", () => {
  it("instructions that fail to load push no context and are retried by ensure()", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockRejectedValueOnce("disk unreadable");
    api.getInstructions.mockResolvedValue(instructionsView("Focus."));
    projects.ensure("/p");
    await settle();
    expect(setContext).not.toHaveBeenCalled(); // empty instructions would re-run saved notes without "Focus."
    expect(projectsState().instructions["/p"]).toBeUndefined();
    expect(projectsState().instructionsErrors["/p"]).toBe("Couldn't load instructions: disk unreadable");
    expect(projectsState().instructionsErrors["/other"]).toBeUndefined(); // per root
    projects.ensure("/p");
    await settle();
    expect(setContext).toHaveBeenCalledTimes(1);
    expect(setContext).toHaveBeenCalledWith("/p", "/p|1|Focus.", "Focus.");
    expect(projectsState().instructionsErrors["/p"]).toBeUndefined(); // cleared by the successful load
  });

  it("editing instructions that haven't loaded is refused: it would save over the stored ones", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockRejectedValue("disk unreadable");
    projects.ensure("/p");
    await settle();
    projects.changeInstructions("/p", { default: "", current: "typed", templateId: null });
    await projects.flush();
    expect(projectsState().instructions["/p"]).toBeUndefined();
    expect(api.saveProjectInstructions).not.toHaveBeenCalled();
    expect(setContext).not.toHaveBeenCalled();
  });

  it("a failed save keeps the error on its own root, and a later successful save clears it", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/a", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/a");
    projects.ensure("/b");
    await settle();
    api.saveProjectInstructions.mockRejectedValueOnce("disk full");
    projects.changeInstructions("/a", { default: "", current: "x", templateId: null });
    await projects.flush();
    expect(projectsState().instructionsErrors["/a"]).toBe("Couldn't save instructions: disk full");
    expect(projectsState().instructionsErrors["/b"]).toBeUndefined();
    await projects.flush(); // retried
    expect(projectsState().instructionsErrors["/a"]).toBeUndefined();
  });

  it("reloading one root's instructions is not held up by another root's failed save", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/a", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/a");
    projects.ensure("/b");
    await settle();
    api.saveProjectInstructions.mockImplementation((root: string) => (root === "/a" ? Promise.reject("disk full") : Promise.resolve()));
    projects.changeInstructions("/a", { default: "", current: "typed in a", templateId: null });
    await projects.flush();
    api.getInstructions.mockResolvedValue(instructionsView("from disk b"));
    await projects.reloadInstructions("/b");
    expect(projectsState().instructions["/b"].current).toBe("from disk b"); // /a's owed save is not /b's
    await projects.reloadInstructions("/a");
    expect(projectsState().instructions["/a"].current).toBe("typed in a"); // /a's own is kept
  });

  it("a refetch that keeps typed text because its save failed leaves the save error up", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    api.saveProjectInstructions.mockRejectedValue("disk full");
    projects.changeInstructions("/p", { default: "", current: "typed", templateId: null });
    await projects.reloadInstructions("/p");
    expect(projectsState().instructions["/p"].current).toBe("typed");
    expect(projectsState().instructionsErrors["/p"]).toBe("Couldn't save instructions: disk full");
  });

  it("a refetch is not allowed to replace typed text whose save failed", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValue(instructionsView(""));
    projects.ensure("/p");
    await settle();
    const typed = { default: "", current: "typed", templateId: null };
    api.saveProjectInstructions.mockRejectedValue("disk full");
    projects.changeInstructions("/p", typed);
    await vi.waitFor(() => expect(projectsState().instructions["/p"]).toEqual(typed));
    api.getInstructions.mockResolvedValue({
      templates: [{ id: "t", name: "T", text: "tt", sections: [] }],
      project: { default: "", current: "older stored copy", templateId: null },
    });
    await projects.reloadInstructions("/p");
    expect(projectsState().instructions["/p"]).toEqual(typed); // the typed copy is the only one with the edit
    expect(projectsState().templates).toEqual([{ id: "t", name: "T", text: "tt", sections: [] }]);
  });

  it("a refetch started before a local template edit doesn't put the old templates back", async () => {
    const { projects, projectsState } = await fresh();
    api.projectSummary.mockResolvedValue(summary("/p", 1));
    api.getInstructions.mockResolvedValueOnce(instructionsView(""));
    api.saveTemplates.mockResolvedValue(undefined);
    projects.ensure("/p");
    await settle();
    const fetched = deferred<InstructionsView>();
    api.getInstructions.mockReturnValue(fetched.promise);
    const reloading = projects.reloadInstructions("/p");
    await settle();
    const mine = [{ id: "m", name: "Mine", text: "edited meanwhile", sections: [] }];
    await projects.changeTemplates(mine);
    fetched.resolve({ templates: [{ id: "o", name: "Old", text: "old", sections: [] }], project: { default: "", current: "other", templateId: null } });
    await reloading;
    expect(projectsState().templates).toEqual(mine);
    expect(projectsState().instructions["/p"].current).toBe("other"); // instructions were not typed: still applied
  });

  it("only the latest started summary load applies", async () => {
    const { projects, projectsState } = await fresh();
    const first = deferred<ProjectSummary>();
    const second = deferred<ProjectSummary>();
    api.getInstructions.mockResolvedValue(instructionsView(""));
    api.projectSummary.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    void projects.refreshSummary("/p");
    void projects.refreshSummary("/p");
    await projects.reloadInstructions("/p");
    second.resolve(summary("/p", 2));
    await settle();
    first.resolve(summary("/p", 1)); // lands late
    await settle();
    expect(projectsState().summaries["/p"].brief?.createdAt).toBe(2);
    expect(setContext).toHaveBeenLastCalledWith("/p", "/p|2|", "");
  });
});
