// Parallel agents stream their sections on separate lanes. Each lane keeps
// its own buffer, and the notes are the lanes joined in lane order, so
// interleaved chunks never mix inside a section. Lanes mirror
// prompt::section_groups in Rust.

export function appendLane(lanes: string[], lane: number, text: string): string[] {
  const next = [...lanes];
  while (next.length <= lane) next.push("");
  next[lane] += text;
  return next;
}

export const joinLanes = (lanes: string[]) => lanes.filter((l) => l.length > 0).join("\n");
