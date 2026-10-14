import { describe, expect, it } from "vitest";
import type { ProviderSettings, Settings } from "./ipc";
import { CLAUDE_ALIASES, EFFORT_OPTIONS, checkModels, isCustomModel, providerLabel } from "./providerOptions";

const cli: Extract<ProviderSettings, { kind: "claudeCli" }> = {
  kind: "claudeCli",
  binaryPath: null,
  fastModel: "haiku",
  deepModel: "sonnet",
  fastEffort: null,
  deepEffort: null,
  auth: "login",
};

const settings = (provider: ProviderSettings): Settings => ({
  provider,
  debounceMs: 1200,
  jev: { gate: true, rerank: true, check: true, model: "jev-latest" },
  agents: { maxAgents: 4, parallelBrief: false, parallelNotes: false },
});

describe("models", () => {
  it("offers fable and treats anything else as custom", () => {
    expect(CLAUDE_ALIASES).toEqual(["haiku", "sonnet", "opus", "fable"]);
    expect(isCustomModel("fable")).toBe(false);
    expect(isCustomModel("claude-opus-5-5")).toBe(true);
  });

  it("trims model names before saving", () => {
    const { provider, error } = checkModels({ ...cli, deepModel: "  claude-opus-5-5 " });
    expect(error).toBeNull();
    expect(provider).toEqual({ ...cli, deepModel: "claude-opus-5-5" });
  });

  it("names the tier whose model is blank", () => {
    expect(checkModels({ ...cli, fastModel: "  " }).error).toBe("Enter a model name for While you type");
    expect(checkModels({ kind: "anthropicApi", fastModel: "a", deepModel: "", fastEffort: null, deepEffort: null }).error).toBe(
      "Enter a model name for Project brief",
    );
    expect(checkModels({ kind: "local", modelId: "qwen3-4b-instruct" }).error).toBeNull();
  });
});

describe("effort", () => {
  it("lists Default first, then the CLI's levels in order", () => {
    expect(EFFORT_OPTIONS.map((o) => o.value)).toEqual([null, "low", "medium", "high", "xhigh", "max"]);
    expect(EFFORT_OPTIONS.map((o) => o.label)).toEqual(["Default", "Low", "Medium", "High", "Extra high", "Max"]);
  });
});

describe("providerLabel", () => {
  it("says when Claude Code uses an API key", () => {
    expect(providerLabel(settings(cli))).toBe("Claude Code, haiku");
    expect(providerLabel(settings({ ...cli, auth: "apiKey" }))).toBe("Claude Code (API key), haiku");
    expect(providerLabel(settings({ kind: "local", modelId: "qwen3-4b-instruct" }))).toBe("Local, qwen3-4b-instruct");
    expect(providerLabel(null)).toBe("Settings");
  });
});
