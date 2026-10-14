import { describe, expect, it } from "vitest";
import { activityLine } from "./brief";

describe("activityLine", () => {
  it("tags explorer activity with its agent", () => {
    expect(activityLine("Read src/main.rs", 2)).toBe("Agent 2 · Read src/main.rs");
  });

  it("leaves single-explorer activity as it was", () => {
    expect(activityLine("Read src/main.rs", 0)).toBe("Read src/main.rs");
  });
});
