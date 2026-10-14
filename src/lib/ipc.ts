// Typed bridge to the Rust commands. Shapes mirror the serde types in
// src-tauri/src/commands.rs and crates/livebrainstorm-core; keep them in sync.
import { Channel, invoke } from "@tauri-apps/api/core";

export type Verdict = "confirmed" | "contradicted" | "unclear" | "notShown" | "notFound";

export interface CheckResult {
  claim: string;
  verdict: Verdict;
  confidence: number | null;
  path: string | null;
  line: number | null;
}

export type UiEvent =
  | { type: "text"; text: string; lane: number }
  | { type: "activity"; label: string; lane: number }
  | { type: "done"; text: string; partial?: boolean }
  | { type: "error"; message: string }
  | { type: "progress"; downloaded: number; total: number | null }
  | { type: "checking"; count: number; total: number }
  | ({ type: "check" } & CheckResult)
  | { type: "notice"; message: string }
  | { type: "checksStopped"; message: string }
  | { type: "started" }
  | { type: "skipped" };

/** Mirrors `Effort` in settings.rs; null means the model's default. */
export type Effort = "low" | "medium" | "high" | "xhigh" | "max";

/** Mirrors `ClaudeAuth`: Claude Code's own login, or the keychain API key. */
export type ClaudeAuth = "login" | "apiKey";

export type ProviderSettings =
  | {
      kind: "claudeCli";
      binaryPath: string | null;
      fastModel: string;
      deepModel: string;
      fastEffort: Effort | null;
      deepEffort: Effort | null;
      auth: ClaudeAuth;
    }
  | { kind: "anthropicApi"; fastModel: string; deepModel: string; fastEffort: Effort | null; deepEffort: Effort | null }
  | { kind: "local"; modelId: string };

export interface JevSettings {
  gate: boolean;
  rerank: boolean;
  check: boolean;
  model: string;
}

export interface AgentSettings {
  maxAgents: number;
  parallelBrief: boolean;
  parallelNotes: boolean;
}

export interface Settings {
  provider: ProviderSettings;
  debounceMs: number;
  jev: JevSettings;
  agents: AgentSettings;
}

export type KeySource = "keychain" | "environment" | "envFile";

/** Where the TypeSafe key comes from; the key itself never reaches the webview. */
export interface JevStatus {
  source: KeySource | null;
}

/** Mirrors `instructions::Template`. Starters have fixed ids; new ones use crypto.randomUUID(). */
export interface Template {
  id: string;
  name: string;
  text: string;
}

/** One project's instructions (`instructions::ProjectInstructions`). */
export interface ProjectInstructions {
  default: string;
  current: string;
  templateId: string | null;
}

export interface InstructionsView {
  templates: Template[];
  project: ProjectInstructions;
}

export type BriefStatus = "missing" | "fresh" | "stale";

export interface StoredBrief {
  root: string;
  fingerprint: string;
  text: string;
  createdAt: number;
  source: string;
}

export interface ProjectSummary {
  name: string;
  root: string;
  indexedFiles: number;
  totalFiles: number;
  briefStatus: BriefStatus;
  brief: StoredBrief | null;
}

export interface LocalModel {
  id: string;
  name: string;
  file: string;
  approxSizeGb: number;
  contextTokens: number;
  note: string;
  downloaded: boolean;
}

export interface LocalStatus {
  runtimeInstalled: boolean;
  runtimeBuild: string;
  platformSupported: boolean;
  models: LocalModel[];
  runningModel: string | null;
  downloading: string[];
}

/** Each streaming call gets its own Channel, so events never cross requests. */
function stream(cmd: string, args: Record<string, unknown>, onEvent: (e: UiEvent) => void): Promise<void> {
  const channel = new Channel<UiEvent>();
  channel.onmessage = onEvent;
  return invoke(cmd, { ...args, onEvent: channel });
}

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setApiKey: (key: string) => invoke<void>("set_api_key", { key }),
  hasApiKey: () => invoke<boolean>("has_api_key"),
  setJevKey: (key: string) => invoke<void>("set_jev_key", { key }),
  jevStatus: () => invoke<JevStatus>("jev_status"),
  checkClaude: (binaryPath: string | null) => invoke<string>("check_claude", { binaryPath }),
  warmUp: () => invoke<void>("warm_up"),

  openProject: (path: string) => invoke<ProjectSummary>("open_project", { path }),
  getInstructions: (root: string) => invoke<InstructionsView>("get_instructions", { root }),
  saveProjectInstructions: (root: string, instructions: ProjectInstructions) =>
    invoke<void>("save_project_instructions", { root, instructions }),
  saveTemplates: (templates: Template[]) => invoke<void>("save_templates", { templates }),
  restoreStarterTemplates: () => invoke<Template[]>("restore_starter_templates"),
  buildBrief: (onEvent: (e: UiEvent) => void) => stream("build_brief", {}, onEvent),

  analyze: (draft: string, previousDraft: string | null, instructions: string, onEvent: (e: UiEvent) => void) =>
    stream("analyze", { draft, previousDraft, instructions }, onEvent),
  cancelAnalysis: () => invoke<void>("cancel_analysis"),

  localStatus: () => invoke<LocalStatus>("local_inventory"),
  installRuntime: (onEvent: (e: UiEvent) => void) => stream("install_runtime", {}, onEvent),
  downloadModel: (modelId: string, onEvent: (e: UiEvent) => void) => stream("download_model", { modelId }, onEvent),
  cancelDownload: (key: string) => invoke<void>("cancel_download", { key }),
  stopLocal: () => invoke<void>("stop_local"),
};

export const errorMessage = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
