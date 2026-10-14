import type { BriefStatus, ProjectSummary } from "../lib/ipc";
import { Button, cx } from "./ui";

interface Props {
  project: ProjectSummary | null;
  opening: boolean;
  briefStatus: BriefStatus | null;
  briefBuilding: boolean;
  providerLabel: string;
  onOpenProject: () => void;
  onShowBrief: () => void;
  instructionsLabel: string;
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
  const brief = p.briefBuilding ? "building" : (p.briefStatus ?? "missing");
  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line px-4">
      <span className="mr-1 text-[15px] font-semibold tracking-tight" aria-hidden>
        LiveBrainstorm
      </span>

      <Button className="font-medium" onClick={p.onOpenProject} disabled={p.opening}>
        {p.opening ? "Reading project…" : p.project ? p.project.name : "Open a project"}
      </Button>

      {p.project && (
        <Button variant="ghost" onClick={p.onShowBrief}>
          <span className={cx("size-2 rounded-full", BRIEF_DOT[brief])} aria-hidden />
          {p.briefBuilding ? "Building project brief…" : BRIEF_LABEL[p.briefStatus ?? "missing"]}
        </Button>
      )}

      <Button variant="ghost" className="min-w-0" onClick={p.onShowInstructions} title="Session instructions">
        <span className="max-w-64 truncate">Instructions: {p.instructionsLabel}</span>
      </Button>

      <span className="flex-1" />
      <Button variant="ghost" onClick={p.onOpenSettings} title="Model and settings">
        {p.providerLabel}
      </Button>
    </header>
  );
}
