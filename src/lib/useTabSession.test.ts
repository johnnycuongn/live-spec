import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { Tab } from "./ipc";
import { TabSession } from "./tabSession";
import { useTabSession } from "./useTabSession";

const tab: Tab = {
  id: "t-1",
  root: null,
  title: null,
  createdAt: 1,
  updatedAt: 1,
  closedAt: null,
  draft: "an idea about retries",
  nextSnapshot: 1,
  history: [],
};

function Probe({ s }: { s: TabSession | null }) {
  const view = useTabSession(s);
  return createElement("p", null, view ? `${view.draft} · ${view.analysis.status}` : "no tab");
}

describe("useTabSession", () => {
  it("reads the session's state, or null without a session", () => {
    const s = new TabSession(tab, { analyze: vi.fn(), saveDraft: vi.fn(), debounceMs: () => 1000 });
    expect(renderToString(createElement(Probe, { s }))).toBe("<p>an idea about retries · idle</p>");
    expect(renderToString(createElement(Probe, { s: null }))).toBe("<p>no tab</p>");
  });
});
