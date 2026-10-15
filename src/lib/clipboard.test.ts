import { describe, expect, it, vi } from "vitest";
import { copyText } from "./clipboard";

describe("copyText", () => {
  it("writes the text and reports success", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    expect(await copyText("the spec", { writeText })).toBe(true);
    expect(writeText).toHaveBeenCalledWith("the spec");
  });

  it("reports failure when the write is refused", async () => {
    expect(await copyText("x", { writeText: () => Promise.reject(new Error("denied")) })).toBe(false);
  });

  it("reports failure when the write throws before returning a promise", async () => {
    expect(
      await copyText("x", {
        writeText: () => {
          throw new Error("not allowed");
        },
      }),
    ).toBe(false);
  });

  it("reports failure when there is no clipboard", async () => {
    expect(await copyText("x", undefined)).toBe(false);
  });
});
