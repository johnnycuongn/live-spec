import { useCallback, useSyncExternalStore } from "react";
import type { TabSession, TabView } from "./tabSession";

/** The session's state, re-rendering on each change; null without a session. */
export function useTabSession(s: TabSession | null): TabView | null {
  const subscribe = useCallback((onChange: () => void) => (s ? s.subscribe(onChange) : () => {}), [s]);
  const read = useCallback(() => (s ? s.getState() : null), [s]);
  // The same read serves as the server snapshot: there's no server render, but tests render to a string.
  return useSyncExternalStore(subscribe, read, read);
}
