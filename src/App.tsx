import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type InstructionsView, type ProjectInstructions, type ProjectSummary, type Settings, type Template } from "./lib/ipc";
import { useLiveAnalysis } from "./lib/useLiveAnalysis";
import { activityLine } from "./lib/brief";
import { providerLabel } from "./lib/providerOptions";
import { EMPTY_INSTRUCTIONS, analysisContextKey, instructionsLabel } from "./lib/instructions";
import { createSaver } from "./lib/saver";
import { TopBar } from "./components/TopBar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { InstructionsDrawer } from "./components/InstructionsDrawer";
import { SettingsDialog } from "./components/SettingsDialog";

const LAST_PROJECT = "livebrainstorm.lastProject";
const draftKey = (root: string | undefined) => `livebrainstorm.draft.${root ?? "none"}`;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [opening, setOpening] = useState(false);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const [instructions, setInstructions] = useState<ProjectInstructions>(EMPTY_INSTRUCTIONS);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [instructionsError, setInstructionsError] = useState<string | null>(null);
  const [showInstructions, setShowInstructions] = useState(false);
  // Which project's instructions are on screen: null until one opens.
  const openRoot = useRef<string | null>(null);

  const [briefBuilding, setBriefBuilding] = useState(false);
  const [briefLive, setBriefLive] = useState("");
  const [briefActivity, setBriefActivity] = useState<string[]>([]);
  const [briefError, setBriefError] = useState<string | null>(null);
  const [showBrief, setShowBrief] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Edits save a moment after typing stops, each under the project it was typed in.
  const saver = useMemo(
    () =>
      createSaver<ProjectInstructions>(api.saveProjectInstructions, 500, (e) =>
        setInstructionsError(`Couldn't save instructions: ${errorMessage(e)}`),
      ),
    [],
  );
  useEffect(() => () => void saver.flush(), [saver]);

  // Opening (or re-opening) a project rescans it, so its summary is the
  // single source of truth for the brief and whether it's stale.
  const brief = project?.brief ?? null;
  const briefStatus = project?.briefStatus ?? null;
  // Changing the instructions re-reads the same draft, like a new project or brief.
  const analysis = useLiveAnalysis(
    draft,
    settings?.debounceMs ?? 1200,
    analysisContextKey(project?.root, brief?.createdAt, instructions.current),
    instructions.current,
  );

  const showView = (view: InstructionsView) => {
    setInstructions(view.project);
    setTemplates(view.templates);
    setInstructionsError(null);
  };

  const loadProject = useCallback(
    async (path: string) => {
      setOpening(true);
      setProjectError(null);
      await saver.flush(); // a pending edit belongs to the project it was typed in
      try {
        const summary = await api.openProject(path);
        // Fetched before any state changes, so the project and its instructions
        // land in one render and the first analysis already has them.
        const view = await api.getInstructions(summary.root);
        openRoot.current = summary.root;
        setProject(summary);
        showView(view);
        setBriefError(null);
        setDraft(localStorage.getItem(draftKey(summary.root)) ?? "");
        localStorage.setItem(LAST_PROJECT, summary.root);
      } catch (e) {
        setProjectError(errorMessage(e));
      } finally {
        setOpening(false);
      }
    },
    [saver],
  );

  useEffect(() => {
    api.getSettings().then(setSettings);
    void api.warmUp();
    // Templates are shared, so load them even if the last project fails to
    // open (a template save writes the whole list). The no-project
    // instructions apply until a project opens.
    void api.getInstructions("").then((view) => {
      setTemplates(view.templates);
      if (openRoot.current === null) setInstructions(view.project);
    });
    const last = localStorage.getItem(LAST_PROJECT);
    if (last) void loadProject(last);
    else setDraft(localStorage.getItem(draftKey(undefined)) ?? "");
  }, [loadProject]);

  // Drafts are per project and survive restarts.
  useEffect(() => {
    localStorage.setItem(draftKey(project?.root), draft);
  }, [draft, project?.root]);

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

  const pickProject = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir === "string") await loadProject(dir);
  };

  const changeInstructions = (next: ProjectInstructions) => {
    setInstructions(next);
    setInstructionsError(null);
    saver.schedule(project?.root ?? "", next);
  };

  const buildBrief = () => {
    setShowInstructions(false);
    setShowBrief(true);
    setBriefBuilding(true);
    setBriefLive("");
    setBriefActivity([]);
    setBriefError(null);
    api
      .buildBrief((ev) => {
        if (ev.type === "text") setBriefLive((t) => t + ev.text);
        else if (ev.type === "activity") setBriefActivity((a) => [...a, activityLine(ev.label, ev.lane)]);
        else if (ev.type === "done") {
          setBriefBuilding(false);
          // Rescan: picks up the saved brief and any files changed meanwhile.
          if (project) void loadProject(project.root);
        } else if (ev.type === "error") {
          setBriefBuilding(false);
          setBriefError(ev.message);
        }
      })
      .catch((e) => {
        setBriefBuilding(false);
        setBriefError(errorMessage(e));
      });
  };

  const saveSettings = async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
  };

  return (
    <div className="flex h-full flex-col">
      <TopBar
        project={project}
        opening={opening}
        briefStatus={briefStatus}
        briefBuilding={briefBuilding}
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
      {projectError && (
        <p className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-fault" role="alert">
          {projectError}
        </p>
      )}

      {/* Both panes fill the remaining height and scroll on their own. */}
      <main className="relative grid min-h-0 flex-1 grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]">
        <DraftPane value={draft} onChange={setDraft} hasProject={!!project} />
        <NotesPane analysis={analysis} hasProject={!!project} briefMissing={!brief && !briefBuilding} onBuildBrief={buildBrief} />
        {showBrief && project && (
          <BriefDrawer
            brief={brief}
            status={briefStatus}
            building={briefBuilding}
            liveText={briefLive}
            activity={briefActivity}
            error={briefError}
            onBuild={buildBrief}
            onClose={() => setShowBrief(false)}
          />
        )}
        {showInstructions && (
          <InstructionsDrawer
            key={project?.root ?? ""}
            instructions={instructions}
            templates={templates}
            localModel={settings?.provider.kind === "local"}
            error={instructionsError}
            onChange={changeInstructions}
            onClose={() => setShowInstructions(false)}
          />
        )}
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
