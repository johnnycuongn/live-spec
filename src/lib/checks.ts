// Display rules for "Checked in code". Verdicts are signals, not proof.
import type { CheckResult, Verdict } from "./ipc";

export const VERDICT: Record<Verdict, { mark: string; label: string; tone: "accent" | "fault" | "faint" }> = {
  confirmed: { mark: "✓", label: "Confirmed", tone: "accent" },
  contradicted: { mark: "✗", label: "Contradicted", tone: "fault" },
  unclear: { mark: "?", label: "Unclear", tone: "faint" },
  notShown: { mark: "–", label: "Not shown in this code", tone: "faint" },
  notFound: { mark: "–", label: "Not found in indexed code", tone: "faint" },
};

/** `done` verdicts so far; `stopped` once Jev failed and the rest were dropped. */
export function checkSummary(checking: { count: number; total: number }, done: number, stopped: boolean): string {
  const conflicts = checking.total === 1 ? "conflict" : "conflicts";
  if (!stopped && done < checking.count) return `Checking ${checking.count} of ${checking.total} ${conflicts}…`;
  return `${done} of ${checking.total} ${conflicts} checked`;
}

export const checkLocation = (r: CheckResult) => (r.path ? `${r.path}${r.line ? `:${r.line}` : ""}` : null);

/** One-line display text: inline `code` and **bold** markers removed. */
export const plainClaim = (claim: string) => claim.replace(/\*\*|`/g, "");
