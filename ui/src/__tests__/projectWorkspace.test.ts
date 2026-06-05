import { describe, expect, it } from "vitest";
import {
  DEFAULT_UNSAVED_PROGRAM_PATH,
  createUnsavedProjectWorkspace,
  defaultAgentPanelOpen,
  isProjectOpen,
} from "../projectWorkspace";

describe("project workspace startup defaults", () => {
  it("starts with an unsaved project containing the starter program", () => {
    const workspace = createUnsavedProjectWorkspace("data demo; run;");

    expect(workspace.projectPath).toBeNull();
    expect(workspace.projectName).toBe("Untitled Project");
    expect(workspace.programs).toEqual([
      { path: DEFAULT_UNSAVED_PROGRAM_PATH, content: "data demo; run;" },
    ]);
    expect(isProjectOpen(workspace.projectName)).toBe(true);
  });

  it("opens the Agent panel by default when no preference exists", () => {
    expect(defaultAgentPanelOpen({ getItem: () => null })).toBe(true);
  });

  it("honors an explicit hidden Agent panel preference", () => {
    expect(defaultAgentPanelOpen({ getItem: () => "false" })).toBe(false);
  });
});
