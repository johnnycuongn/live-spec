import { describe, expect, it } from "vitest";
import type { TabSummary } from "./ipc";
import {
  afterClose,
  groupTabs,
  projectName,
  readCollapsed,
  renameResult,
  rowTitle,
  stepTab,
  unreadableNotice,
  visualOrder,
  windowTitle,
  writeCollapsed,
} from "./tabList";
import { tabTitle } from "./tabTitle";

const t = (id: string, root: string | null, firstLine = "", title: string | null = null): TabSummary => ({
  id,
  root,
  title,
  firstLine,
  updatedAt: 0,
  closedAt: null,
});

describe("projectName", () => {
  it("is the folder name, for either separator", () => {
    expect(projectName("/Users/me/code/shop-api")).toBe("shop-api");
    expect(projectName("/Users/me/code/shop-api/")).toBe("shop-api");
    expect(projectName("C:\\code\\shop-api")).toBe("shop-api");
    expect(projectName(null)).toBe("No project");
  });
});

describe("groupTabs", () => {
  it("groups by project in order of first appearance, keeping list order inside a group", () => {
    const groups = groupTabs([t("a1", "/p/a"), t("n1", null), t("b1", "/p/b"), t("a2", "/p/a")]);
    expect(groups.map((g) => g.name)).toEqual(["a", "No project", "b"]);
    expect(groups[0].tabs.map((x) => x.id)).toEqual(["a1", "a2"]);
  });

  it("a new tab joins the end of its project's group", () => {
    // create_tab appends to the stored list; the group shows it last.
    const after = groupTabs([t("a1", "/p/a"), t("b1", "/p/b"), t("a2", "/p/a")]);
    expect(visualOrder(after)).toEqual(["a1", "a2", "b1"]);
  });

  it("two projects with the same folder name get their parent folder", () => {
    const names = groupTabs([t("x", "/work/app"), t("y", "/oss/app"), t("z", "/work/api")]).map((g) => g.name);
    expect(names).toEqual(["app · work", "app · oss", "api"]);
  });
});

describe("tab order", () => {
  const order = visualOrder(groupTabs([t("a1", "/a"), t("b1", "/b"), t("a2", "/a")]));

  it("⌃Tab follows the sidebar's order and wraps", () => {
    expect(order).toEqual(["a1", "a2", "b1"]);
    expect(stepTab(order, "a2", 1)).toBe("b1"); // the stored list would wrap to a1
    expect(stepTab(order, "b1", 1)).toBe("a1");
    expect(stepTab(order, "a1", -1)).toBe("b1");
    expect(stepTab(order, null, 1)).toBe("a1");
    expect(stepTab(order, null, -1)).toBe("b1");
    expect(stepTab([], null, 1)).toBeNull();
  });

  it("closing a tab shows the next one, else the previous, else none", () => {
    expect(afterClose(order, "a1")).toBe("a2");
    expect(afterClose(order, "b1")).toBe("a2");
    expect(afterClose(["only"], "only")).toBeNull();
    expect(afterClose(order, "gone")).toBeNull();
  });
});

describe("titles", () => {
  it("tabTitle: the title, else the first line, else Untitled", () => {
    expect(tabTitle({ title: "Checkout retry", firstLine: "what if" })).toBe("Checkout retry");
    expect(tabTitle({ title: null, firstLine: "what if" })).toBe("what if");
    expect(tabTitle({ title: null, firstLine: "" })).toBe("Untitled");
  });

  it("a shown tab's title follows its live draft", () => {
    const saved = t("a", "/a", "old first line");
    expect(rowTitle(saved, null)).toBe("old first line");
    expect(rowTitle(saved, "\n  new first line\nmore")).toBe("new first line");
    expect(rowTitle({ ...saved, title: "Named" }, "new first line")).toBe("Named");
  });

  it("the window title names the active tab", () => {
    expect(windowTitle("Checkout retry")).toBe("LiveBrainstorm — Checkout retry");
    expect(windowTitle(null)).toBe("LiveBrainstorm");
  });
});

describe("renameResult", () => {
  it("confirming the automatic title unchanged keeps it automatic", () => {
    expect(renameResult("what if", null, "what if")).toBeUndefined();
    expect(renameResult("  what if  ", null, "what if")).toBeUndefined();
  });

  it("a blank name returns to the automatic title", () => {
    expect(renameResult("   ", "Named", "what if")).toBeNull();
    expect(renameResult("", null, "what if")).toBeUndefined();
  });

  it("a new name is trimmed; the same name is no change", () => {
    expect(renameResult("  Checkout retry ", null, "what if")).toBe("Checkout retry");
    expect(renameResult("Named", "Named", "what if")).toBeUndefined();
  });
});

describe("unreadableNotice", () => {
  it("names the kept copy", () => {
    expect(unreadableNotice("/data/sessions/tabs/t-1a-0.json.bad")).toBe("Couldn't read a tab; kept it as t-1a-0.json.bad");
    expect(unreadableNotice("C:\\data\\sessions\\workspace.json.bad.1")).toBe(
      "Couldn't read the saved windows; kept them as workspace.json.bad.1",
    );
  });
});

describe("sidebar flag", () => {
  const memory = () => {
    const m = new Map<string, string>();
    return { m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
  };

  it("starts collapsed in a narrow window, unless the window remembers a choice", () => {
    const s = memory();
    expect(readCollapsed("w-1", 1099, s)).toBe(true);
    expect(readCollapsed("w-1", 1100, s)).toBe(false);
    writeCollapsed("w-1", false, s);
    expect(s.m.get("livebrainstorm.sidebar.w-1")).toBe("0");
    expect(readCollapsed("w-1", 900, s)).toBe(false);
    expect(readCollapsed("w-2", 900, s)).toBe(true);
  });

  it("storage that throws falls back to the width and never breaks", () => {
    const broken = {
      getItem: (): string | null => {
        throw new Error("SecurityError");
      },
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
    };
    expect(readCollapsed("w-1", 900, broken)).toBe(true);
    expect(() => writeCollapsed("w-1", true, broken)).not.toThrow();
    expect(readCollapsed("w-1", 1400, undefined)).toBe(false);
  });
});
