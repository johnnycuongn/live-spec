import { describe, expect, it } from "vitest";
import { appendLane, joinLanes } from "./lanes";
import { parseSections } from "./sections";

describe("lanes", () => {
  it("interleaved lanes render in lane order", () => {
    let lanes: string[] = [];
    lanes = appendLane(lanes, 1, "## Gaps\n- go");
    lanes = appendLane(lanes, 0, "## Intent\n- why");
    lanes = appendLane(lanes, 1, " faster");
    lanes = appendLane(lanes, 0, "\n## Conflicts\n- bug");
    expect(joinLanes(lanes)).toBe("## Intent\n- why\n## Conflicts\n- bug\n## Gaps\n- go faster");
    expect(parseSections(joinLanes(lanes))).toEqual({ intent: "- why", conflicts: "- bug", gaps: "- go faster" });
  });

  it("a lane that hasn't started leaves no gap", () => {
    expect(joinLanes(appendLane([], 2, "## Questions\n- q"))).toBe("## Questions\n- q");
  });

  it("never mutates the previous buffers", () => {
    const before = ["## Intent\n- a"];
    appendLane(before, 0, " b");
    expect(before).toEqual(["## Intent\n- a"]);
  });
});
