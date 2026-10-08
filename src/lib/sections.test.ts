import { describe, expect, it } from "vitest";
import { parseSections } from "./sections";

describe("parseSections", () => {
  it("splits canonical headings", () => {
    const md = "## Intent\n- add caching\n## Faults\n- `cache.ts` exists\n## Directions\n- reuse it\n## Questions\n- TTL?";
    expect(parseSections(md)).toEqual({
      intent: "- add caching",
      faults: "- `cache.ts` exists",
      directions: "- reuse it",
      questions: "- TTL?",
    });
  });

  it("handles partial streams and loose formats", () => {
    expect(parseSections("## Intent\n- half")).toEqual({ intent: "- half" });
    expect(parseSections("**Faults:**\n- x\n### Questions for you\n- y")).toEqual({ faults: "- x", questions: "- y" });
  });

  it("ignores text before the first heading", () => {
    expect(parseSections("Sure, here you go\n## Intent\n- a")).toEqual({ intent: "- a" });
  });
});
