// The native window title follows the shown tab. Tauri doesn't copy document.title to
// the window, so it's set with setTitle (core:window:allow-set-title in the capability).
// The text comes from windowTitle in tabList.ts.
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Sets this window's title. Cosmetic: a failure is logged and leaves the previous title. */
export function setWindowTitle(title: string): Promise<void> {
  document.title = title;
  return getCurrentWindow()
    .setTitle(title)
    .catch((e: unknown) => console.error("setTitle failed", e));
}

/** Keeps this window's native title at `title`. */
export function useWindowTitle(title: string): void {
  useEffect(() => {
    void setWindowTitle(title);
  }, [title]);
}
