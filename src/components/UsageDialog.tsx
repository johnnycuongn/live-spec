import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { api, errorMessage, type UsageBucket } from "../lib/ipc";
import { onUsageChanged } from "../lib/events";
import {
  DAYS_SHOWN,
  formatDay,
  groupByLocalDay,
  lineLabel,
  rowCells,
  sinceHourFor,
  sumTotals,
  usageNotes,
  type UsageTotals,
} from "../lib/usage";
import { Button, cx } from "./ui";

export const SUBSCRIPTION_NOTE =
  "On a Claude subscription you aren't billed per token. This is what the same usage would cost on the API.";

/** The last 30 local days: a total, then one row per day, today first. A day
 *  with runs expands into its projects and kinds; a quiet day shows "—". A
 *  native modal, like Settings. */
export function UsageDialog({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [view, setView] = useState<{ buckets: UsageBucket[]; at: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  // Native modal: focus trap, Esc (fires `close`) and ::backdrop for free.
  // StrictMode runs this twice, and older WebKit throws on a second showModal().
  useEffect(() => {
    if (dialog.current && !dialog.current.open) dialog.current.showModal();
  }, []);

  useEffect(() => {
    let alive = true;
    const load = () => {
      const at = Date.now();
      api.usageReport(sinceHourFor(at, DAYS_SHOWN)).then(
        (buckets) => {
          if (!alive) return;
          setView({ buckets, at });
          setError(null);
        },
        (e) => alive && setError(errorMessage(e)),
      );
    };
    load();
    const unlisten = onUsageChanged(load);
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);

  const days = view ? groupByLocalDay(view.buckets, view.at) : [];
  const total = sumTotals(days);

  return (
    <dialog
      ref={dialog}
      onClose={onClose}
      // A press on the backdrop targets the <dialog> itself; the content div covers the rest.
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      aria-labelledby="usage-title"
      className="m-auto max-h-[calc(100%-2.5rem)] w-[min(620px,calc(100%-2.5rem))] overflow-y-auto rounded-xl border border-line bg-page text-ink shadow-2xl backdrop:bg-black/40"
    >
      <div className="grid gap-5 p-6">
        <header className="flex items-center justify-between">
          <h2 id="usage-title" className="text-lg font-semibold tracking-tight">
            Usage
          </h2>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </header>

        {error && (
          <p className="text-[13px] text-fault" role="alert">
            {error}
          </p>
        )}

        {view && total.runs === 0 && <p className="text-[13px] text-muted">No usage in the last 30 days.</p>}

        {view && (
          <table className="w-full text-[13px] tabular-nums">
            <thead>
              <tr className="border-b border-line text-left text-xs text-muted">
                <th className="py-1.5 font-normal">Day</th>
                <th className="py-1.5 text-right font-normal">Tokens</th>
                <th className="py-1.5 text-right font-normal">Est. cost</th>
                <th className="py-1.5 text-right font-normal">Runs</th>
              </tr>
            </thead>
            <tbody>
              <Row label="Last 30 days" totals={total} strong />
              {days.map((d) => {
                const open = expanded === d.day;
                return (
                  <Fragment key={d.day}>
                    <Row
                      label={
                        d.lines.length === 0 ? (
                          <span className="flex items-center gap-1.5 text-muted">
                            <span aria-hidden className="w-3" />
                            {formatDay(d.day)}
                          </span>
                        ) : (
                          <button
                            type="button"
                            aria-expanded={open}
                            onClick={() => setExpanded(open ? null : d.day)}
                            className="flex cursor-pointer items-center gap-1.5 text-left hover:text-accent"
                          >
                            <span aria-hidden className="w-3 text-faint">
                              {open ? "▾" : "▸"}
                            </span>
                            {formatDay(d.day)}
                          </button>
                        )
                      }
                      totals={d}
                    />
                    {open &&
                      d.lines.map((l) => (
                        <Row key={`${l.root ?? ""}|${l.kind}`} label={<span className="pl-[18px] text-muted">{lineLabel(l)}</span>} totals={l} />
                      ))}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}

        <p className="text-xs leading-relaxed text-muted">{SUBSCRIPTION_NOTE}</p>
      </div>
    </dialog>
  );
}

/** One table row; estimates and unpriced runs are labelled under the label. */
function Row({ label, totals, strong }: { label: ReactNode; totals: UsageTotals; strong?: boolean }) {
  const notes = usageNotes(totals);
  const cells = rowCells(totals);
  return (
    <tr className={cx("border-b border-line align-top", strong && "font-semibold")}>
      <td className="py-1.5 pr-3">
        {label}
        {notes.length > 0 && <div className="pl-[18px] text-xs font-normal text-muted">{notes.join(" · ")}</div>}
      </td>
      <td className="py-1.5 text-right">{cells.tokens}</td>
      <td className="py-1.5 text-right">{cells.cost}</td>
      <td className="py-1.5 text-right">{cells.runs}</td>
    </tr>
  );
}
