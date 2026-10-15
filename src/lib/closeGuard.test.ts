import { describe, expect, it, vi } from "vitest";
import { createCloseGuard } from "./closeGuard";

describe("createCloseGuard", () => {
  it("closes when everything saved", async () => {
    const close = vi.fn(async () => {});
    const g = createCloseGuard(async () => true, close);
    await expect(g.request()).resolves.toBe("closed");
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("refuses when a save failed, and a second consecutive request closes anyway", async () => {
    const close = vi.fn(async () => {});
    const flushAll = vi.fn(async () => false);
    const g = createCloseGuard(flushAll, close);
    await expect(g.request()).resolves.toBe("refused");
    expect(close).not.toHaveBeenCalled();
    expect(g.armed()).toBe(true);
    await expect(g.request()).resolves.toBe("closed");
    expect(close).toHaveBeenCalledTimes(1);
    expect(flushAll).toHaveBeenCalledTimes(1); // the repeat does not wait on the failing save
    expect(g.armed()).toBe(false);
  });

  it("disarm (any other action or tab change) makes the next request refuse again", async () => {
    const close = vi.fn(async () => {});
    const g = createCloseGuard(async () => false, close);
    await g.request();
    g.disarm();
    expect(g.armed()).toBe(false);
    await expect(g.request()).resolves.toBe("refused");
    expect(close).not.toHaveBeenCalled();
  });

  it("a close that throws propagates and leaves the guard disarmed", async () => {
    const g = createCloseGuard(
      async () => false,
      async () => {
        throw "no";
      },
    );
    await g.request();
    await expect(g.request()).rejects.toBe("no");
    expect(g.armed()).toBe(false);
  });
});
