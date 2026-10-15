// Session-instruction rules, kept pure so they're testable. Types mirror
// crates/livebrainstorm-core/src/instructions.rs; the cap mirrors
// MAX_INSTRUCTION_CHARS in prompt.rs.
import type { ProjectInstructions, Template } from "./ipc";
import { isDefaultSections, normalizeSections } from "./sections";

export const MAX_INSTRUCTION_CHARS = 4000;

export const EMPTY_INSTRUCTIONS: ProjectInstructions = { default: "", current: "", templateId: null };

/** The top-bar label: which template, edited or not, or where the text came from. */
export function instructionsLabel(p: ProjectInstructions, templates: Template[]): string {
  if (!p.current.trim()) return "No instructions";
  if (p.templateId !== null) {
    const t = templates.find((x) => x.id === p.templateId);
    if (!t) return "Custom";
    return t.text === p.current ? t.name : `${t.name} (edited)`;
  }
  return p.current === p.default ? "Project default" : "Custom";
}

/** Changing the instructions re-reads the same draft, like a new project or brief. */
export const analysisContextKey = (root: string | undefined, briefCreatedAt: number | undefined, instructions: string) =>
  `${root ?? ""}|${briefCreatedAt ?? 0}|${instructions.trim()}`;

/**
 * A tab's key: its project's key plus its sections, only when they differ
 * from the default set. So a tab saved by the old build keeps the same key
 * and relaunching never re-runs it.
 */
export const tabContextKey = (projectKey: string, sections: string[]) =>
  isDefaultSections(sections) ? projectKey : `${projectKey}|${sections.join(",")}`;

/** Characters as the backend counts them (Unicode scalars), not UTF-16 units. */
export const charCount = (s: string) => [...s].length;

const count = (n: number) => n.toLocaleString("en-US");

export function counterText(length: number): { text: string; over: boolean } {
  const over = length > MAX_INSTRUCTION_CHARS;
  const text = `${count(length)} / ${count(MAX_INSTRUCTION_CHARS)}`;
  return { text: over ? `${text} (the rest is ignored)` : text, over };
}

export const firstLine = (text: string) => text.trim().split("\n")[0];

export interface AppliedTemplate {
  next: ProjectInstructions;
  /** The shown tab's sections after applying: the template's, or unchanged when it has none. */
  sections: string[];
  undo: { instructions: ProjectInstructions; sections: string[] };
}

/** Applying replaces the current text and, when the template has sections, the shown tab's sections; `undo` is exactly what was there. */
export function applyTemplate(p: ProjectInstructions, t: Template, current: string[]): AppliedTemplate {
  return {
    next: { ...p, current: t.text, templateId: t.id },
    sections: t.sections.length > 0 ? normalizeSections(t.sections) : current,
    undo: { instructions: p, sections: current },
  };
}

export const clearInstructions = (p: ProjectInstructions): ProjectInstructions => ({ ...p, current: "", templateId: null });
export const resetToDefault = (p: ProjectInstructions): ProjectInstructions => ({ ...p, current: p.default, templateId: null });
export const saveAsDefault = (p: ProjectInstructions): ProjectInstructions => ({ ...p, default: p.current });

export function upsertTemplate(list: Template[], t: Template): Template[] {
  return list.some((x) => x.id === t.id) ? list.map((x) => (x.id === t.id ? t : x)) : [...list, t];
}

export const removeTemplate = (list: Template[], id: string) => list.filter((t) => t.id !== id);
