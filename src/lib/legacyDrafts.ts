// The one-time upgrade from the build that kept one draft per project in
// localStorage: each non-empty draft becomes a tab. The keys go only once the
// import has succeeded, so a failure loses nothing.
import { errorMessage, type LegacyDraft } from "./ipc";

const DRAFT_PREFIX = "livebrainstorm.draft.";
const LAST_PROJECT = "livebrainstorm.lastProject";

type LegacyStorage = Pick<Storage, "length" | "key" | "getItem" | "removeItem">;

/** Every saved draft (the root "none" is no project), the last project, and every key to remove afterwards. */
export function collectLegacyDrafts(storage: Pick<Storage, "length" | "key" | "getItem">): {
  drafts: LegacyDraft[];
  activeRoot: string | null;
  keys: string[];
} {
  const drafts: LegacyDraft[] = [];
  const keys: string[] = [];
  for (let i = 0; i < storage.length; i++) {
    const key = storage.key(i);
    if (key === null || !key.startsWith(DRAFT_PREFIX)) continue;
    keys.push(key);
    const draft = storage.getItem(key) ?? "";
    const root = key.slice(DRAFT_PREFIX.length);
    if (draft.trim()) drafts.push({ root: root === "none" ? null : root, draft });
  }
  const activeRoot = storage.getItem(LAST_PROJECT);
  if (activeRoot !== null) keys.push(LAST_PROJECT);
  // localStorage's key order isn't defined: projects by path, then the no-project draft.
  drafts.sort((a, b) => (a.root === null ? 1 : b.root === null ? -1 : a.root.localeCompare(b.root)));
  return { drafts, activeRoot, keys };
}

/** localStorage, or undefined where it can't be used. */
export function browserStorage(): LegacyStorage | undefined {
  try {
    return typeof localStorage === "undefined" ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

/** Imports the old drafts into this window. Returns the message to show if that failed, else null. */
export async function importLegacy(
  windowId: string,
  storage: LegacyStorage | undefined,
  importDrafts: (window: string, drafts: LegacyDraft[], activeRoot: string | null) => Promise<void>,
): Promise<string | null> {
  if (!storage) return null;
  const legacy = collectLegacyDrafts(storage);
  if (legacy.keys.length === 0) return null;
  try {
    if (legacy.drafts.length > 0) await importDrafts(windowId, legacy.drafts, legacy.activeRoot);
  } catch (e) {
    return `Couldn't bring over your earlier drafts: ${errorMessage(e)}`; // the keys stay, so nothing is lost
  }
  for (const key of legacy.keys) {
    try {
      storage.removeItem(key);
    } catch {
      // Left behind. Harmless: the workspace is no longer fresh, so it never imports again.
    }
  }
  return null;
}
