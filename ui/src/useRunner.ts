import { useCallback, useEffect, useRef, useState } from "react";
import type { OnMount } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

import type {
  EngineEvent,
  LogLine,
  ResultBlock,
  SubmitEventPayload,
  TabConfig,
} from "./types";

export type Pane = "log" | "output" | "dataset";

export type EditorRef = React.MutableRefObject<Parameters<OnMount>[0] | null>;
export type MonacoRef = React.MutableRefObject<Parameters<OnMount>[1] | null>;

function newSubmissionId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `sub-${Date.now()}-${Math.random()}`;
}

/**
 * Owns the run lifecycle: log/output state, the engine event subscription,
 * and the submit / run-project / cancel commands. The Monaco refs are used
 * to read the submitted selection and to pin error markers.
 */
export function useRunner(editorRef: EditorRef, monacoRef: MonacoRef) {
  const [log, setLog] = useState<LogLine[]>([]);
  const [outputs, setOutputs] = useState<ResultBlock[]>([]);
  const [pane, setPane] = useState<Pane>("log");
  const [running, setRunning] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const currentSubmissionRef = useRef<string | null>(null);

  const bumpRefresh = useCallback(() => setRefreshToken((t) => t + 1), []);

  const applyEvent = useCallback(
    (event: EngineEvent) => {
      switch (event.kind) {
        case "source":
          setLog((p) => [...p, { level: "source", text: event.text }]);
          break;
        case "note":
          setLog((p) => [...p, { level: "note", text: `NOTE: ${event.text}` }]);
          break;
        case "warning":
          setLog((p) => [...p, { level: "warning", text: `WARNING: ${event.text}` }]);
          break;
        case "error":
          setLog((p) => [...p, { level: "error", text: `ERROR: ${event.text}` }]);
          break;
        case "output":
          setOutputs((p) => [...p, event.block]);
          setPane("output");
          break;
        case "done":
          setRunning(false);
          bumpRefresh();
          break;
      }
    },
    [bumpRefresh],
  );

  // Engine events.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    (async () => {
      const fn = await listen<SubmitEventPayload>("pas://event", (msg) => {
        const { submission_id, event } = msg.payload;
        if (submission_id !== currentSubmissionRef.current) return;
        applyEvent(event);
        // Pin parse-error markers on the editor.
        if (event.kind === "error" && event.source_span) {
          const monaco = monacoRef.current;
          const editor = editorRef.current;
          if (monaco && editor) {
            const model = editor.getModel();
            if (model) {
              monaco.editor.setModelMarkers(model, "pas", [
                {
                  startLineNumber: event.source_span.start_line,
                  startColumn: event.source_span.start_col,
                  endLineNumber: event.source_span.end_line,
                  endColumn: event.source_span.end_col,
                  message: event.text,
                  severity: monaco.MarkerSeverity.Error,
                },
              ]);
            }
          }
        }
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const cancel = useCallback(async () => {
    try {
      await invoke("cancel");
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `cancel failed: ${String(e)}` }]);
    }
  }, []);

  const submit = useCallback(async () => {
    const editor = editorRef.current;
    if (!editor) return;
    const model = editor.getModel();
    if (!model) return;
    const selection = editor.getSelection();
    const program =
      selection && !selection.isEmpty() ? model.getValueInRange(selection) : model.getValue();
    if (!program.trim()) return;

    const id = newSubmissionId();
    currentSubmissionRef.current = id;
    setLog([]);
    setOutputs([]);
    setRunning(true);
    setPane("log");
    // Clear any prior error markers before the new submission.
    const monaco = monacoRef.current;
    if (monaco) {
      monaco.editor.setModelMarkers(model, "pas", []);
    }
    try {
      await invoke<string>("submit", { program, submissionId: id });
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `submit failed: ${String(e)}` }]);
      setRunning(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runProject = useCallback(async (programs: TabConfig[]) => {
    if (programs.length === 0) return;

    const id = newSubmissionId();
    currentSubmissionRef.current = id;
    setLog([]);
    setOutputs([]);
    setRunning(true);
    setPane("log");

    try {
      await invoke("submit_files", { programs, submissionId: id });
    } catch (e) {
      setLog((p) => [
        ...p,
        { level: "error", text: `run project failed: ${String(e)}` },
      ]);
      setRunning(false);
    }
  }, []);

  const clearLog = useCallback(() => setLog([]), []);
  const clearOutputs = useCallback(() => setOutputs([]), []);

  return {
    log,
    setLog,
    outputs,
    pane,
    setPane,
    running,
    setRunning,
    refreshToken,
    bumpRefresh,
    currentSubmissionRef,
    submit,
    cancel,
    runProject,
    clearLog,
    clearOutputs,
  };
}
