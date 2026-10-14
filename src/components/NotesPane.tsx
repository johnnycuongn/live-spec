import type { CheckResult } from "../lib/ipc";
import { VERDICT, checkLocation, checkSummary, plainClaim } from "../lib/checks";
import { SECTIONS, parseSections, type SectionKey } from "../lib/sections";
import type { AnalysisState } from "../lib/analysisState";
import { MiniMarkdown } from "./MiniMarkdown";
import { Button, Eyebrow, RULE, cx, type RuleState } from "./ui";

/** The shown tab's analysis, whether the draft has moved on since, and "Try again". */
type Analysis = AnalysisState & { stale: boolean; rerun: () => void };

interface Props {
  analysis: Analysis;
  hasProject: boolean;
  briefMissing: boolean;
  onBuildBrief: () => void;
}

const DOT: Record<SectionKey, string> = {
  intent: "bg-muted",
  faults: "bg-fault",
  directions: "bg-accent",
  questions: "bg-question",
};

function statusText(a: Analysis) {
  if (a.status === "thinking") return "Thinking";
  if (a.status === "error") return "Couldn't think this through";
  if (a.stale) return "Your draft moved on. Pause to refresh.";
  if (a.status === "done") return "Up to date with your draft";
  return "Reading along";
}

/** The accent rule between the panes: solid when current, animated while thinking. */
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

export function NotesPane({ analysis: a, hasProject, briefMissing, onBuildBrief }: Props) {
  const sections = parseSections(a.markdown);
  const rule: RuleState = a.status === "thinking" ? "thinking" : a.stale ? "stale" : a.status === "done" ? "fresh" : "idle";

  return (
    <aside className="relative flex min-h-0 flex-col bg-panel" aria-label="Notes">
      <Rule state={rule} />
      <div className="min-h-0 flex-1 overflow-y-auto px-7 pt-7 pb-10">
        <p className="mb-5 text-xs text-faint" aria-live="polite">
          {statusText(a)}
        </p>
        {a.notices.map((n, i) => (
          <p key={i} className="mb-2 text-xs text-muted">
            {n}
          </p>
        ))}
        {a.partial && a.status === "done" && (
          <p className="mb-4 text-xs text-muted">
            Some sections are missing.{" "}
            <Button variant="link" onClick={a.rerun}>
              Try again
            </Button>
          </p>
        )}

        {!hasProject &&<p className="mb-4 text-[13px] text-muted">No project open, so notes won't know your code.</p>}
        {hasProject && briefMissing && (
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
            <Button onClick={a.rerun}>Try again</Button>
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
          <div className={cx("grid gap-6 transition-opacity motion-reduce:transition-none", a.stale && "opacity-60")}>
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
            {a.status === "thinking" && !a.markdown && (
              <p className="text-[13px] text-muted">Reading your draft against the project…</p>
            )}
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
