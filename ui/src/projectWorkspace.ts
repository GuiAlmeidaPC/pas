import type { TabConfig } from "./types";

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

export function defaultAgentPanelOpen(storage: Pick<Storage, "getItem"> | null | undefined): boolean {
  try {
    const saved = storage?.getItem("pas.show_ai_panel");
    return saved === null || saved === undefined ? true : saved === "true";
  } catch {
    return true;
  }
}
