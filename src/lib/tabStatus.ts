// What a tab shows about its analysis: the rule's state and a status line.
// Shared by the rule between the panes (NotesPane) and each sidebar row.
import type { RuleState } from "../components/ui";
import type { TabView } from "./tabSession";
import { firstLine } from "./tabTitle";

/** The parts of a tab's state the indicator reads. */
export type TabLike = Pick<TabView, "analysis" | "stale">;

/** `null` is a tab with no session yet (not shown since launch): faint, like idle. */
export function ruleState(v: TabLike | null): RuleState {
  if (!v) return "idle";
  if (v.analysis.status === "thinking") return "thinking";
  if (v.stale) return "stale";
  return v.analysis.status === "done" ? "fresh" : "idle";
}

/** The line under a tab's title. The text carries the state when motion is reduced. */
export function statusLine(v: TabLike | null): { text: string; fault: boolean } | null {
  if (!v) return null;
  const a = v.analysis;
  if (a.status === "thinking") return { text: "Analysing…", fault: false };
  if (a.status === "error") return { text: firstLine(a.error ?? "") || "Couldn't think this through", fault: true };
  if (v.stale) return { text: "Draft moved on", fault: false };
  if (a.status === "done") return { text: "Notes current", fault: false };
  return null;
}

/** A collapsed project group pulses when any of its tabs is analysing. */
export const anyThinking = (views: (TabLike | null | undefined)[]) => views.some((v) => v?.analysis.status === "thinking");
