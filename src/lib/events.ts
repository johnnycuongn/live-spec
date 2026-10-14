// Events every window receives from the shell (`app.emit`), unlike a
// command's own Channel. Names and payloads mirror src-tauri/src/events.rs.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
