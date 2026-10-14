import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { initialAnalysis } from "./lib/analysisState";
import { activityLine } from "./lib/brief";
import { onBriefStatus } from "./lib/events";
import { api, errorMessage, type ProjectInstructions, type ProjectSummary, type Settings, type Tab, type Template } from "./lib/ipc";
import { EMPTY_INSTRUCTIONS, instructionsLabel } from "./lib/instructions";
import { browserStorage, importLegacy } from "./lib/legacyDrafts";
import { providerLabel } from "./lib/providerOptions";
import { createSaver } from "./lib/saver";
import { tabContext, type LoadedProject } from "./lib/tabContext";
import { TabSession, type TabSessionDeps } from "./lib/tabSession";
import { useTabSession } from "./lib/useTabSession";
import { currentWindowId } from "./lib/windowId";
import { TopBar } from "./components/TopBar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { InstructionsDrawer } from "./components/InstructionsDrawer";
import { TemplateList } from "./components/TemplateList";
import { SettingsDialog } from "./components/SettingsDialog";

/** One project's brief build, as this window sees it. */
interface BriefRun {
  building: boolean;
  live: string;
  activity: string[];
  error: string | null;
}

const IDLE_BRIEF: BriefRun = { building: false, live: "", activity: [], error: null };

/** A file the backend kept as `.bad`, named for the banner. */
function unreadableNotice(path: string): string {
  const file = path.split(/[\\/]/).at(-1) ?? path;
  return file.startsWith("workspace.json")
    ? `Couldn't read the saved windows; kept them as ${file}`
    : `Couldn't read a tab; kept it as ${file}`;
}

export default function App() {
  const windowId = useMemo(() => currentWindowId(), []);
  const [settings, setSettings] = useState<Settings | null>(null);
  const settingsRef = useRef<Settings | null>(null);
  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  // The window's active tab: its live analysis, and the project it reads.
  const [session, setSession] = useState<TabSession | null>(null);
  const sessionRef = useRef<TabSession | null>(null);
  const view = useTabSession(session);
  const root = session?.root ?? null;
  const [project, setProject] = useState<LoadedProject | null>(null);
  const [opening, setOpening] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [notices, setNotices] = useState<string[]>([]);

  const [instructions, setInstructions] = useState<ProjectInstructions>(EMPTY_INSTRUCTIONS);
  // Whose instructions `instructions` holds ("" = no project); null while they load.
  const [instructionsRoot, setInstructionsRoot] = useState<string | null>(null);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [instructionsError, setInstructionsError] = useState<string | null>(null);
  const [showInstructions, setShowInstructions] = useState(false);

  const [briefs, setBriefs] = useState<Record<string, BriefRun>>({});
  const [showBrief, setShowBrief] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Draft edits save 500 ms after typing stops, each under the tab it was typed in.
  const draftSaver = useMemo(
    () =>
      createSaver<string>(
        (tabId, draft) => api.saveDraft(tabId, draft).then(() => setSaveError(null)),
        500,
        (e) => setSaveError(`Couldn't save your draft: ${errorMessage(e)}`),
      ),
    [],
  );
  // Instruction edits, the same way, each under the project they were typed in.
  const instructionsSaver = useMemo(
    () =>
      createSaver<ProjectInstructions>(api.saveProjectInstructions, 500, (e) =>
        setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`),
      ),
    [],
  );
  useEffect(() => {
    // Closing the window doesn't unmount React, so flush when the page goes away too.
    const flush = () => {
      void draftSaver.flush();
      void instructionsSaver.flush();
    };
    window.addEventListener("pagehide", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      flush();
    };
  }, [draftSaver, instructionsSaver]);

  const deps = useMemo<TabSessionDeps>(
    () => ({
      analyze: api.analyze,
      saveDraft: (tabId, draft) => draftSaver.schedule(tabId, draft),
      debounceMs: () => settingsRef.current?.debounceMs ?? 1200,
    }),
    [draftSaver],
  );

  const patchBrief = (r: string, f: (b: BriefRun) => BriefRun) =>
    setBriefs((all) => ({ ...all, [r]: f(all[r] ?? IDLE_BRIEF) }));

  /** The shown tab's project summary. A folder that can't be read keeps its message. */
  const loadSummary = async (r: string) => {
    try {
      const summary = await api.projectSummary(r);
      if (sessionRef.current?.root === r) setProject({ root: r, summary, error: null });
    } catch (e) {
      // The tab still shows its draft and notes, and analyses without code context.
      if (sessionRef.current?.root === r) setProject({ root: r, summary: null, error: errorMessage(e) });
    }
  };

  /** The shown tab's instructions ("" for no project), and the shared templates. */
  const loadInstructions = async (r: string | null) => {
    await instructionsSaver.flush(); // a pending edit must not lose to the read below
    try {
      const v = await api.getInstructions(r ?? "");
      setTemplates(v.templates);
      if (sessionRef.current?.root !== r) return;
      setInstructions(v.project);
      setInstructionsError(null);
    } catch (e) {
      if (sessionRef.current?.root !== r) return;
      setInstructions(EMPTY_INSTRUCTIONS);
      setInstructionsError(`Couldn't load instructions: ${errorMessage(e)}`);
    }
    setInstructionsRoot(r ?? "");
  };

  /** Shows `tab`: a new session for it, then its project and instructions. `known` is a summary just fetched. */
  const show = (tab: Tab, known?: ProjectSummary) => {
    sessionRef.current?.dispose(); // a run it started finishes in the backend and saves to its own tab
    const s = new TabSession(tab, deps);
    sessionRef.current = s;
    setSession(s);
    setShowBrief(false);
    setShowInstructions(false);
    setInstructionsRoot(null);
    setProject(tab.root === null ? null : { root: tab.root, summary: known ?? null, error: null });
    void loadInstructions(tab.root);
    if (tab.root !== null && !known) void loadSummary(tab.root);
  };

  // Load this window's tabs once. StrictMode runs effects twice; the ref keeps it to one load.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    api.getSettings().then(setSettings);
    void api.warmUp();
    // Templates are shared, so load them even if this window's tabs don't load
    // (a template save writes the whole list).
    void api.getInstructions("").then((v) => setTemplates(v.templates));
    void (async () => {
      try {
        let ws = await api.listWorkspace(windowId);
        if (ws.fresh) {
          // The first launch of this build: the drafts the old build kept in localStorage become tabs.
          const failed = await importLegacy(windowId, browserStorage(), api.importLegacyDrafts);
          if (failed) setLoadError(failed);
          ws = await api.listWorkspace(windowId);
        }
        setNotices(ws.unreadable.map(unreadableNotice));
        let tab = ws.active;
        if (!tab && ws.tabs.length > 0) {
          await api.activateTab(windowId, ws.tabs[0].id);
          tab = await api.getTab(ws.tabs[0].id);
        }
        // An empty window still takes a draft: a tab with no project, as before tabs.
        show(tab ?? (await api.createTab(windowId, null)));
      } catch (e) {
        setLoadError(errorMessage(e));
      }
    })();
  }, []);

  // A brief built in any window: the rescanned summary carries the new brief,
  // and the new context re-reads the draft.
  useEffect(() => {
    const unlisten = onBriefStatus((e) => {
      if (e.state === "building") {
        patchBrief(e.root, (b) => (b.building ? b : { ...b, building: true, error: null }));
      } else if (e.state === "done") {
        patchBrief(e.root, (b) => ({ ...b, building: false }));
        if (sessionRef.current?.root === e.root) void loadSummary(e.root);
      } else {
        const message = e.message ?? "Couldn't build the project brief.";
        patchBrief(e.root, (b) => ({ ...b, building: false, error: b.error ?? message }));
      }
    });
    return () => void unlisten.then((u) => u());
  }, []);

  // The tab's context reaches its session only once both its project summary
  // and its instructions have loaded.
  const context = tabContext(root, project, instructionsRoot === null ? null : { root: instructionsRoot, current: instructions.current });
  useEffect(() => {
    if (session && context) session.setContext(context.key, context.instructions);
  }, [session, context?.key, context?.instructions]);

  // Esc closes either drawer. The settings <dialog> handles its own Esc.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showSettings) {
        setShowBrief(false);
        setShowInstructions(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  /** "Open a project…": this window's tab for that project, or a new one, as drafts were per project before tabs. */
  const openFolder = async (path: string) => {
    setOpening(true);
    setLoadError(null);
    try {
      await draftSaver.flush(); // the tab you leave keeps its last edit
      const summary = await api.openProject(path); // scans, replacing any loaded index
      if (sessionRef.current?.root === summary.root) {
        setProject({ root: summary.root, summary, error: null }); // a rescan of the project on screen
        return;
      }
      const ws = await api.listWorkspace(windowId);
      const existing = ws.tabs.find((t) => t.root === summary.root);
      let tab: Tab;
      if (existing) {
        await api.activateTab(windowId, existing.id);
        tab = await api.getTab(existing.id);
      } else {
        tab = await api.createTab(windowId, summary.root);
      }
      show(tab, summary);
    } catch (e) {
      setLoadError(errorMessage(e));
    } finally {
      setOpening(false);
    }
  };

  const pickProject = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir === "string") await openFolder(dir);
  };

  const changeInstructions = (next: ProjectInstructions) => {
    setInstructions(next);
    setInstructionsError(null);
    instructionsSaver.schedule(root ?? "", next);
  };

  const changeTemplates = async (next: Template[]) => {
    setTemplates(next);
    try {
      await api.saveTemplates(next);
      setInstructionsError(null);
    } catch (e) {
      setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`);
    }
  };

  const restoreStarters = async () => {
    try {
      setTemplates(await api.restoreStarterTemplates());
      setInstructionsError(null);
    } catch (e) {
      setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`);
    }
  };

  const buildBrief = () => {
    if (root === null || briefs[root]?.building) return;
    const r = root;
    setShowInstructions(false);
    setShowBrief(true);
    patchBrief(r, () => ({ building: true, live: "", activity: [], error: null }));
    api
      .buildBrief(r, (ev) => {
        if (ev.type === "text") {
          const text = ev.text;
          patchBrief(r, (b) => ({ ...b, live: b.live + text }));
        } else if (ev.type === "activity") {
          const line = activityLine(ev.label, ev.lane);
          patchBrief(r, (b) => ({ ...b, activity: [...b.activity, line] }));
        } else if (ev.type === "done") {
          // The summary refreshes on `brief-status: done`, which every window receives.
          patchBrief(r, (b) => ({ ...b, building: false }));
        } else if (ev.type === "error") {
          const message = ev.message;
          patchBrief(r, (b) => ({ ...b, building: false, error: message }));
        }
      })
      .catch((e) => patchBrief(r, (b) => ({ ...b, building: false, error: errorMessage(e) })));
  };

  const saveSettings = async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
  };

  const summary = project !== null && project.root === root ? project.summary : null;
  const brief = summary?.brief ?? null;
  const run = root !== null ? (briefs[root] ?? IDLE_BRIEF) : IDLE_BRIEF;
  const loadingProject = root !== null && summary === null && (project?.error ?? null) === null;
  const errors = [loadError, project !== null && project.root === root ? project.error : null, saveError].filter(
    (m): m is string => !!m,
  );
  const analysis = view
    ? { ...view.analysis, stale: view.stale, rerun: () => session?.rerun() }
    : { ...initialAnalysis, stale: false, rerun: () => {} };

  return (
    <div className="flex h-full flex-col">
      <TopBar
        project={summary}
        opening={opening || loadingProject}
        briefStatus={summary?.briefStatus ?? null}
        briefBuilding={run.building}
        providerLabel={providerLabel(settings)}
        instructionsLabel={instructionsLabel(instructions, templates)}
        onOpenProject={pickProject}
        onShowBrief={() => {
          setShowInstructions(false);
          setShowBrief((v) => !v);
        }}
        onShowInstructions={() => {
          setShowBrief(false);
          setShowInstructions((v) => !v);
        }}
        onOpenSettings={() => setShowSettings(true)}
      />
      {errors.map((m, i) => (
        <p key={i} className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-fault" role="alert">
          {m}
        </p>
      ))}
      {notices.map((m, i) => (
        <p key={i} className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-muted">
          {m}
        </p>
      ))}

      {/* Both panes fill the remaining height and scroll on their own. */}
      <main className="relative grid min-h-0 flex-1 grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]">
        <DraftPane value={view?.draft ?? ""} onChange={(text) => session?.setDraft(text)} hasProject={!!summary} />
        <NotesPane analysis={analysis} hasProject={!!summary} briefMissing={!brief && !run.building} onBuildBrief={buildBrief} />
        {showBrief && summary && (
          <BriefDrawer
            brief={brief}
            status={summary.briefStatus}
            building={run.building}
            liveText={run.live}
            activity={run.activity}
            error={run.error}
            onBuild={buildBrief}
            onClose={() => setShowBrief(false)}
          />
        )}
        {showInstructions && instructionsRoot !== null && (
          <InstructionsDrawer
            key={instructionsRoot}
            instructions={instructions}
            templates={templates}
            localModel={settings?.provider.kind === "local"}
            error={instructionsError}
            onChange={changeInstructions}
            onClose={() => setShowInstructions(false)}
          >
            <TemplateList templates={templates} current={instructions.current} onSave={changeTemplates} onRestore={restoreStarters} />
          </InstructionsDrawer>
        )}
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
