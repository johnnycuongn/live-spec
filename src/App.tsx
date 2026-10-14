import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type ProjectSummary, type Settings } from "./lib/ipc";
import { useLiveAnalysis } from "./lib/useLiveAnalysis";
import { providerLabel } from "./lib/providerOptions";
import { activityLine } from "./lib/brief";
import { TopBar } from "./components/TopBar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { SettingsDialog } from "./components/SettingsDialog";

const LAST_PROJECT = "livebrainstorm.lastProject";
const draftKey = (root: string | undefined) => `livebrainstorm.draft.${root ?? "none"}`;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [opening, setOpening] = useState(false);
  const [projectError, setProjectError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const [briefBuilding, setBriefBuilding] = useState(false);
  const [briefLive, setBriefLive] = useState("");
  const [briefActivity, setBriefActivity] = useState<string[]>([]);
  const [briefError, setBriefError] = useState<string | null>(null);
  const [showBrief, setShowBrief] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  // Opening (or re-opening) a project rescans it, so its summary is the
  // single source of truth for the brief and whether it's stale.
  const brief = project?.brief ?? null;
  const briefStatus = project?.briefStatus ?? null;
  const analysis = useLiveAnalysis(draft, settings?.debounceMs ?? 1200, `${project?.root ?? ""}|${brief?.createdAt ?? 0}`);

  const loadProject = useCallback(async (path: string) => {
    setOpening(true);
    setProjectError(null);
    try {
      const summary = await api.openProject(path);
      setProject(summary);
      setBriefError(null);
      setDraft(localStorage.getItem(draftKey(summary.root)) ?? "");
      localStorage.setItem(LAST_PROJECT, summary.root);
    } catch (e) {
      setProjectError(errorMessage(e));
    } finally {
      setOpening(false);
    }
  }, []);

  useEffect(() => {
    api.getSettings().then(setSettings);
    void api.warmUp();
    const last = localStorage.getItem(LAST_PROJECT);
    if (last) void loadProject(last);
    else setDraft(localStorage.getItem(draftKey(undefined)) ?? "");
  }, [loadProject]);

  // Drafts are per project and survive restarts.
  useEffect(() => {
    localStorage.setItem(draftKey(project?.root), draft);
  }, [draft, project?.root]);

  // Esc closes the brief drawer. The settings <dialog> handles its own Esc.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showSettings) setShowBrief(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  const pickProject = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir === "string") await loadProject(dir);
  };

  const buildBrief = () => {
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
        onOpenProject={pickProject}
        onShowBrief={() => setShowBrief((v) => !v)}
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
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
