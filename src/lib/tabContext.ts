// When the shown tab's analysis context is ready, and what it is. The context
// key comes from the project's brief and the instructions, so it is computed
// only once both have loaded: a key computed earlier ("root|0|") would differ
// from the saved notes' and start a spurious run at launch.
import type { ProjectSummary } from "./ipc";
import { analysisContextKey } from "./instructions";

/** The shown tab's project as the window loaded it: its summary, or why it couldn't be read. */
export interface LoadedProject {
  root: string;
  summary: ProjectSummary | null;
  error: string | null;
}

/** The current instructions loaded for one root ("" for no project). */
export interface LoadedInstructions {
  root: string;
  current: string;
}

/**
 * The tab's context key and instructions, or null while its project summary
 * or its instructions are still loading. A project that couldn't be read has
 * no brief: it analyses without code context.
 */
export function tabContext(
  root: string | null,
  project: LoadedProject | null,
  instructions: LoadedInstructions | null,
): { key: string; instructions: string } | null {
  if (!instructions || instructions.root !== (root ?? "")) return null;
  let briefCreatedAt: number | undefined;
  if (root !== null) {
    if (!project || project.root !== root || (project.summary === null && project.error === null)) return null;
    briefCreatedAt = project.summary?.brief?.createdAt;
  }
  return { key: analysisContextKey(root ?? undefined, briefCreatedAt, instructions.current), instructions: instructions.current };
}
