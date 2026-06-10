import type { Layout, Library, ProjectConfig, TabConfig } from "./types";

export const DEFAULT_UNSAVED_PROJECT_NAME = "Untitled Project";
export const DEFAULT_UNSAVED_PROGRAM_PATH = "untitled.pas";

export interface UnsavedProjectWorkspace {
  projectPath: null;
  projectName: string;
  programs: TabConfig[];
}

export function createUnsavedProjectWorkspace(content: string): UnsavedProjectWorkspace {
  return {
    projectPath: null,
    projectName: DEFAULT_UNSAVED_PROJECT_NAME,
    programs: [{ path: DEFAULT_UNSAVED_PROGRAM_PATH, content }],
  };
}

export function isProjectOpen(projectName: string | null): boolean {
  return projectName !== null;
}

/**
 * Merge currently-open tab paths into the program list so saving captures
 * any files the user opened without explicitly adding to the project.
 * Order: existing project programs first, then any open tabs not already
 * in that list.
 */
export function mergeProgramPaths(
  activePrograms: TabConfig[],
  openTabPaths: string[],
): string[] {
  const seen = new Set<string>();
  const programPaths: string[] = [];
  for (const p of activePrograms) {
    if (!seen.has(p.path)) {
      programPaths.push(p.path);
      seen.add(p.path);
    }
  }
  for (const p of openTabPaths) {
    if (!seen.has(p)) {
      programPaths.push(p);
      seen.add(p);
    }
  }
  return programPaths;
}

export interface ProjectConfigInput {
  name: string | null;
  libraries: Library[];
  /** Programs with their content already resolved for embedding. */
  programs: TabConfig[];
  openTabPaths: string[];
  activeTabPath: string | null;
  layout: Layout;
}

/** Assemble the on-disk project JSON from the current workspace state. */
export function buildProjectConfig({
  name,
  libraries,
  programs,
  openTabPaths,
  activeTabPath,
  layout,
}: ProjectConfigInput): ProjectConfig {
  return {
    version: 1,
    name: name ?? "project",
    libnames: libraries
      .filter((l) => l.kind !== "memory")
      .map((l) => ({
        name: l.name,
        kind: l.kind,
        path: l.path,
        format: l.format ?? null,
      })),
    programs,
    open_tabs: openTabPaths.map((p) => ({ path: p })),
    active_tab: activeTabPath,
    layout,
  };
}

export function defaultAgentPanelOpen(storage: Pick<Storage, "getItem"> | null | undefined): boolean {
  try {
    const saved = storage?.getItem("pas.show_ai_panel");
    return saved === null || saved === undefined ? true : saved === "true";
  } catch {
    return true;
  }
}
