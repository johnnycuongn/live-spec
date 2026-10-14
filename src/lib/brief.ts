// While the brief builds, parallel explorers report on lanes 1..n and the
// single explorer (or the merge) on lane 0. Mirrors build_brief in Rust.
export const activityLine = (label: string, lane: number) => (lane > 0 ? `Agent ${lane} · ${label}` : label);
