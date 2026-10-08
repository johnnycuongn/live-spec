// Splits the streaming analysis into the four note sections.
// Tolerant on purpose: small local models write "**Faults:**" or "### Faults".
// Headings mirror SECTIONS in crates/livebrainstorm-core/src/prompt.rs.

export type SectionKey = "intent" | "faults" | "directions" | "questions";

export const SECTIONS: { key: SectionKey; title: string }[] = [
  { key: "intent", title: "What you're after" },
  { key: "faults", title: "Faults" },
  { key: "directions", title: "Directions" },
  { key: "questions", title: "Questions for you" },
];

const HEADING = /^\s*(?:#{1,4}\s*|\*\*\s*)(intent|faults?|directions?|questions?)\b[^\n]*$/i;

const keyFor = (word: string): SectionKey => {
  const w = word.toLowerCase();
  if (w.startsWith("intent")) return "intent";
  if (w.startsWith("fault")) return "faults";
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
