// Model, effort and label rules for the provider settings, kept pure so they're
// testable. Mirrors `Effort` / `ClaudeAuth` in crates/livebrainstorm-core/src/settings.rs.
import type { Effort, ProviderSettings, Settings } from "./ipc";

/** Claude Code accepts these aliases, which track the latest models. */
export const CLAUDE_ALIASES = ["haiku", "sonnet", "opus", "fable"];

export const EFFORT_OPTIONS: { value: Effort | null; label: string }[] = [
  { value: null, label: "Default" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "Extra high" },
  { value: "max", label: "Max" },
];

/** A saved model that isn't an alias opens the picker on "Custom…". */
export const isCustomModel = (model: string) => !CLAUDE_ALIASES.includes(model);

const TIERS = [
  ["fastModel", "While you type"],
  ["deepModel", "Project brief"],
] as const;

/** Trims model names; a blank one blocks saving with a message naming its tier. */
export function checkModels(p: ProviderSettings): { provider: ProviderSettings; error: string | null } {
  if (p.kind === "local") return { provider: p, error: null };
  const provider = { ...p, fastModel: p.fastModel.trim(), deepModel: p.deepModel.trim() };
  const blank = TIERS.find(([field]) => !provider[field]);
  return { provider, error: blank ? `Enter a model name for ${blank[1]}` : null };
}

export function providerLabel(s: Settings | null): string {
  if (!s) return "Settings";
  const p = s.provider;
  if (p.kind === "claudeCli") return `Claude Code${p.auth === "apiKey" ? " (API key)" : ""}, ${p.fastModel}`;
  if (p.kind === "anthropicApi") return `API, ${p.fastModel}`;
  return `Local, ${p.modelId}`;
}
