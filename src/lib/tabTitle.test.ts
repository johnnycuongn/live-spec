import { describe, expect, it } from "vitest";
import { firstLine, tabTitle } from "./tabTitle";

describe("firstLine", () => {
  // The same strings as sessions::tests::first_line_matches_the_frontend in Rust.
  it("is the first non-empty line, trimmed, at most 48 characters", () => {
    expect(firstLine("")).toBe("");
    expect(firstLine("\n  \n\t Retry failed checkout payments  \nsecond line")).toBe("Retry failed checkout payments");
    expect(firstLine("line one\r\nline two")).toBe("line one");
    expect(firstLine("a".repeat(60))).toBe("a".repeat(48));
    expect(firstLine(`${"x".repeat(47)} yz`)).toBe("x".repeat(47));
    expect(firstLine("😀".repeat(50))).toBe("😀".repeat(48));
    // Unicode White_Space is trimmed (NEL, no-break and ideographic spaces); a BOM is not whitespace.
    expect(firstLine("\u0085 Retry payments　\nmore")).toBe("Retry payments");
    expect(firstLine("﻿Retry")).toBe("﻿Retry");
  });
});

describe("tabTitle", () => {
  it("is the title, else the first line, else Untitled", () => {
    expect(tabTitle({ title: "Checkout retry", firstLine: "Retry failed" })).toBe("Checkout retry");
    expect(tabTitle({ title: null, firstLine: "Retry failed" })).toBe("Retry failed");
    expect(tabTitle({ title: null, firstLine: "" })).toBe("Untitled");
  });
});
