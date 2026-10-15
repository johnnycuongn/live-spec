import { describe, expect, it } from "vitest";
import { moveActive, typeahead } from "./listbox";

describe("moveActive", () => {
  it("steps and clamps at the ends", () => {
    expect(moveActive("ArrowDown", 0, 3)).toBe(1);
    expect(moveActive("ArrowDown", 2, 3)).toBe(2);
    expect(moveActive("ArrowUp", 0, 3)).toBe(0);
    expect(moveActive("Home", 2, 3)).toBe(0);
    expect(moveActive("End", 0, 3)).toBe(2);
    expect(moveActive("PageDown", 0, 3)).toBe(2);
  });

  it("ignores other keys and empty lists", () => {
    expect(moveActive("a", 0, 3)).toBeNull();
    expect(moveActive("ArrowDown", 0, 0)).toBeNull();
  });
});

describe("typeahead", () => {
  const labels = ["Default", "Low", "Medium", "High", "Extra high", "Max"];

  it("finds the next label with the typed prefix, ignoring case", () => {
    expect(typeahead(labels, "h", 0)).toBe(3);
    expect(typeahead(labels, "ex", 0)).toBe(4);
  });

  it("cycles through labels when one letter repeats", () => {
    expect(typeahead(labels, "m", 0)).toBe(2);
    expect(typeahead(labels, "m", 2)).toBe(5);
    expect(typeahead(labels, "mm", 5)).toBe(2);
  });

  it("keeps the current option while a longer prefix still matches it", () => {
    expect(typeahead(labels, "ma", 5)).toBe(5);
  });

  it("returns null when nothing matches", () => {
    expect(typeahead(labels, "z", 0)).toBeNull();
    expect(typeahead(labels, "", 0)).toBeNull();
  });
});
