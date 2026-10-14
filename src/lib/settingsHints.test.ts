import { describe, expect, it } from "vitest";
import { agentHint, clampAgents, clampedAgents, jevHint, jevUsable, keySourceLabel, PARALLEL_BRIEF_HINT, PARALLEL_NOTES_HINT } from "./settingsHints";

describe("agents", () => {
  it("clampAgents keeps 1..8", () => {
    expect([0, 1, 4, 8, 12, -3, 2.6, Number.NaN].map(clampAgents)).toEqual([1, 1, 4, 8, 8, 1, 3, 1]);
  });

  it("clampedAgents fixes a stored maxAgents before it is saved", () => {
    const agents = { maxAgents: 12, parallelBrief: true, parallelNotes: false };
    expect(clampedAgents(agents)).toEqual({ maxAgents: 8, parallelBrief: true, parallelNotes: false });
    expect(clampedAgents({ ...agents, maxAgents: 0 }).maxAgents).toBe(1);
    expect(clampedAgents({ ...agents, maxAgents: Number.NaN }).maxAgents).toBe(1);
  });

  it("explains what an agent costs per provider", () => {
    expect(agentHint("claudeCli")).toBe("Each agent is a separate claude process on your subscription.");
    expect(agentHint("anthropicApi")).toBe("Each agent is a separate API call, billed for the shared project context.");
    expect(agentHint("local")).toBe("Local models run one agent at a time.");
  });

  it("says plainly that parallel was measured slower", () => {
    expect(PARALLEL_NOTES_HINT).toBe("Off by default: with Claude Code, four agents measured slower than one.");
    expect(PARALLEL_BRIEF_HINT).toBe("Off by default: on a typical repo, parallel explorers plus the merge took about 16% longer.");
  });
});

describe("jev", () => {
  it("names where the key comes from", () => {
    expect(keySourceLabel("keychain")).toBe("Saved in your Keychain.");
    expect(keySourceLabel("environment")).toBe("Using TYPESAFE_API_KEY from the environment.");
    expect(keySourceLabel("envFile")).toBe("Using TYPESAFE_API_KEY from .env.local (development).");
    expect(keySourceLabel(null)).toBe("No TypeSafe key found.");
  });

  it("local disables agents and jev with a reason", () => {
    expect(jevUsable("keychain", "local")).toBe(false);
    expect(jevHint("keychain", "local")).toBe("Jev never runs with the Local model: nothing leaves this computer.");
  });

  it("needs a key before the switches mean anything", () => {
    expect(jevUsable(null, "claudeCli")).toBe(false);
    expect(jevHint(null, "claudeCli")).toBe("Add a key to turn these on. Jev sends your draft, faults and short code excerpts to TypeSafe.");
    expect(jevUsable("envFile", "anthropicApi")).toBe(true);
    expect(jevHint("envFile", "anthropicApi")).toBe("Jev sends your draft, faults and short code excerpts to TypeSafe.");
  });
});
