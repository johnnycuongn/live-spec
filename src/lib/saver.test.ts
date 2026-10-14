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
});
