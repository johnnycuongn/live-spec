// Events every window receives from the shell (`app.emit`), unlike a
// command's own Channel. Names and payloads mirror src-tauri/src/events.rs.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** A brief build for `root` started, finished or failed (`events::BriefStatus`). */
export interface BriefStatusEvent {
  root: string;
  state: "building" | "done" | "error";
  message: string | null;
}

/** A session changed (`events::WorkspaceChanged`): these windows' tab lists, and the shared closed list. */
export interface WorkspaceChangedEvent {
  windows: string[];
}

export function onBriefStatus(fn: (e: BriefStatusEvent) => void): Promise<UnlistenFn> {
  return listen<BriefStatusEvent>("brief-status", (e) => fn(e.payload));
}

export function onWorkspaceChanged(fn: (e: WorkspaceChangedEvent) => void): Promise<UnlistenFn> {
  return listen<WorkspaceChangedEvent>("workspace-changed", (e) => fn(e.payload));
}

// ---- Phase 3 ----

/** Mirrors `events::MenuEvent`: an app-menu item, by its `ActionId`. */
export interface MenuEvent {
  id: string;
}

/** This window's app-menu items. Window-scoped on purpose: the shell sends each item to the
 *  focused window only, and a global `listen` would also receive the other windows' items. */
export const onMenu = (fn: (e: MenuEvent) => void): Promise<UnlistenFn> =>
  getCurrentWindow().listen<MenuEvent>("menu", (e) => fn(e.payload));
