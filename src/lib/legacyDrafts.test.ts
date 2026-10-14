import { describe, expect, it, vi } from "vitest";
import type { LegacyDraft } from "./ipc";
import { collectLegacyDrafts, importLegacy } from "./legacyDrafts";

/** A localStorage stand-in whose contents the test can read. */
function storage(entries: Record<string, string>) {
  const m = new Map(Object.entries(entries));
  return {
    m,
    get length() {
      return m.size;
    },
    key: (i: number) => [...m.keys()][i] ?? null,
    getItem: (k: string) => m.get(k) ?? null,
    removeItem: (k: string) => void m.delete(k),
  };
}

const importer = () => vi.fn((_window: string, _drafts: LegacyDraft[], _activeRoot: string | null) => Promise.resolve());

describe("collectLegacyDrafts", () => {
  it("collects every non-empty draft; the root 'none' is no project", () => {
    const s = storage({
      "livebrainstorm.draft./b": "b idea",
      "livebrainstorm.draft.none": "loose idea",
      "livebrainstorm.draft./a": "  ",
      "livebrainstorm.lastProject": "/b",
      other: "x",
    });
    expect(collectLegacyDrafts(s)).toEqual({
      drafts: [
        { root: "/b", draft: "b idea" },
        { root: null, draft: "loose idea" },
      ],
      activeRoot: "/b",
      keys: ["livebrainstorm.draft./b", "livebrainstorm.draft.none", "livebrainstorm.draft./a", "livebrainstorm.lastProject"],
    });
  });
});

describe("importLegacy", () => {
  it("imports, then removes only the old build's keys", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea", "livebrainstorm.lastProject": "/a", other: "x" });
    const importDrafts = importer();
    expect(await importLegacy("w-1", s, importDrafts)).toBeNull();
    expect(importDrafts).toHaveBeenCalledWith("w-1", [{ root: "/a", draft: "a idea" }], "/a");
    expect([...s.m.keys()]).toEqual(["other"]);
  });

  it("keeps the keys when the import fails, and says so", async () => {
    const s = storage({ "livebrainstorm.draft./a": "a idea" });
    const failed = await importLegacy("w-1", s, () => Promise.reject(new Error("disk full")));
    expect(failed).toBe("Couldn't bring over your earlier drafts: disk full");
    expect(s.m.has("livebrainstorm.draft./a")).toBe(true);
  });

  it("only empty drafts: nothing to import, and the keys go", async () => {
    const s = storage({ "livebrainstorm.draft./a": "  " });
    const importDrafts = importer();
    expect(await importLegacy("w-1", s, importDrafts)).toBeNull();
    expect(importDrafts).not.toHaveBeenCalled();
    expect(s.m.size).toBe(0);
  });

  it("does nothing without storage or keys", async () => {
    const importDrafts = importer();
    expect(await importLegacy("w-1", undefined, importDrafts)).toBeNull();
    expect(await importLegacy("w-1", storage({ other: "x" }), importDrafts)).toBeNull();
    expect(importDrafts).not.toHaveBeenCalled();
  });
});
