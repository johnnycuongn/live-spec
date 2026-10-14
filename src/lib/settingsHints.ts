// Copy and rules for the Agents and Jev settings groups, kept pure so they're
// testable. The Rust side enforces the same limits (Settings::max_agents
// clamps to 1..=8; Jev never runs for the Local provider).
import type { KeySource, ProviderSettings } from "./ipc";

type Kind = ProviderSettings["kind"];

export const PARALLEL_NOTES_HINT = "Off by default: with Claude Code, four agents measured slower than one.";
export const PARALLEL_BRIEF_HINT = "Off by default: on a typical repo, parallel explorers plus the merge took about 16% longer.";

export const clampAgents = (n: number) => (Number.isFinite(n) ? Math.min(8, Math.max(1, Math.round(n))) : 1);

const AGENT_HINT: Record<Kind, string> = {
  claudeCli: "Each agent is a separate claude process on your subscription.",
  anthropicApi: "Each agent is a separate API call, billed for the shared project context.",
  local: "Local models run one agent at a time.",
};

export const agentHint = (kind: Kind) => AGENT_HINT[kind];

const SOURCE_LABEL: Record<KeySource, string> = {
  keychain: "Saved in your Keychain.",
  environment: "Using TYPESAFE_API_KEY from the environment.",
  envFile: "Using TYPESAFE_API_KEY from .env.local (development).",
};

export const keySourceLabel = (source: KeySource | null) => (source ? SOURCE_LABEL[source] : "No TypeSafe key found.");

export const jevUsable = (source: KeySource | null, kind: Kind) => source !== null && kind !== "local";

const SENDS = "Jev sends your draft, faults and short code excerpts to TypeSafe.";

export function jevHint(source: KeySource | null, kind: Kind): string {
  if (kind === "local") return "Jev never runs with the Local model: nothing leaves this computer.";
  return source ? SENDS : `Add a key to turn these on. ${SENDS}`;
}
