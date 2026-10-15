import { describe, expect, it } from "vitest";
import { checkLocation, checkSummary, plainClaim } from "./checks";

describe("checkSummary", () => {
  it("counts down while verdicts arrive", () => {
    expect(checkSummary({ count: 2, total: 3 }, 1, false)).toBe("Checking 2 of 3 conflicts…");
    expect(checkSummary({ count: 2, total: 3 }, 2, false)).toBe("2 of 3 conflicts checked");
  });

  it("stops claiming progress after a failure", () => {
    expect(checkSummary({ count: 2, total: 3 }, 1, true)).toBe("1 of 3 conflicts checked");
  });

  it("uses the singular for one conflict", () => {
    expect(checkSummary({ count: 1, total: 1 }, 1, false)).toBe("1 of 1 conflict checked");
  });
});

describe("display helpers", () => {
  it("formats where the evidence is", () => {
    expect(checkLocation({ claim: "c", verdict: "confirmed", confidence: 0.9, path: "src/a.ts", line: 4 })).toBe("src/a.ts:4");
    expect(checkLocation({ claim: "c", verdict: "notFound", confidence: null, path: null, line: null })).toBeNull();
  });

  it("strips inline markdown from claims", () => {
    expect(plainClaim("**Bold** `createInvoice` ignores discounts")).toBe("Bold createInvoice ignores discounts");
  });
});
