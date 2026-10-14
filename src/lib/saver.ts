/**
 * Saves a value a moment after the last change. A pending save remembers its
 * own key (the project root), so switching projects can never save one
 * project's text under another's. `flush` saves now; call it before switching
 * projects and on unmount. Failures go to `onError`, never thrown. `flush` also
 * waits for a save already in flight (the timer may have started it), and resolves
 * to whether everything saved, so a caller about to drop the text can refuse.
 */
export function createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void) {
  let pending: { key: string; value: T } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const inflight = new Set<Promise<boolean>>();
  const send = (p: { key: string; value: T }): void => {
    const r = new Promise<void>((resolve) => resolve(save(p.key, p.value))).then(
      () => true,
      (e) => (onError(e), false),
    );
    inflight.add(r);
    void r.finally(() => inflight.delete(r));
  };

  const flush = async (): Promise<boolean> => {
    if (timer) clearTimeout(timer);
    timer = null;
    const p = pending;
    pending = null;
    if (p) send(p);
    const results = await Promise.all([...inflight]);
    return results.every(Boolean);
  };

  return {
    schedule(key: string, value: T) {
      if (pending && pending.key !== key) void flush(); // another project's edit goes out first
      pending = { key, value };
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void flush(), delayMs);
    },
    flush,
  };
}
