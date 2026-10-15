// Close Window must save every edit first, but a save that keeps failing (disk error, a
// tab file gone) must not trap the user in the window. The first request that finds an
// edit unsaved is refused and arms the guard; a second consecutive request closes anyway,
// losing the edit only on that explicit repeat. Anything else the user does disarms it.

export function createCloseGuard(flushAll: () => Promise<boolean>, close: () => Promise<void>) {
  let armed = false;
  return {
    /** "refused": an edit couldn't be saved, press again to close anyway. */
    async request(): Promise<"closed" | "refused"> {
      if (armed) {
        armed = false;
        await close();
        return "closed";
      }
      if (!(await flushAll())) {
        armed = true;
        return "refused";
      }
      await close();
      return "closed";
    },
    /** Any other action or a tab change: the next request is a first one again. */
    disarm(): void {
      armed = false;
    },
    armed: () => armed,
  };
}
