import { useState } from "react";
import type { CheckResult } from "../lib/ipc";
import { VERDICT, checkLocation, checkSummary, plainClaim } from "../lib/checks";
import { historyPosition, snapshotAnalysis, snapshotTime } from "../lib/history";
import { SECTIONS, parseSections, type SectionKey } from "../lib/sections";
import type { TabView } from "../lib/tabSession";
import { ruleState } from "../lib/tabStatus";
import { MiniMarkdown } from "./MiniMarkdown";
import { Button, Eyebrow, RULE, cx, type RuleState } from "./ui";

interface Props {
  view: TabView;
  hasProject: boolean;
  briefMissing: boolean;
  onBuildBrief: () => void;
  onRerun: () => void;
  /** Shows analysis `index` of the history (0 = latest). */
  onView: (index: number) => void;
}

const DOT: Record<SectionKey, string> = {
  intent: "bg-muted",
  faults: "bg-fault",
  directions: "bg-accent",
  questions: "bg-question",
};

function statusText(v: TabView) {
  const a = v.analysis;
  if (a.status === "thinking") return "Thinking";
  if (a.status === "error") return "Couldn't think this through";
  if (v.stale) return "Your draft moved on. Pause to refresh.";
  if (a.status === "done") return "Up to date with your draft";
  return "Reading along";
}

/** The accent rule between the panes: solid when current, animated while thinking. Always the live state. */
function Rule({ state }: { state: RuleState }) {
  return (
    <div className="absolute inset-y-0 left-0 w-0.5 bg-line" aria-hidden>
      <div className={cx("absolute inset-0 origin-top", RULE[state])} />
    </div>
  );
}

const TONE = { accent: "text-accent", fault: "text-fault", faint: "text-faint" } as const;

function CheckRow({ check }: { check: CheckResult }) {
  const v = VERDICT[check.verdict];
  const where = checkLocation(check);
  return (
    <li className="grid grid-cols-[1rem_minmax(0,1fr)] gap-x-2 text-[13px] leading-snug">
      <span className={cx("font-semibold", TONE[v.tone])} aria-hidden>
        {v.mark}
      </span>
      <span className="min-w-0">
        <span className="block truncate" title={plainClaim(check.claim)}>
          {plainClaim(check.claim)}
        </span>
        <span className="text-xs text-muted">
          {v.label}
          {where && (
            <>
              {" · "}
              <code className="font-mono text-[0.85em]">{where}</code>
            </>
          )}
        </span>
      </span>
    </li>
  );
}

/**
 * `‹ 2 of 5 ›` and the time of the analysis on screen. Hidden with fewer than
 * two, and over the live notes while a run is under way (history[0] isn't
 * them yet). Over an older analysis it stays, so a run that starts while you
 * browse never strands you there.
 */
function HistoryHeader({ view, onView }: { view: TabView; onView: (index: number) => void }) {
  const pos = historyPosition(view.viewing, view.history.length);
  if (!pos.show || (view.viewing === 0 && view.analysis.status === "thinking")) return null;
  // The latest is the live state: it has history[0]'s time once that analysis is what's on screen.
  const shown = view.viewing > 0 ? view.history[view.viewing] : view.analysis.status === "done" ? view.history[0] : undefined;
  return (
    <nav className="mb-4 flex items-center gap-1 text-xs tabular-nums text-faint" aria-label="Earlier analyses">
      <Button variant="mini" aria-label="Earlier analysis" disabled={pos.earlier === null} onClick={() => pos.earlier !== null && onView(pos.earlier)}>
        ‹
      </Button>
      <span aria-live="polite">{pos.label}</span>
      <Button variant="mini" aria-label="Later analysis" disabled={pos.later === null} onClick={() => pos.later !== null && onView(pos.later)}>
        ›
      </Button>
      {shown && <span className="ml-1">{snapshotTime(shown.at)}</span>}
    </nav>
  );
}

/** Over an older analysis: the banner, and the draft it read, collapsed by default. */
function EarlierDraft({ draft }: { draft: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mb-5 grid gap-2 text-xs text-muted">
      <p>
        Notes on an earlier draft.{" "}
        <Button variant="link" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "Hide that draft" : "Show that draft"}
        </Button>
      </p>
      {open && <p className="whitespace-pre-wrap border-l-2 border-line pl-3 font-serif text-[15px] leading-relaxed text-muted">{draft}</p>}
    </div>
  );
}

export function NotesPane({ view, hasProject, briefMissing, onBuildBrief, onRerun, onView }: Props) {
  const older = view.viewing > 0 ? view.history[view.viewing] : undefined;
  // An older analysis is read-only: its notes, checks and notices, never a retry.
  const a = older ? snapshotAnalysis(older) : view.analysis;
  const stale = older ? false : view.stale;
  const sections = parseSections(a.markdown);

  return (
    <aside className="relative flex min-h-0 flex-col bg-panel" aria-label="Notes">
      <Rule state={ruleState(view)} />
      <div className="min-h-0 flex-1 overflow-y-auto px-7 pt-7 pb-10">
        <HistoryHeader view={view} onView={onView} />
        {older ? (
          <EarlierDraft key={older.id} draft={older.draft} />
        ) : (
          <p className="mb-5 text-xs text-faint" aria-live="polite">
            {statusText(view)}
          </p>
        )}
        {a.notices.map((n, i) => (
          <p key={i} className="mb-2 text-xs text-muted">
            {n}
          </p>
        ))}
        {a.partial && a.status === "done" && (
          <p className="mb-4 text-xs text-muted">
            Some sections are missing.
            {!older && (
              <>
                {" "}
                <Button variant="link" onClick={onRerun}>
                  Try again
                </Button>
              </>
            )}
          </p>
        )}

        {!older && !hasProject && <p className="mb-4 text-[13px] text-muted">This tab has no project, so notes won't know your code.</p>}
        {!older && hasProject && briefMissing && (
          <p className="mb-4 text-[13px] text-muted">
            Notes are sharper with a project brief.{" "}
            <Button variant="link" onClick={onBuildBrief}>
              Build it now
            </Button>
          </p>
        )}

        {a.status === "error" && (
          <div className="grid justify-items-start gap-2 text-[13px] text-fault" role="alert">
            <p className="whitespace-pre-wrap">{a.error}</p>
            <Button onClick={onRerun}>Try again</Button>
          </div>
        )}

        {a.status === "idle" && (
          <div className="max-w-[46ch] space-y-2.5 text-[13px] leading-relaxed text-muted">
            <p>Start writing on the left. When you pause, notes appear here:</p>
            <p>
              what you seem to be after, faults against how the project actually works, directions worth trying, and the
              questions that matter most.
            </p>
          </div>
        )}

        {a.status !== "idle" && a.status !== "error" && (
          <div className={cx("grid gap-6 transition-opacity motion-reduce:transition-none", stale && "opacity-60")}>
            {SECTIONS.map(({ key, title }) =>
              sections[key] !== undefined ? (
                <section key={key} className="grid gap-1.5">
                  <h2>
                    <Eyebrow dot={DOT[key]}>{title}</Eyebrow>
                  </h2>
                  <MiniMarkdown text={sections[key]!} />
                </section>
              ) : null,
            )}
            {a.status === "thinking" && !a.markdown && <p className="text-[13px] text-muted">Reading your draft against the project…</p>}
            {a.checking && (
              <section className="grid gap-1.5">
                <h2>
                  <Eyebrow dot="bg-ink">Checked in code</Eyebrow>
                </h2>
                <p className="text-xs text-faint">{checkSummary(a.checking, a.checks.length, a.checksStopped)}</p>
                <ul className="grid gap-2">
                  {a.checks.map((c, i) => (
                    <CheckRow key={i} check={c} />
                  ))}
                </ul>
              </section>
            )}
          </div>
        )}
      </div>
    </aside>
  );
}
