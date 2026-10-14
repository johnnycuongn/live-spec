/**
 * Saves a value a moment after the last change. A pending save remembers its
 * own key (the project root), so switching projects can never save one
 * project's text under another's. `flush` saves now; call it before switching
 * projects and on unmount. Failures go to `onError`, never thrown.
 */
export function createSaver<T>(save: (key: string, value: T) => Promise<void>, delayMs: number, onError: (e: unknown) => void) {
  let pending: { key: string; value: T } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const flush = (): Promise<void> => {
    if (timer) clearTimeout(timer);
    timer = null;
    const p = pending;
    pending = null;
    return p ? save(p.key, p.value).catch(onError) : Promise.resolve();
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
