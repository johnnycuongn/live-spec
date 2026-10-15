import { describe, expect, it } from "vitest";
import {
  CATALOGUE,
  DEFAULT_SECTIONS,
  SECTIONS,
  effectiveSections,
  isDefaultSections,
  normalizeSections,
  parseSections,
  sectionsSentence,
  toggleSection,
} from "./sections";

describe("the catalogue", () => {
  it("mirrors sections.rs: ids, order and the default set", () => {
    expect(CATALOGUE.map((s) => s.key)).toEqual(["intent", "conflicts", "gaps", "ambiguities", "questions"]);
    expect(DEFAULT_SECTIONS).toEqual(CATALOGUE.map((s) => s.key));
    expect(SECTIONS.filter((s) => s.legacy).map((s) => s.key)).toEqual(["directions"]);
  });

  it("normalizes to catalogue order, known ids only, once each; empty stays empty", () => {
    expect(normalizeSections(["questions", "intent", "questions", "bogus"])).toEqual(["intent", "questions"]);
    expect(normalizeSections([])).toEqual([]);
    expect(normalizeSections(["directions"])).toEqual([]);
    expect(effectiveSections([])).toEqual(DEFAULT_SECTIONS);
    expect(effectiveSections(["gaps"])).toEqual(["gaps"]);
  });

  it("the default set is empty or the whole catalogue in order", () => {
    expect(isDefaultSections([])).toBe(true);
    expect(isDefaultSections([...DEFAULT_SECTIONS])).toBe(true);
    expect(isDefaultSections(["intent", "conflicts", "gaps", "ambiguities"])).toBe(false);
    expect(isDefaultSections(["questions", "intent", "conflicts", "gaps", "ambiguities"])).toBe(false);
  });

  it("toggles keep catalogue order and never empty a tab's list", () => {
    expect(toggleSection(["intent", "gaps"], "conflicts")).toEqual(["intent", "conflicts", "gaps"]);
    expect(toggleSection(["intent", "gaps"], "gaps")).toEqual(["intent"]);
    expect(toggleSection(["gaps"], "gaps")).toEqual(["gaps"]);
    expect(toggleSection(["gaps"], "gaps", true)).toEqual([]); // a template may leave the tab's sections alone
    expect(toggleSection([], "gaps")).toEqual(["intent", "conflicts", "ambiguities", "questions"]); // empty means the default set
  });

  it("writes the empty-state sentence from the chosen titles", () => {
    expect(sectionsSentence(["intent", "conflicts", "questions"])).toBe("Intent, Conflicts with code and Questions for you.");
    expect(sectionsSentence(["gaps"])).toBe("Gaps.");
    expect(sectionsSentence([])).toBe("Intent, Conflicts with code, Gaps, Ambiguities and Questions for you.");
  });
});

describe("parseSections", () => {
  it("splits canonical headings", () => {
    const md = "## Intent\n- add caching\n## Conflicts\n- `cache.ts` exists\n## Gaps\n- tests\n## Ambiguities\n- 'fast'\n## Questions\n- TTL?";
    expect(parseSections(md)).toEqual({
      intent: "- add caching",
      conflicts: "- `cache.ts` exists",
      gaps: "- tests",
      ambiguities: "- 'fast'",
      questions: "- TTL?",
    });
  });

  it("handles partial streams and loose formats", () => {
    expect(parseSections("## Intent\n- half")).toEqual({ intent: "- half" });
    expect(parseSections("**Conflicts:**\n- x\n### Questions for you\n- y")).toEqual({ conflicts: "- x", questions: "- y" });
    expect(parseSections("## Conflicts with code\n- x\n## Ambiguity\n- y")).toEqual({ conflicts: "- x", ambiguities: "- y" });
  });

  it("reads the old build's notes: Faults are conflicts, Directions stay as a legacy section", () => {
    expect(parseSections("## Faults\n- f\n## Directions\n- d")).toEqual({ conflicts: "- f", directions: "- d" });
  });

  it("ignores text before the first heading and bold words inside bullets", () => {
    expect(parseSections("Sure, here you go\n## Intent\n- a")).toEqual({ intent: "- a" });
    expect(parseSections("## Intent\n- a\n**Intentional** duplication\n- b")).toEqual({ intent: "- a\n**Intentional** duplication\n- b" });
  });
});
