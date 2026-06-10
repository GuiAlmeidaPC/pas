import type { MenuDef } from "./MenuBar";
import type { Pane } from "./useRunner";
import type { DatasetRef } from "./types";

export interface MenuActions {
  newTab: () => void;
  openFile: () => void;
  saveActiveTab: () => void;
  saveActiveTabAs: () => void;
  closeActiveTab: () => void;
  newProject: () => void;
  openProject: () => void;
  saveProject: () => void;
  saveProjectAs: () => void;
  addProgramToProject: () => void;
  runEditorAction: (id: string) => void;
  setZoomPercent: (update: (z: number) => number) => void;
  resetZoom: () => void;
  toggleBottomPane: () => void;
  toggleLayoutOrientation: () => void;
  toggleAIPanel: () => void;
  showPane: (pane: Pane) => void;
  submit: () => void;
  runProject: () => void;
  cancel: () => void;
  clearLog: () => void;
  clearOutputs: () => void;
  showShortcuts: () => void;
  showAbout: () => void;
}

export interface MenuState {
  hasProject: boolean;
  hasOutput: boolean;
  hasLog: boolean;
  activeDataset: DatasetRef | null;
  running: boolean;
  projectProgramCount: number;
  showBottomPane: boolean;
  showAIPanel: boolean;
  layoutOrientation: "vertical" | "horizontal";
}

/** The application menu bar definition. Pure: state in, menu tree out. */
export function buildMenus(actions: MenuActions, state: MenuState): MenuDef[] {
  return [
    {
      label: "File",
      items: [
        { label: "New Tab", shortcut: "Ctrl+N", onClick: actions.newTab },
        { label: "Open File…", shortcut: "Ctrl+O", onClick: actions.openFile },
        { separator: true },
        { label: "Save", shortcut: "Ctrl+S", onClick: actions.saveActiveTab },
        { label: "Save As…", onClick: actions.saveActiveTabAs },
        ...(state.hasProject
          ? [{ label: "Save to Standalone PAS File…", onClick: actions.saveActiveTabAs }]
          : []),
        { separator: true },
        {
          label: "Close Tab",
          shortcut: "Ctrl+W",
          onClick: actions.closeActiveTab,
        },
      ],
    },
    {
      label: "Project",
      items: [
        { label: "New Project", onClick: actions.newProject },
        { label: "Open Project…", onClick: actions.openProject },
        { label: "Save Project", onClick: actions.saveProject },
        { label: "Save Project As…", onClick: actions.saveProjectAs },
        { separator: true },
        { label: "Add Program to Project…", onClick: actions.addProgramToProject },
      ],
    },
    {
      label: "Edit",
      items: [
        {
          label: "Undo",
          shortcut: "Ctrl+Z",
          onClick: () => actions.runEditorAction("undo"),
        },
        {
          label: "Redo",
          shortcut: "Ctrl+Shift+Z",
          onClick: () => actions.runEditorAction("redo"),
        },
        { separator: true },
        {
          label: "Find",
          shortcut: "Ctrl+F",
          onClick: () => actions.runEditorAction("actions.find"),
        },
        {
          label: "Replace",
          shortcut: "Ctrl+H",
          onClick: () =>
            actions.runEditorAction("editor.action.startFindReplaceAction"),
        },
        { separator: true },
        {
          label: "Select All",
          shortcut: "Ctrl+A",
          onClick: () => actions.runEditorAction("editor.action.selectAll"),
        },
      ],
    },
    {
      label: "View",
      items: [
        {
          label: "Zoom In",
          shortcut: "Ctrl+=",
          onClick: () => actions.setZoomPercent((z) => Math.min(300, z + 10)),
        },
        {
          label: "Zoom Out",
          shortcut: "Ctrl+-",
          onClick: () => actions.setZoomPercent((z) => Math.max(50, z - 10)),
        },
        {
          label: "Reset Zoom",
          shortcut: "Ctrl+0",
          onClick: actions.resetZoom,
        },
        { separator: true },
        {
          label: state.showBottomPane ? "Hide Bottom Panel" : "Show Bottom Panel",
          onClick: actions.toggleBottomPane,
        },
        {
          label: state.layoutOrientation === "vertical" ? "Split Side-by-Side" : "Split Stacked",
          onClick: actions.toggleLayoutOrientation,
        },
        { separator: true },
        {
          label: state.showAIPanel ? "Hide Agent" : "Show Agent",
          onClick: actions.toggleAIPanel,
        },
        { separator: true },
        { label: "Show Log", onClick: () => actions.showPane("log") },
        {
          label: "Show Output",
          onClick: () => actions.showPane("output"),
          disabled: !state.hasOutput,
        },
        {
          label: "Show Dataset",
          onClick: () => actions.showPane("dataset"),
          disabled: !state.activeDataset,
        },
      ],
    },
    {
      label: "Run",
      items: [
        {
          label: state.running ? "Running…" : "Submit",
          shortcut: "F3",
          onClick: actions.submit,
          disabled: state.running,
        },
        {
          label: "Run Project",
          onClick: actions.runProject,
          disabled: state.running || state.projectProgramCount === 0,
        },
        {
          label: "Cancel",
          shortcut: "F4",
          onClick: actions.cancel,
          disabled: !state.running,
        },
        { separator: true },
        { label: "Clear Log", onClick: actions.clearLog, disabled: !state.hasLog },
        { label: "Clear Output", onClick: actions.clearOutputs, disabled: !state.hasOutput },
      ],
    },
    {
      label: "Help",
      items: [
        { label: "Keyboard Shortcuts…", onClick: actions.showShortcuts },
        { separator: true },
        { label: "About PAS…", onClick: actions.showAbout },
      ],
    },
  ];
}
