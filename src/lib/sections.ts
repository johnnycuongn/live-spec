// The section catalogue and the streaming parser. Mirrors sections.rs in
// crates/livebrainstorm-core: ids, order, the default set and the heading
// alias table must match. Tolerant on purpose: small local models write
// "**Conflicts:**" or "### Conflicts with code".

export type SectionKey = "intent" | "conflicts" | "gaps" | "ambiguities" | "questions" | "directions";

export interface SectionDef {
  key: SectionKey;
  title: string;
  /** The eyebrow dot: red for a clash with the code, blue for something to add, amber for an open item. */
  dot: string;
  /** The old build's section: rendered when a saved analysis has it, never offered. */
  legacy?: boolean;
}

export const SECTIONS: SectionDef[] = [
  { key: "intent", title: "Intent", dot: "bg-muted" },
  { key: "conflicts", title: "Conflicts with code", dot: "bg-fault" },
  { key: "gaps", title: "Gaps", dot: "bg-accent" },
  { key: "ambiguities", title: "Ambiguities", dot: "bg-question" },
  { key: "questions", title: "Questions for you", dot: "bg-question" },
  { key: "directions", title: "Directions", dot: "bg-accent", legacy: true },
];

/** What a tab can pick, in the order the notes show them. */
export const CATALOGUE: SectionDef[] = SECTIONS.filter((s) => !s.legacy);

/** A tab with no stored choice (an empty list) gets the whole catalogue. */
export const DEFAULT_SECTIONS: string[] = CATALOGUE.map((s) => s.key);

/** Known ids in catalogue order, once each. Empty stays empty: the default set. */
export const normalizeSections = (ids: string[]): string[] => CATALOGUE.map((s) => s.key).filter((k) => ids.includes(k));

export const effectiveSections = (ids: string[]): string[] => (ids.length === 0 ? DEFAULT_SECTIONS : ids);

export const isDefaultSections = (ids: string[]): boolean =>
  ids.length === 0 || (ids.length === DEFAULT_SECTIONS.length && ids.every((id, i) => id === DEFAULT_SECTIONS[i]));

/**
 * Adds or removes `key`. A tab keeps at least one section (the last one stays);
 * `allowEmpty` is for templates, where an empty list means "leave the tab's sections alone".
 */
export function toggleSection(chosen: string[], key: string, allowEmpty = false): string[] {
  const current = effectiveSections(chosen);
  if (current.includes(key)) {
    const next = current.filter((k) => k !== key);
    return next.length === 0 && !allowEmpty ? current : next;
  }
  return normalizeSections([...current, key]);
}

/** "Intent, Conflicts with code and Questions for you." for the notes' empty state. */
export function sectionsSentence(ids: string[]): string {
  const titles = CATALOGUE.filter((s) => effectiveSections(ids).includes(s.key)).map((s) => s.title);
  if (titles.length <= 1) return `${titles.join("")}.`;
  return `${titles.slice(0, -1).join(", ")} and ${titles[titles.length - 1]}.`;
}

const HEADING = /^\s*(?:#{1,4}\s*|\*\*\s*)(intent|conflicts?|faults?|gaps?|ambiguit(?:y|ies)|questions?|directions?)\b[^\n]*$/i;

const keyFor = (word: string): SectionKey => {
  const w = word.toLowerCase();
  if (w.startsWith("intent")) return "intent";
  if (w.startsWith("conflict") || w.startsWith("fault")) return "conflicts";
  if (w.startsWith("gap")) return "gaps";
  if (w.startsWith("ambigu")) return "ambiguities";
  if (w.startsWith("direction")) return "directions";
  return "questions";
};

export function parseSections(markdown: string): Partial<Record<SectionKey, string>> {
  const out: Partial<Record<SectionKey, string>> = {};
  let current: SectionKey | null = null;
  for (const line of markdown.split("\n")) {
    const m = HEADING.exec(line);
    if (m) {
      current = keyFor(m[1]);
      out[current] = out[current] ?? "";
      continue;
    }
    if (current) out[current] += `${line}\n`;
  }
  for (const k of Object.keys(out) as SectionKey[]) out[k] = out[k]!.trim();
  return out;
}
