import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type Settings } from "./lib/ipc";
import { providerLabel } from "./lib/providerOptions";
import { EMPTY_INSTRUCTIONS, instructionsLabel } from "./lib/instructions";
import { useActions } from "./lib/actions";
import { IDLE_BRIEF, projects, useProjects } from "./lib/projects";
import { projectName, readCollapsed, rowTitle, windowTitle, writeCollapsed } from "./lib/tabList";
import { sessionFor, tabs, useWorkspace } from "./lib/tabs";
import { useTabSession } from "./lib/useTabSession";
import { currentWindowId } from "./lib/windowId";
import { TopBar } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { DraftPane } from "./components/DraftPane";
import { NotesPane } from "./components/NotesPane";
import { BriefDrawer } from "./components/BriefDrawer";
import { InstructionsDrawer } from "./components/InstructionsDrawer";
import { TemplateList } from "./components/TemplateList";
import { SettingsDialog } from "./components/SettingsDialog";
import { Button, cx } from "./components/ui";

export default function App() {
  const ws = useWorkspace();
  const pj = useProjects();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [showInstructions, setShowInstructions] = useState(false);
  /** The project whose brief drawer is open; any project in the sidebar can open it. */
  const [briefRoot, setBriefRoot] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(() => readCollapsed(currentWindowId(), window.innerWidth));

  const session = ws.activeId ? (sessionFor(ws.activeId) ?? null) : null;
  const view = useTabSession(session);
  const root = session?.root ?? null;
  const summary = root !== null ? (pj.summaries[root] ?? null) : null;
  const briefRun = root !== null ? (pj.briefs[root] ?? IDLE_BRIEF) : IDLE_BRIEF;
  const instructions = pj.instructions[root ?? ""] ?? EMPTY_INSTRUCTIONS;
  const activeSummary = ws.view?.tabs.find((t) => t.id === ws.activeId) ?? null;
  const title = activeSummary ? rowTitle(activeSummary, view?.draft ?? null) : null;

  useEffect(() => {
    void api.getSettings().then((s) => {
      setSettings(s);
      tabs.setDebounce(s.debounceMs);
    });
    void api.warmUp();
    projects.start();
    void tabs.load();
  }, []);

  // A shown tab's project loads on first show; its context reaches the tab once summary and instructions are in.
  useEffect(() => {
    if (session) projects.ensure(session.root);
  }, [session]);

  // An "Open a project…" failure belongs to the moment it happened, not to the tab shown next.
  useEffect(() => {
    setOpenError(null);
  }, [ws.activeId]);

  useEffect(() => {
    document.title = windowTitle(title);
  }, [title]);

  // Esc closes either drawer. The settings <dialog> handles its own Esc.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showSettings) {
        setBriefRoot(null);
        setShowInstructions(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showSettings]);

  const toggleSidebar = () => {
    const next = !collapsed;
    writeCollapsed(currentWindowId(), next);
    setCollapsed(next);
  };

  // "Open a project…": a new tab on the chosen folder.
  const openFolder = async () => {
    const dir = await open({ directory: true, title: "Choose a project folder" });
    if (typeof dir !== "string") return;
    setOpening(true);
    setOpenError(null);
    try {
      await tabs.newTab(await projects.openFolder(dir));
    } catch (e) {
      setOpenError(errorMessage(e));
    } finally {
      setOpening(false);
    }
  };

  const showBrief = (r: string) => {
    setShowInstructions(false);
    projects.ensure(r);
    setBriefRoot((current) => (current === r ? null : r));
  };

  const buildBrief = (r: string) => {
    setShowInstructions(false);
    setBriefRoot(r);
    projects.ensure(r);
    projects.buildBrief(r);
  };

  // Phase 3 adds new-window and close-window, and routes its menu events here.
  useActions({
    "new-tab": () => void tabs.newTab(root),
    "open-project": () => void openFolder(),
    "reopen-tab": () => {
      const last = ws.view?.closed[0];
      if (last) void tabs.reopen(last.id);
    },
    "close-tab": () => {
      if (ws.activeId) void tabs.close(ws.activeId);
    },
    "next-tab": () => tabs.next(1),
    "prev-tab": () => tabs.next(-1),
    "toggle-sidebar": toggleSidebar,
    settings: () => setShowSettings(true),
  });

  const saveSettings = async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
    tabs.setDebounce(s.debounceMs);
  };

  // A folder that couldn't be opened or read: shown under the top bar, as a Precondition message.
  // Instructions that couldn't load stop the tab's analyses, so they show here too (not their save errors).
  const alert =
    openError ??
    (root !== null ? (pj.summaryErrors[root] ?? null) : null) ??
    (pj.instructions[root ?? ""] ? null : pj.instructionsError);
  const drawerSummary = briefRoot !== null ? (pj.summaries[briefRoot] ?? null) : null;
  const drawerRun = briefRoot !== null ? (pj.briefs[briefRoot] ?? IDLE_BRIEF) : IDLE_BRIEF;

  return (
    <div className="flex h-full flex-col">
      <TopBar
        tab={
          session
            ? {
                projectName: root === null ? null : projectName(root),
                briefStatus: summary?.briefStatus ?? null,
                briefBuilding: briefRun.building,
                instructionsLabel: instructionsLabel(instructions, pj.templates),
              }
            : null
        }
        providerLabel={providerLabel(settings)}
        sidebarOpen={!collapsed}
        onToggleSidebar={toggleSidebar}
        onShowBrief={() => {
          if (root !== null) showBrief(root);
        }}
        onShowInstructions={() => {
          setBriefRoot(null);
          setShowInstructions((v) => !v);
        }}
        onOpenSettings={() => setShowSettings(true)}
      />
      {alert && (
        <p className="shrink-0 border-b border-line px-4 py-2 text-[13px] text-fault" role="alert">
          {alert}
        </p>
      )}

      {/* Sidebar, draft and notes. The draft and notes panes scroll on their own. */}
      <main
        className={cx(
          "relative grid min-h-0 flex-1",
          collapsed ? "grid-cols-[minmax(0,3fr)_minmax(320px,2fr)]" : "grid-cols-[248px_minmax(0,3fr)_minmax(320px,2fr)]",
        )}
      >
        <Sidebar
          hidden={collapsed}
          opening={opening}
          providerLabel={providerLabel(settings)}
          onOpenProject={() => void openFolder()}
          onShowBrief={showBrief}
          onBuildBrief={buildBrief}
          onOpenSettings={() => setShowSettings(true)}
        />
        {session && view ? (
          <>
            {/* Keyed by tab, so the textarea's undo history and the notes' local state never cross tabs.
                Sibling keys must differ, hence the prefixes. */}
            <DraftPane
              key={`draft:${session.id}`}
              value={view.draft}
              onChange={(text) => session.setDraft(text)}
              hasProject={root !== null}
            />
            <NotesPane
              key={`notes:${session.id}`}
              view={view}
              hasProject={root !== null}
              briefMissing={summary !== null && !summary.brief && !briefRun.building}
              onBuildBrief={() => {
                if (root !== null) buildBrief(root);
              }}
              onRerun={() => session.rerun()}
              onView={(i) => session.view(i)}
            />
          </>
        ) : (
          <section className="col-span-2 grid place-items-center bg-page" aria-label="No tab open">
            {!ws.view && ws.error && (
              <p className="text-[13px] text-fault" role="alert">
                {ws.error}
              </p>
            )}
            {ws.view && (
              <div className="grid justify-items-center gap-3 text-center">
                <p className="text-[13px] text-muted">No tab is open in this window.</p>
                <div className="flex gap-2">
                  <Button variant="primary" onClick={() => void openFolder()} disabled={opening}>
                    {opening ? "Reading project…" : "Open a project…"}
                  </Button>
                  <Button variant="ghost" onClick={() => void tabs.newTab(null)}>
                    New tab without a project
                  </Button>
                </div>
              </div>
            )}
          </section>
        )}
        {briefRoot !== null && (
          <BriefDrawer
            projectName={projectName(briefRoot)}
            brief={drawerSummary?.brief ?? null}
            status={drawerSummary?.briefStatus ?? null}
            building={drawerRun.building}
            liveText={drawerRun.live}
            activity={drawerRun.activity}
            error={drawerRun.error}
            onBuild={() => buildBrief(briefRoot)}
            onClose={() => setBriefRoot(null)}
          />
        )}
        {showInstructions && session && (
          <InstructionsDrawer
            key={root ?? ""}
            instructions={instructions}
            templates={pj.templates}
            localModel={settings?.provider.kind === "local"}
            error={pj.instructionsError}
            onChange={(next) => projects.changeInstructions(root, next)}
            onClose={() => setShowInstructions(false)}
          >
            <TemplateList
              templates={pj.templates}
              current={instructions.current}
              onSave={(next) => void projects.changeTemplates(next)}
              onRestore={() => void projects.restoreStarters()}
            />
          </InstructionsDrawer>
        )}
      </main>

      {showSettings && settings && <SettingsDialog settings={settings} onSave={saveSettings} onClose={() => setShowSettings(false)} />}
    </div>
  );
}
