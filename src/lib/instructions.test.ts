import { describe, expect, it } from "vitest";
import type { ProjectInstructions, Template } from "./ipc";
import {
  EMPTY_INSTRUCTIONS,
  analysisContextKey,
  applyTemplate,
  charCount,
  clearInstructions,
  counterText,
  firstLine,
  instructionsLabel,
  removeTemplate,
  resetToDefault,
  saveAsDefault,
  upsertTemplate,
} from "./instructions";

const t = (id: string, name: string, text: string): Template => ({ id, name, text });
const bug = t("bug-hunt", "Bug hunt", "Treat my draft as a hypothesis.");
const feat = t("feature-design", "Feature design", "I'm designing a new feature.");
const p = (fields: Partial<ProjectInstructions>): ProjectInstructions => ({ ...EMPTY_INSTRUCTIONS, ...fields });

describe("instructionsLabel", () => {
  it("covers every state in the spec's table", () => {
    expect(instructionsLabel(p({}), [bug])).toBe("No instructions");
    expect(instructionsLabel(p({ current: "  \n", templateId: "bug-hunt" }), [bug])).toBe("No instructions");
    expect(instructionsLabel(p({ current: bug.text, templateId: "bug-hunt" }), [bug])).toBe("Bug hunt");
    expect(instructionsLabel(p({ current: `${bug.text} Also logs.`, templateId: "bug-hunt" }), [bug])).toBe("Bug hunt (edited)");
    expect(instructionsLabel(p({ current: bug.text, templateId: "bug-hunt" }), [feat])).toBe("Custom");
    expect(instructionsLabel(p({ current: "d", default: "d" }), [bug])).toBe("Project default");
    expect(instructionsLabel(p({ current: "mine", default: "d" }), [bug])).toBe("Custom");
  });

  it("a template edited after it was applied reads as edited", () => {
    expect(instructionsLabel(p({ current: bug.text, templateId: "bug-hunt" }), [{ ...bug, text: "Different now." }])).toBe(
      "Bug hunt (edited)",
    );
  });
});

describe("analysisContextKey", () => {
  it("changes with the instructions, ignoring surrounding whitespace", () => {
    const key = analysisContextKey("/p", 1, "Focus on tests.");
    expect(analysisContextKey("/p", 1, "  Focus on tests.\n")).toBe(key);
    expect(analysisContextKey("/p", 1, "Focus on docs.")).not.toBe(key);
    expect(analysisContextKey("/p", 2, "Focus on tests.")).not.toBe(key);
    expect(analysisContextKey("/q", 1, "Focus on tests.")).not.toBe(key);
    expect(analysisContextKey(undefined, undefined, "")).toBe("|0|");
  });
});

describe("actions", () => {
  it("apply replaces the text and keeps what was there for undo", () => {
    const before = p({ current: "mine", default: "d" });
    const { next, undo } = applyTemplate(before, bug);
    expect(next).toEqual({ default: "d", current: bug.text, templateId: "bug-hunt" });
    expect(undo).toBe(before);
  });

  it("clear, reset to default, save as default", () => {
    const s = p({ default: "d", current: bug.text, templateId: "bug-hunt" });
    expect(clearInstructions(s)).toEqual({ default: "d", current: "", templateId: null });
    expect(resetToDefault(s)).toEqual({ default: "d", current: "d", templateId: null });
    expect(saveAsDefault(s)).toEqual({ default: bug.text, current: bug.text, templateId: "bug-hunt" });
  });

  it("templates: edit in place, add at the end, remove by id", () => {
    expect(upsertTemplate([feat, bug], { ...bug, name: "Bugs" }).map((x) => x.name)).toEqual(["Feature design", "Bugs"]);
    expect(upsertTemplate([feat], bug).map((x) => x.id)).toEqual(["feature-design", "bug-hunt"]);
    expect(removeTemplate([feat, bug], "feature-design")).toEqual([bug]);
  });

  it("counts characters the way the backend clips them, not UTF-16 units", () => {
    expect(charCount("héllo")).toBe(5);
    expect(charCount("😀".repeat(2500))).toBe(2500);
    expect("😀".repeat(2500).length).toBe(5000);
  });

  it("counter and the default's first line", () => {
    expect(counterText(1234)).toEqual({ text: "1,234 / 4,000", over: false });
    expect(counterText(4001)).toEqual({ text: "4,001 / 4,000 (the rest is ignored)", over: true });
    expect(firstLine("\n  Focus on tests.\nMore")).toBe("Focus on tests.");
  });
});
