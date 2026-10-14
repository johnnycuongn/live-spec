/**
 * Saves a value a moment after the last change. A pending save remembers its
 * own key (the project root), so switching projects can never save one
 * project's text under another's. `flush` saves now; call it before switching
 * projects and on unmount. Failures go to `onError`, never thrown. `flush` also
 * waits for a save already in flight (the timer may have started it), and resolves
 * to whether everything saved, so a caller about to drop the text can refuse.
 * A save that failed stays owed: the next `flush` retries it (and is false if it
 * fails again) until a newer value for that key supersedes it.
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

  const inflight = new Set<Promise<boolean>>();
  const send = (p: Entry): void => {
    const r = new Promise<void>((resolve) => resolve(save(p.key, p.value))).then(
      () => true,
      (e) => {
        if (latest.get(p.key) === p.seq) failed.set(p.key, p);
        onError(e);
        return false;
      },
    );
    inflight.add(r);
    void r.finally(() => inflight.delete(r));
  };

  const flush = async (): Promise<boolean> => {
    if (timer) clearTimeout(timer);
    timer = null;
    const retry = [...failed.values()];
    failed.clear();
    const p = pending;
    pending = null;
    retry.forEach(send);
    if (p) send(p);
    const results = await Promise.all([...inflight]);
    return results.every(Boolean) && failed.size === 0;
  };

  return {
    schedule(key: string, value: T) {
      if (pending && pending.key !== key) void flush(); // another project's edit goes out first
      pending = { key, value, seq: ++seq };
      latest.set(key, pending.seq);
      failed.delete(key); // the newer value supersedes a failed older one
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void flush(), delayMs);
    },
    flush,
  };
}
