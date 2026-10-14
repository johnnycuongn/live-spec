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
  | { type: "done"; text: string }
  | { type: "error"; message: string }
  | { type: "progress"; downloaded: number; total: number | null }
  | { type: "checking"; count: number; total: number }
  | ({ type: "check" } & CheckResult)
  | { type: "notice"; message: string }
  | { type: "started" }
  | { type: "skipped" };

export type ProviderSettings =
  | { kind: "claudeCli"; binaryPath: string | null; fastModel: string; deepModel: string }
  | { kind: "anthropicApi"; fastModel: string; deepModel: string }
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
  checkClaude: (binaryPath: string | null) => invoke<string>("check_claude", { binaryPath }),
  warmUp: () => invoke<void>("warm_up"),

  openProject: (path: string) => invoke<ProjectSummary>("open_project", { path }),
  buildBrief: (onEvent: (e: UiEvent) => void) => stream("build_brief", {}, onEvent),

  analyze: (draft: string, previousDraft: string | null, onEvent: (e: UiEvent) => void) =>
    stream("analyze", { draft, previousDraft }, onEvent),
  cancelAnalysis: () => invoke<void>("cancel_analysis"),

  localStatus: () => invoke<LocalStatus>("local_inventory"),
  installRuntime: (onEvent: (e: UiEvent) => void) => stream("install_runtime", {}, onEvent),
  downloadModel: (modelId: string, onEvent: (e: UiEvent) => void) => stream("download_model", { modelId }, onEvent),
  cancelDownload: (key: string) => invoke<void>("cancel_download", { key }),
  stopLocal: () => invoke<void>("stop_local"),
};

export const errorMessage = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
