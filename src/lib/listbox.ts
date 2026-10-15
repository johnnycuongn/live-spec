// Keyboard logic for the custom Select (ui.tsx), kept pure so it can be tested without a DOM.

/** Where a navigation key moves the active option, or null for any other key. */
export function moveActive(key: string, active: number, count: number): number | null {
  if (count === 0) return null;
  switch (key) {
    case "ArrowDown":
      return Math.min(active + 1, count - 1);
    case "ArrowUp":
      return Math.max(active - 1, 0);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    case "PageDown":
      return Math.min(active + 10, count - 1);
    case "PageUp":
      return Math.max(active - 10, 0);
    default:
      return null;
  }
}

/**
 * The option type-ahead lands on: the first label after `active` that starts
 * with `typed`, wrapping around. Repeating one letter ("ss") cycles through
 * the labels starting with it, as a native select does.
 */
export function typeahead(labels: string[], typed: string, active: number): number | null {
  if (!typed) return null;
  const text = typed.toLowerCase();
  const same = [...text].every((c) => c === text[0]);
  const prefix = same ? text[0] : text;
  // A longer prefix may still match the current option; a single letter moves on.
  const start = same ? active + 1 : active;
  for (let k = 0; k < labels.length; k++) {
    const i = (start + k) % labels.length;
    if (labels[i].toLowerCase().startsWith(prefix)) return i;
  }
  return null;
}
