// Menu markup: checkbox items for toggles (the Sections picker), and plain
// menus (sidebar ⋯, tab context menu) left exactly as they were.
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Menu, type MenuItem } from "./ui";

beforeEach(() => vi.stubGlobal("window", { innerWidth: 1280, innerHeight: 820 }));
afterEach(() => vi.unstubAllGlobals());

const render = (items: MenuItem[]) =>
  renderToString(createElement(Menu, { at: { x: 10, y: 10 }, items, label: "Test", onClose: () => {} }));

describe("Menu", () => {
  it("renders toggles as checkbox items with a mark on the checked ones", () => {
    const html = render([
      { label: "Intent", checked: true, onSelect: () => {} },
      { label: "Gaps", checked: false, onSelect: () => {} },
    ]);
    expect(html.match(/role="menuitemcheckbox"/g)).toHaveLength(2);
    expect(html).toContain('aria-checked="true"');
    expect(html).toContain('aria-checked="false"');
    expect(html.match(/✓/g)).toHaveLength(1);
  });

  it("leaves a menu without toggles as plain items with no mark column", () => {
    const html = render([
      { label: "Rename", onSelect: () => {} },
      { label: "Close", onSelect: () => {} },
    ]);
    expect(html.match(/role="menuitem"/g)).toHaveLength(2);
    expect(html).not.toContain("aria-checked");
    expect(html).not.toContain("<span");
    expect(html).toContain(">Rename</button>");
  });
});
