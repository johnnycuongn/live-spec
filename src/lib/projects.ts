// Each project's data in this window, by root: its summary (the brief and
// whether it's current), its session instructions, and its brief build. Every
// tab of a project shares them. A project's analysis context comes from both
// the summary and the instructions, so it reaches tabs.ts only once both have
// loaded: a key computed earlier ("root|0|") would differ from the saved notes'
// and start a spurious run at launch.
import { useSyncExternalStore } from "react";
import { activityLine } from "./brief";
import { onBriefStatus } from "./events";
import { api, errorMessage, type ProjectInstructions, type ProjectSummary, type Template } from "./ipc";
import { analysisContextKey } from "./instructions";
import { createSaver } from "./saver";
import { tabs } from "./tabs";

export interface BriefRun {
  building: boolean;
  /** The brief as it streams in; only the window that started the build receives it. */
  live: string;
  activity: string[];
  error: string | null;
}

export const IDLE_BRIEF: BriefRun = { building: false, live: "", activity: [], error: null };

export interface ProjectsState {
  summaries: Record<string, ProjectSummary>;
  /** A project that couldn't be read (moved, deleted, unreadable): the message to show. */
  summaryErrors: Record<string, string>;
  /** Keyed by root; "" is tabs with no project. */
  instructions: Record<string, ProjectInstructions>;
  templates: Template[];
  /** Why a root's instructions couldn't load or save, by root ("" is tabs with no project). */
  instructionsErrors: Record<string, string>;
  /** Why the templates couldn't be saved. */
  templatesError: string | null;
  briefs: Record<string, BriefRun>;
}

let state: ProjectsState = {
  summaries: {},
  summaryErrors: {},
  instructions: {},
  templates: [],
  instructionsErrors: {},
  templatesError: null,
  briefs: {},
};
const listeners = new Set<() => void>();
const loading = new Set<string>();
/** The newest started summary load per root: a slower, older load that lands late is dropped. */
const summarySeq = new Map<string, number>();
const nextSeq = (root: string) => {
  const n = (summarySeq.get(root) ?? 0) + 1;
  summarySeq.set(root, n);
  return n;
};
/** Counts the edits typed into each root's instructions, so a refetch can tell the user typed meanwhile. */
const edits = new Map<string, number>();
/** The same for the templates, which every root shares. */
let templateEdits = 0;
let started = false;

function set(patch: Partial<ProjectsState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

const key = (root: string | null) => root ?? "";

function without<T>(record: Record<string, T>, k: string): Record<string, T> {
  const next = { ...record };
  delete next[k];
  return next;
}

// Instruction edits save a moment after typing stops, each under the project it was typed in. The
// error is recorded against that root here (the saver's onError doesn't know the key); a later
// successful save of the root clears it.
const saver = createSaver<ProjectInstructions>(
  (root, p) =>
    api.saveProjectInstructions(root, p).then(
      () => set({ instructionsErrors: without(state.instructionsErrors, root) }),
      (e) => {
        set({ instructionsErrors: { ...state.instructionsErrors, [root]: `Couldn't save instructions: ${errorMessage(e)}` } });
        throw e;
      },
    ),
  500,
  () => {},
);

/**
 * Hands the project's context to its tabs once both halves have loaded. A
 * project that couldn't be read hands over only the instructions (a null key):
 * its tabs keep the context their saved notes ran with, so a missing folder
 * never re-runs them at launch, and the next pause analyses without code context.
 */
function pushContext(root: string | null) {
  const instructions = state.instructions[key(root)];
  if (!instructions) return;
  if (root !== null && state.summaryErrors[root] !== undefined) {
    tabs.setContext(root, null, instructions.current);
    return;
  }
  if (root !== null && !state.summaries[root]) return;
  const brief = root === null ? null : (state.summaries[root]?.brief ?? null);
  tabs.setContext(root, analysisContextKey(root ?? undefined, brief?.createdAt, instructions.current), instructions.current);
}

async function loadSummary(root: string): Promise<void> {
  const n = nextSeq(root);
  try {
    const summary = await api.projectSummary(root);
    if (n !== summarySeq.get(root)) return; // a newer load has started
    set({ summaries: { ...state.summaries, [root]: summary }, summaryErrors: without(state.summaryErrors, root) });
  } catch (e) {
    if (n !== summarySeq.get(root)) return;
    // The tab still opens with its draft and notes, and analyses without code context.
    set({ summaryErrors: { ...state.summaryErrors, [root]: errorMessage(e) } });
  }
  pushContext(root);
}

async function loadInstructions(root: string | null): Promise<void> {
  const edited = edits.get(key(root)) ?? 0;
  const templatesEdited = templateEdits;
  const saved = await saver.flush(key(root)); // this root's pending edit must not lose to the read below; another root's owed save is not its concern
  try {
    const view = await api.getInstructions(key(root));
    // Typed since this load began: the fetched copy is older than the text on screen. Keep the text.
    // So is text whose save failed (flush false): the stored copy lacks it, and this is the only copy.
    const keepTyped = state.instructions[key(root)] !== undefined && (!saved || (edits.get(key(root)) ?? 0) !== edited);
    set({
      // Templates edited here during the fetch are newer than the fetched ones, too.
      ...(templateEdits === templatesEdited ? { templates: view.templates } : {}),
      ...(keepTyped ? {} : { instructions: { ...state.instructions, [key(root)]: view.project } }),
      // Kept text means a failed save (or a newer edit): its error stays until a save succeeds.
      ...(keepTyped ? {} : { instructionsErrors: without(state.instructionsErrors, key(root)) }),
    });
  } catch (e) {
    // Not loaded, not empty: empty instructions would re-run a tab's saved notes without the
    // ones they were made with, and typing could overwrite the saved file. ensure() retries.
    set({ instructionsErrors: { ...state.instructionsErrors, [key(root)]: `Couldn't load instructions: ${errorMessage(e)}` } });
    return;
  }
  pushContext(root);
}

function once(id: string, load: () => Promise<void>) {
  if (loading.has(id)) return;
  loading.add(id);
  void load().finally(() => loading.delete(id));
}

/** Loads what a shown tab's project needs. A project that couldn't be read is retried each time. */
function ensure(root: string | null): void {
  if (root !== null && !state.summaries[root]) once(`s:${root}`, () => loadSummary(root));
  if (!state.instructions[key(root)]) once(`i:${key(root)}`, () => loadInstructions(root));
}

/** "Open a project…": scans the folder (replacing any loaded index) and returns its root. */
async function openFolder(path: string): Promise<string> {
  const summary = await api.openProject(path);
  nextSeq(summary.root); // supersedes any summary load still in flight for this root
  set({ summaries: { ...state.summaries, [summary.root]: summary }, summaryErrors: without(state.summaryErrors, summary.root) });
  pushContext(summary.root);
  return summary.root;
}

function changeInstructions(root: string | null, next: ProjectInstructions): void {
  // Not loaded yet: the edit would be saved over the stored instructions it never saw.
  if (!state.instructions[key(root)]) return;
  edits.set(key(root), (edits.get(key(root)) ?? 0) + 1);
  set({ instructions: { ...state.instructions, [key(root)]: next }, instructionsErrors: without(state.instructionsErrors, key(root)) });
  saver.schedule(key(root), next);
  pushContext(root); // re-reads the shown tab's draft, as editing instructions does today
}

async function changeTemplates(next: Template[]): Promise<void> {
  templateEdits++;
  set({ templates: next });
  try {
    await api.saveTemplates(next);
    set({ templatesError: null });
  } catch (e) {
    set({ templatesError: `Couldn't save templates: ${errorMessage(e)}` });
  }
}

async function restoreStarters(): Promise<void> {
  templateEdits++;
  try {
    set({ templates: await api.restoreStarterTemplates(), templatesError: null });
  } catch (e) {
    set({ templatesError: `Couldn't save templates: ${errorMessage(e)}` });
  }
}

function patchBrief(root: string, f: (b: BriefRun) => BriefRun) {
  set({ briefs: { ...state.briefs, [root]: f(state.briefs[root] ?? IDLE_BRIEF) } });
}

function buildBrief(root: string): void {
  if (state.briefs[root]?.building) return; // one build per project; the backend refuses a second too
  patchBrief(root, () => ({ building: true, live: "", activity: [], error: null }));
  api
    .buildBrief(root, (ev) => {
      if (ev.type === "text") {
        const text = ev.text;
        patchBrief(root, (b) => ({ ...b, live: b.live + text }));
      } else if (ev.type === "activity") {
        const line = activityLine(ev.label, ev.lane);
        patchBrief(root, (b) => ({ ...b, activity: [...b.activity, line] }));
      } else if (ev.type === "done") {
        patchBrief(root, (b) => ({ ...b, building: false }));
      } else if (ev.type === "error") {
        const message = ev.message;
        patchBrief(root, (b) => ({ ...b, building: false, error: message }));
      }
    })
    .catch((e) => patchBrief(root, (b) => ({ ...b, building: false, error: errorMessage(e) })));
}

/** `brief-status` from any window. On done, the rescanned summary carries the new brief to every tab of the project. */
function briefStatus(e: { root: string; state: string; message?: string | null }): void {
  if (e.state === "building") {
    patchBrief(e.root, (b) => (b.building ? b : { ...b, building: true, error: null }));
  } else if (e.state === "done") {
    patchBrief(e.root, (b) => ({ ...b, building: false }));
    void loadSummary(e.root);
  } else if (e.state === "error") {
    const message = e.message ?? "Couldn't build the project brief.";
    patchBrief(e.root, (b) => ({ ...b, building: false, error: b.error ?? message }));
  }
}

function start(): void {
  if (started) return; // StrictMode runs effects twice
  started = true;
  ensure(null); // the templates, and the instructions for tabs with no project
  void onBriefStatus((e) => briefStatus(e));
  if (typeof window !== "undefined") window.addEventListener("pagehide", () => void saver.flush());
}

export const projects = {
  start,
  ensure,
  openFolder,
  refreshSummary: loadSummary,
  // Events name the no-project session "" (as the instructions store does); here it is null.
  reloadInstructions: (root: string | null) => loadInstructions(root || null),
  changeInstructions,
  changeTemplates,
  restoreStarters,
  buildBrief,
  briefStatus,
  /** Saves every root's pending instructions (all, when the window closes); false if any save failed. */
  flush: () => saver.flush(),
};

export function projectsState(): ProjectsState {
  return state;
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export function useProjects(): ProjectsState {
  return useSyncExternalStore(subscribe, projectsState);
}
