import type { BriefStatus } from "../lib/ipc";
import { Button, cx } from "./ui";

/** What the top bar shows about the active tab. */
export interface TopBarTab {
  /** Null for a tab with no project. */
  projectName: string | null;
  /** Null until the project's summary has loaded. */
  briefStatus: BriefStatus | null;
  briefBuilding: boolean;
  instructionsLabel: string;
}

interface Props {
  tab: TopBarTab | null;
  providerLabel: string;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  onShowBrief: () => void;
  onShowInstructions: () => void;
  onOpenSettings: () => void;
}

const BRIEF_LABEL: Record<BriefStatus, string> = {
  fresh: "Project brief is current",
  stale: "Files changed since the brief",
  missing: "No project brief yet",
};

const BRIEF_DOT: Record<BriefStatus | "building", string> = {
  fresh: "bg-accent",
  stale: "bg-question",
  missing: "bg-faint",
  building: "bg-accent animate-pulse-dot motion-reduce:animate-none",
};

export function TopBar(p: Props) {
  const t = p.tab;
  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
      <Button
        variant="mini"
        aria-label={p.sidebarOpen ? "Hide sidebar" : "Show sidebar"}
        aria-expanded={p.sidebarOpen}
        title="Show or hide the sidebar"
        onClick={p.onToggleSidebar}
      >
        <svg viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" aria-hidden>
          <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
          <path d="M6 2.5v11" />
        </svg>
      </Button>
      <span className="mr-1 text-[15px] font-semibold tracking-tight" aria-hidden>
        Live Spec
      </span>

      {t && (
        <>
          <span className={cx("min-w-0 truncate text-[13px] font-medium", t.projectName === null && "text-muted")}>
            {t.projectName ?? "No project"}
          </span>

          {t.projectName !== null && (t.briefStatus !== null || t.briefBuilding) && (
            <Button variant="ghost" onClick={p.onShowBrief}>
              <span className={cx("size-2 rounded-full", BRIEF_DOT[t.briefBuilding ? "building" : (t.briefStatus ?? "missing")])} aria-hidden />
              {t.briefBuilding ? "Building project brief…" : BRIEF_LABEL[t.briefStatus ?? "missing"]}
            </Button>
          )}

          <Button variant="ghost" className="min-w-0" onClick={p.onShowInstructions} title="Session instructions">
            <span className="max-w-64 truncate">Instructions: {t.instructionsLabel}</span>
          </Button>

          <span className="flex-1" />
          <Button variant="ghost" onClick={p.onOpenSettings} title="Model and settings">
            {p.providerLabel}
          </Button>
        </>
      )}
    </header>
  );
}
