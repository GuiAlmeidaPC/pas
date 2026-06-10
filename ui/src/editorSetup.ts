import type { OnMount } from "@monaco-editor/react";

import { registerPasLanguage } from "./pasLang";
import type { EditorRef, MonacoRef } from "./useRunner";

export interface AiTrigger {
  prompt: string;
  timestamp: number;
}

export interface EditorMountDeps {
  editorRef: EditorRef;
  monacoRef: MonacoRef;
  activeIdRef: React.MutableRefObject<string>;
  setCursor: (cursor: { line: number; col: number } | null) => void;
  setActiveSelection: (text: string) => void;
  submit: () => void;
  cancel: () => void;
  saveActiveTab: () => void;
  newTab: () => void;
  openFile: () => void;
  closeTab: (id: string) => void;
  setShowAIPanel: (show: boolean) => void;
  setAiTrigger: (trigger: AiTrigger) => void;
}

/**
 * Builds the Monaco onMount handler: registers the PAS language, wires
 * keyboard shortcuts, and installs the Agent context-menu actions.
 */
export function createEditorMount(deps: EditorMountDeps): OnMount {
  return (editor, monaco) => {
    deps.editorRef.current = editor;
    deps.monacoRef.current = monaco;
    registerPasLanguage(monaco);
    if (editor.getModel()) monaco.editor.setModelLanguage(editor.getModel()!, "pas");
    editor.onDidChangeCursorPosition((e) =>
      deps.setCursor({ line: e.position.lineNumber, col: e.position.column }),
    );
    editor.onDidChangeCursorSelection((e) => {
      const model = editor.getModel();
      if (model) {
        deps.setActiveSelection(model.getValueInRange(e.selection));
      } else {
        deps.setActiveSelection("");
      }
    });

    editor.addCommand(monaco.KeyCode.F3, () => deps.submit());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => deps.submit());
    editor.addCommand(monaco.KeyCode.F4, () => deps.cancel());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => deps.saveActiveTab());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyN, () => deps.newTab());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyO, () => deps.openFile());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW, () =>
      deps.closeTab(deps.activeIdRef.current),
    );

    const addAgentSelectionAction = (
      id: string,
      label: string,
      order: number,
      buildPrompt: (text: string) => string,
    ) => {
      editor.addAction({
        id,
        label,
        contextMenuGroupId: "1_modification",
        contextMenuOrder: order,
        precondition: "editorHasSelection",
        run: (ed) => {
          const selection = ed.getSelection();
          const model = ed.getModel();
          if (selection && model) {
            const text = model.getValueInRange(selection);
            deps.setShowAIPanel(true);
            deps.setAiTrigger({ prompt: buildPrompt(text), timestamp: Date.now() });
          }
        },
      });
    };

    addAgentSelectionAction(
      "ai-explain-code",
      "Agent: Explain Selection",
      1,
      (text) => `Explain this code:\n\n\`\`\`pas\n${text}\n\`\`\``,
    );
    addAgentSelectionAction(
      "ai-refactor-code",
      "Agent: Refactor/Optimize Selection",
      2,
      (text) => `Refactor and optimize this code segment:\n\n\`\`\`pas\n${text}\n\`\`\``,
    );
  };
}
