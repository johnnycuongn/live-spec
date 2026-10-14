// Tab titles. `firstLine` mirrors `sessions::first_line` in Rust, a
// cross-language contract: both test suites pin the same strings.

/** Characters (Unicode scalars, not UTF-16 units) a title takes from the draft. */
export const TITLE_CHARS = 48;

// Unicode White_Space, which Rust's `str::trim` uses. JS's own `trim` differs
// on two characters: it strips U+FEFF and keeps U+0085.
const LEADING = /^\p{White_Space}+/u;
const TRAILING = /\p{White_Space}+$/u;

/** The draft's first non-empty line, trimmed, at most TITLE_CHARS characters. */
export function firstLine(draft: string): string {
  for (const line of draft.split("\n")) {
    const t = line.replace(LEADING, "").replace(TRAILING, "");
    if (t) return Array.from(t).slice(0, TITLE_CHARS).join("").replace(TRAILING, "");
  }
  return "";
}

/** The explicit title, else the draft's first line, else "Untitled". */
export function tabTitle(t: { title: string | null; firstLine: string }): string {
  return t.title || t.firstLine || "Untitled";
}
