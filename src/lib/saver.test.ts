import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSaver } from "./saver";

describe("createSaver", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("saves once, a moment after the last change", () => {
    const save = vi.fn((_key: string, _value: string) => Promise.resolve());
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "x");
    vi.advanceTimersByTime(300);
    s.schedule("/a", "xy");
    vi.advanceTimersByTime(499);
    expect(save).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("/a", "xy");
  });

  it("an edit pending for one project is saved to that project when another starts", () => {
    const save = vi.fn((_key: string, _value: string) => Promise.resolve());
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "typed in a");
    s.schedule("/b", "typed in b");
    expect(save).toHaveBeenCalledWith("/a", "typed in a");
    vi.advanceTimersByTime(500);
    expect(save).toHaveBeenLastCalledWith("/b", "typed in b");
    expect(save).toHaveBeenCalledTimes(2);
  });

  it("flush saves now, and only once", async () => {
    const save = vi.fn((_key: string, _value: string) => Promise.resolve());
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "x");
    await s.flush();
    await s.flush();
    vi.advanceTimersByTime(1000);
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("/a", "x");
  });

  it("reports a failed save instead of throwing", async () => {
    const onError = vi.fn();
    const s = createSaver(() => Promise.reject(new Error("disk full")), 500, onError);
    s.schedule("/a", "x");
    await s.flush();
    expect(onError).toHaveBeenCalledWith(new Error("disk full"));
  });

  it("flush resolves true when everything saved, false when a save failed", async () => {
    const ok = createSaver(() => Promise.resolve(), 500, () => {});
    ok.schedule("/a", "x");
    await expect(ok.flush()).resolves.toBe(true);
    await expect(ok.flush()).resolves.toBe(true); // nothing pending
    const bad = createSaver(() => Promise.reject(new Error("disk full")), 500, () => {});
    bad.schedule("/a", "x");
    await expect(bad.flush()).resolves.toBe(false);
  });

  it("flush waits for a save the timer already started", async () => {
    let finish!: () => void;
    const save = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    const s = createSaver(save, 500, () => {});
    s.schedule("/a", "x");
    vi.advanceTimersByTime(500); // the timer fires: the save is in flight, nothing pending
    expect(save).toHaveBeenCalledTimes(1);
    let done = false;
    const flushed = s.flush().then((ok) => ((done = true), ok));
    await Promise.resolve();
    await Promise.resolve();
    expect(done).toBe(false);
    finish();
    await expect(flushed).resolves.toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("flush reports a failure of the save already in flight", async () => {
    let fail!: (e: unknown) => void;
    const onError = vi.fn();
    const s = createSaver(() => new Promise<void>((_, reject) => (fail = reject)), 500, onError);
    s.schedule("/a", "x");
    vi.advanceTimersByTime(500);
    const flushed = s.flush();
    fail(new Error("disk full"));
    await expect(flushed).resolves.toBe(false);
    expect(onError).toHaveBeenCalledTimes(1);
  });
});
