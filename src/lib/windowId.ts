// This window's workspace id. Windows are labelled with their id (`w-<n>`).
// A window with any other label (the one tauri.conf.json declares, while it
// declares one) is workspace window `w-1`.
import { getCurrentWindow } from "@tauri-apps/api/window";

export function currentWindowId(): string {
  let label = "";
  try {
    label = getCurrentWindow().label;
  } catch {
    // Outside Tauri (tests) there is no window.
  }
  return /^w-/.test(label) ? label : "w-1";
}
