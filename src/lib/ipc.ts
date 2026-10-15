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
  /** Catalogue ids; empty leaves the tab's sections alone. */
  sections: string[];
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

/** One saved analysis (`sessions::Snapshot`). `at` is unix seconds; `context` is the run's `analysisContextKey`. */
export interface Snapshot {
  id: number;
  at: number;
  draft: string;
  context: string;
  markdown: string;
  partial: boolean;
  notices: string[];
  checks: CheckResult[];
  /** The claim count from the last `checking` event. Fewer checks, without `checksStopped`, means an edit cut the run off. */
  checksTotal: number | null;
  /** The Conflicts count from the same event (`total`); null in files saved before it existed. The field keeps its old name. */
  checksFaults: number | null;
  checksStopped: boolean;
}

/** One tab (`sessions::Tab`): its draft and its last analyses, newest first (at most 5). */
export interface Tab {
  id: string;
  root: string | null;
  title: string | null;
  createdAt: number;
  updatedAt: number;
  closedAt: number | null;
  draft: string;
  /** Catalogue ids in catalogue order; empty means the default set. */
  sections: string[];
  nextSnapshot: number;
  history: Snapshot[];
}

/** What lists show (`sessions::TabSummary`): no draft body or history. */
export interface TabSummary {
  id: string;
  root: string | null;
  title: string | null;
  /** The draft's first non-empty line, at most 48 characters (`firstLine` in tabTitle.ts). */
  firstLine: string;
  updatedAt: number;
  closedAt: number | null;
}

/** Mirrors `sessions::WindowSummary`: one window, named in menus by its active tab. */
export interface WindowSummary {
  id: string;
  active: TabSummary | null;
  tabs: number;
}

/** One window's tabs, its full active tab, and the shared closed list (`sessions::WindowView`). */
export interface WindowView {
  window: string;
  tabs: TabSummary[];
  active: Tab | null;
  closed: TabSummary[];
  /** The first launch of this build: import the old localStorage drafts. */
  fresh: boolean;
  /** Files kept as `.bad` since the app started. */
  unreadable: string[];
}

/** A draft the old build kept in localStorage (`sessions::LegacyDraft`). */
export interface LegacyDraft {
  root: string | null;
  draft: string;
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

/** What a model call was for (`usage::UsageKind`). */
export type UsageKind = "notes" | "brief";

/** One hour of one project's usage for one kind and model (`usage::UsageBucket`). `hour` is unix seconds / 3600, UTC. */
export interface UsageBucket {
  hour: number;
  root: string | null;
  kind: UsageKind;
  model: string;
  input: number;
  cacheWrite: number;
  cacheRead: number;
  output: number;
  costUsd: number;
  runs: number;
  estimatedRuns: number;
  estimatedCostUsd: number;
  unpricedRuns: number;
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
  projectSummary: (root: string) => invoke<ProjectSummary>("project_summary", { root }),
  getInstructions: (root: string) => invoke<InstructionsView>("get_instructions", { root }),
  saveProjectInstructions: (root: string, instructions: ProjectInstructions) =>
    invoke<void>("save_project_instructions", { root, instructions }),
  saveTemplates: (templates: Template[]) => invoke<void>("save_templates", { templates }),
  restoreStarterTemplates: () => invoke<Template[]>("restore_starter_templates"),
  buildBrief: (root: string, onEvent: (e: UiEvent) => void) => stream("build_brief", { root }, onEvent),

  analyze: (
    tabId: string,
    draft: string,
    previousDraft: string | null,
    instructions: string,
    sections: string[],
    context: string,
    onEvent: (e: UiEvent) => void,
  ) => stream("analyze", { tabId, draft, previousDraft, instructions, sections, context }, onEvent),
  cancelAnalysis: (tabId: string) => invoke<void>("cancel_analysis", { tabId }),
  usageReport: (sinceHour: number) => invoke<UsageBucket[]>("usage_report", { sinceHour }),

  listWorkspace: (window: string) => invoke<WindowView>("list_workspace", { window }),
  getTab: (tabId: string) => invoke<Tab>("get_tab", { tabId }),
  createTab: (window: string, root: string | null, sections: string[]) => invoke<Tab>("create_tab", { window, root, sections }),
  setTabSections: (tabId: string, sections: string[]) => invoke<void>("set_tab_sections", { tabId, sections }),
  saveDraft: (tabId: string, draft: string) => invoke<void>("save_draft", { tabId, draft }),
  renameTab: (tabId: string, title: string | null) => invoke<void>("rename_tab", { tabId, title }),
  activateTab: (window: string, tabId: string) => invoke<void>("activate_tab", { window, tabId }),
  closeTab: (tabId: string) => invoke<void>("close_tab", { tabId }),
  reopenTab: (tabId: string, window: string) => invoke<void>("reopen_tab", { tabId, window }),
  deleteTab: (tabId: string) => invoke<void>("delete_tab", { tabId }),
  openWindow: () => invoke<string>("open_window"),
  moveTab: (tabId: string, to: string | null) => invoke<string>("move_tab", { tabId, to }),
  listWindows: () => invoke<WindowSummary[]>("list_windows"),
  importLegacyDrafts: (window: string, drafts: LegacyDraft[], activeRoot: string | null) =>
    invoke<void>("import_legacy_drafts", { window, drafts, activeRoot }),

  localStatus: () => invoke<LocalStatus>("local_inventory"),
  installRuntime: (onEvent: (e: UiEvent) => void) => stream("install_runtime", {}, onEvent),
  downloadModel: (modelId: string, onEvent: (e: UiEvent) => void) => stream("download_model", { modelId }, onEvent),
  cancelDownload: (key: string) => invoke<void>("cancel_download", { key }),
  stopLocal: () => invoke<void>("stop_local"),
};

export const errorMessage = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
