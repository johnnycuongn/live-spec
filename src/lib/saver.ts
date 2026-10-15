/**
 * Saves a value a moment after the last change. A pending save remembers its
 * own key (the project root), so switching projects can never save one
 * project's text under another's. `flush` saves now; call it before switching
 * projects and on unmount. Failures go to `onError`, never thrown. `flush` also
 * waits for a save already in flight (the timer may have started it), and resolves
 * to whether everything saved, so a caller about to drop the text can refuse.
 * A save that failed stays owed: the next `flush` retries it (and is false if it
 * fails again) until a newer value for that key supersedes it. `flush(key)` does all
 * of that for one key only, so a caller acting on behalf of that key (closing its tab)
 * is not refused by another key's owed save; `flush()` is global. `owed()` counts the
 * failed saves still owed.
 */
export function createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void) {
  interface Entry {
    key: string;
    value: T;
    seq: number;
  }
  let pending: Entry | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  /** Failed saves not yet superseded or retried, by key. */
  const failed = new Map<string, Entry>();
  /** The newest value scheduled per key: only its failure is worth retrying. */
  const latest = new Map<string, number>();
  let seq = 0;

  const inflight = new Map<Promise<boolean>, string>();
  const send = (p: Entry): void => {
    const r = new Promise<void>((resolve) => resolve(save(p.key, p.value))).then(
      () => true,
      (e) => {
        if (latest.get(p.key) === p.seq) failed.set(p.key, p);
        onError(e);
        return false;
      },
    );
    inflight.set(r, p.key);
    void r.finally(() => inflight.delete(r));
  };

  const flush = async (key?: string): Promise<boolean> => {
    // A keyed flush leaves another key's pending edit, and the timer that will send it, alone.
    const mine = pending !== null && (key === undefined || pending.key === key);
    if (mine) {
      if (timer) clearTimeout(timer);
      timer = null;
    }
    const retry = [...failed.values()].filter((e) => key === undefined || e.key === key);
    retry.forEach((e) => failed.delete(e.key));
    const p = mine ? pending : null;
    if (mine) pending = null;
    retry.forEach(send);
    if (p) send(p);
    const waiting = [...inflight].filter(([, k]) => key === undefined || k === key).map(([r]) => r);
    const results = await Promise.all(waiting);
    return results.every(Boolean) && (key === undefined ? failed.size === 0 : !failed.has(key));
  };

  return {
    schedule(key: string, value: T) {
      if (pending && pending.key !== key) void flush(pending.key); // another project's edit goes out first
      pending = { key, value, seq: ++seq };
      latest.set(key, pending.seq);
      failed.delete(key); // the newer value supersedes a failed older one
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void flush(), delayMs);
    },
    flush,
    /** How many failed saves are still owed (not yet superseded or retried). */
    owed: () => failed.size,
  };
}
