import type { BriefStatus, StoredBrief } from "../lib/ipc";
import { MiniMarkdown } from "./MiniMarkdown";
import { Button } from "./ui";

interface Props {
  brief: StoredBrief | null;
  status: BriefStatus | null;
  building: boolean;
  liveText: string;
  activity: string[];
  error: string | null;
  onBuild: () => void;
  onClose: () => void;
}

export function BriefDrawer(p: Props) {
  const when = p.brief ? new Date(p.brief.createdAt * 1000).toLocaleString() : null;
  return (
    <div
      className="absolute inset-y-0 right-0 flex w-[min(560px,100%)] flex-col border-l border-line bg-page shadow-[-12px_0_32px_rgb(0_0_0/0.08)]"
      role="dialog"
      aria-label="Project brief"
    >
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-5">
        <h2 className="flex-1 text-[15px] font-semibold">Project brief</h2>
        <Button onClick={p.onBuild} disabled={p.building}>
          {p.building ? "Building…" : p.brief ? "Rebuild brief" : "Build brief"}
        </Button>
        <Button variant="ghost" onClick={p.onClose} aria-label="Close brief">
          Close
        </Button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 pt-4 pb-10">
        {!p.building && (
          <p className="mb-4 text-xs text-muted">
            {p.brief
              ? `Written by ${p.brief.source} on ${when}.${p.status === "stale" ? " Files have changed since." : ""}`
              : "LiveBrainstorm reads the project once and writes this summary, then uses it every time it reads your draft."}
          </p>
        )}
        {p.error && (
          <p className="mb-4 whitespace-pre-wrap text-[13px] text-fault" role="alert">
            {p.error}
          </p>
        )}

        {p.building && (
          <>
            <ol className="mb-4 space-y-0.5 font-mono text-xs text-muted" aria-label="What LiveBrainstorm is reading">
              {p.activity.slice(-8).map((a, i) => (
                <li key={i} className="truncate last:text-ink">
                  {a}
                </li>
              ))}
            </ol>
            {p.liveText && <MiniMarkdown text={p.liveText} className="max-w-[70ch] text-sm" />}
          </>
        )}
        {!p.building && p.brief && <MiniMarkdown text={p.brief.text} className="max-w-[70ch] text-sm" />}
      </div>
    </div>
  );
}
