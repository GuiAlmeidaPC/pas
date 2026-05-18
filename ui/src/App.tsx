import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Editor, { OnMount } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { LibraryTree } from "./LibraryTree";
import { DatasetViewer } from "./DatasetViewer";
import { Splitter } from "./Splitter";
import { EditorTabs } from "./Tabs";
import { StatusBar } from "./StatusBar";
import { registerSasLanguage } from "./sasLang";
import type {
  DatasetRef,
  EngineEvent,
  LogLine,
  Library,
  ProjectConfig,
  ResultBlock,
  SubmitEventPayload,
} from "./types";

const STARTER_PROGRAM = `/* PAS v0.6 — multi-tab editor & project files
   F3 submits (selection or whole buffer). Ctrl+S saves. Ctrl+N new tab.
*/

proc sql;
    create table demo as
        select 'Ada' as name, 1815 as born union all
        select 'Alan',         1912 union all
        select 'Grace',        1906;
quit;

data ages;
    set demo;
    age = 2026 - born;
run;

proc sql; select * from ages order by age; quit;
`;

interface Tab {
  id: string;
  path: string | null;
  title: string;
  content: string;
  saved_content: string; // baseline for dirty detection
}

type Pane = "log" | "output" | "dataset";

function makeTab(opts: { id?: string; path?: string | null; title?: string; content: string }): Tab {
  const id =
    opts.id ?? (typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `tab-${Date.now()}-${Math.random()}`);
  return {
    id,
    path: opts.path ?? null,
    title: opts.title ?? "untitled.sas",
    content: opts.content,
    saved_content: opts.content,
  };
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

export default function App() {
  const editorRef = useRef<Parameters<OnMount>[0] | null>(null);
  const monacoRef = useRef<Parameters<OnMount>[1] | null>(null);
  const [tabs, setTabs] = useState<Tab[]>(() => [makeTab({ content: STARTER_PROGRAM })]);
  const [activeId, setActiveId] = useState<string>(() => tabs[0].id);
  const [log, setLog] = useState<LogLine[]>([]);
  const [outputs, setOutputs] = useState<ResultBlock[]>([]);
  const [pane, setPane] = useState<Pane>("log");
  const [running, setRunning] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const [activeDataset, setActiveDataset] = useState<DatasetRef | null>(null);
  const [sidebarW, setSidebarW] = useState(240);
  const [bottomH, setBottomH] = useState(320);
  const [cursor, setCursor] = useState<{ line: number; col: number } | null>(null);
  const [projectPath, setProjectPath] = useState<string | null>(null);
  const [projectName, setProjectName] = useState<string | null>(null);
  const [libCount, setLibCount] = useState(1);

  const currentSubmissionRef = useRef<string | null>(null);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;

  const activeTab = tabs.find((t) => t.id === activeId) ?? null;

  // Engine events.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    (async () => {
      const fn = await listen<SubmitEventPayload>("pas://event", (msg) => {
        const { submission_id, event } = msg.payload;
        if (submission_id !== currentSubmissionRef.current) return;
        applyEvent(event, setLog, setOutputs, setRunning, setPane, () =>
          setRefreshToken((t) => t + 1),
        );
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
  }, []);

  // Track library count for status bar.
  useEffect(() => {
    (async () => {
      try {
        const libs = await invoke<Library[]>("list_libraries");
        setLibCount(libs.length);
      } catch {
        /* ignore */
      }
    })();
  }, [refreshToken]);

  // ── submission control ──────────────────────────────────────────────
  const cancel = useCallback(async () => {
    try { await invoke("cancel"); }
    catch (e) {
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

    const id = typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `sub-${Date.now()}-${Math.random()}`;
    currentSubmissionRef.current = id;
    setLog([]);
    setOutputs([]);
    setRunning(true);
    setPane("log");
    // Clear any prior error markers before the new submission.
    {
      const monaco = monacoRef.current;
      const editor = editorRef.current;
      if (monaco && editor) {
        const model = editor.getModel();
        if (model) monaco.editor.setModelMarkers(model, "pas", []);
      }
    }
    try {
      await invoke<string>("submit", { program, submissionId: id });
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `submit failed: ${String(e)}` }]);
      setRunning(false);
    }
  }, []);

  // ── tab management ───────────────────────────────────────────────────
  const newTab = useCallback(() => {
    setTabs((prev) => {
      const t = makeTab({ content: "" });
      setActiveId(t.id);
      return [...prev, t];
    });
  }, []);

  const updateTabContent = useCallback((id: string, content: string) => {
    setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, content } : t)));
  }, []);

  const closeTab = useCallback((id: string) => {
    setTabs((prev) => {
      const tab = prev.find((t) => t.id === id);
      if (tab && tab.content !== tab.saved_content) {
        if (!window.confirm(`Discard unsaved changes to ${tab.title}?`)) return prev;
      }
      const idx = prev.findIndex((t) => t.id === id);
      const next = prev.filter((t) => t.id !== id);
      if (next.length === 0) {
        const fresh = makeTab({ content: "" });
        setActiveId(fresh.id);
        return [fresh];
      }
      if (id === activeIdRef.current) {
        const sibling = next[Math.max(0, idx - 1)];
        setActiveId(sibling.id);
      }
      return next;
    });
  }, []);

  const openFile = useCallback(async () => {
    const path = await openDialog({
      filters: [{ name: "SAS", extensions: ["sas"] }, { name: "All files", extensions: ["*"] }],
    });
    if (!path || Array.isArray(path)) return;
    try {
      const content = await invoke<string>("read_file", { path });
      // Reuse tab if same path already open.
      const existing = tabsRef.current.find((t) => t.path === path);
      if (existing) {
        setActiveId(existing.id);
        return;
      }
      const t = makeTab({ path, title: basename(path), content });
      setTabs((prev) => [...prev, t]);
      setActiveId(t.id);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `open: ${String(e)}` }]);
    }
  }, []);

  const saveActiveTab = useCallback(async () => {
    const tab = tabsRef.current.find((t) => t.id === activeIdRef.current);
    if (!tab) return;
    let path = tab.path;
    if (!path) {
      const chosen = await saveDialog({
        defaultPath: tab.title,
        filters: [{ name: "SAS", extensions: ["sas"] }],
      });
      if (!chosen) return;
      path = chosen;
    }
    try {
      await invoke("write_file", { path, content: tab.content });
      setTabs((prev) =>
        prev.map((t) =>
          t.id === tab.id ? { ...t, path, title: basename(path!), saved_content: t.content } : t,
        ),
      );
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `save: ${String(e)}` }]);
    }
  }, []);

  // ── project operations ──────────────────────────────────────────────
  const newProject = useCallback(() => {
    setProjectPath(null);
    setProjectName(null);
  }, []);

  const openProject = useCallback(async () => {
    const path = await openDialog({
      filters: [{ name: "PAS Project", extensions: ["pas.json", "json"] }],
    });
    if (!path || Array.isArray(path)) return;
    try {
      const project = await invoke<ProjectConfig>("read_project", { path });
      // Apply libnames (logs to current session).
      if (project.libnames.length > 0) {
        const id = crypto.randomUUID();
        currentSubmissionRef.current = id;
        setRunning(true);
        setLog([]);
        await invoke("apply_project_libnames", { libnames: project.libnames });
      }
      // Open each tab file.
      const newTabs: Tab[] = [];
      for (const t of project.open_tabs) {
        try {
          const content = await invoke<string>("read_file", { path: t.path });
          newTabs.push(makeTab({ path: t.path, title: basename(t.path), content }));
        } catch (e) {
          console.error("failed to open project tab", t.path, e);
        }
      }
      if (newTabs.length === 0) newTabs.push(makeTab({ content: "" }));
      setTabs(newTabs);
      const wantedActive = newTabs.find((x) => x.path === project.active_tab);
      setActiveId(wantedActive?.id ?? newTabs[0].id);
      if (project.layout.sidebar_width) setSidebarW(project.layout.sidebar_width);
      if (project.layout.bottom_height) setBottomH(project.layout.bottom_height);
      setProjectPath(path);
      setProjectName(project.name);
      setRefreshToken((t) => t + 1);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `open project: ${String(e)}` }]);
    }
  }, []);

  const saveProject = useCallback(async () => {
    let path = projectPath;
    let name = projectName;
    if (!path) {
      const chosen = await saveDialog({
        defaultPath: name ? `${name}.pas.json` : "project.pas.json",
        filters: [{ name: "PAS Project", extensions: ["pas.json", "json"] }],
      });
      if (!chosen) return;
      path = chosen;
      name = basename(chosen).replace(/\.pas\.json$|\.json$/, "");
    }
    try {
      const libs = await invoke<Library[]>("list_libraries");
      const project: ProjectConfig = {
        version: 1,
        name: name ?? "project",
        libnames: libs
          .filter((l) => l.kind !== "memory")
          .map((l) => ({
            name: l.name,
            kind: l.kind,
            path: l.path,
            format: l.format ?? null,
          })),
        open_tabs: tabsRef.current.filter((t) => t.path).map((t) => ({ path: t.path! })),
        active_tab: tabsRef.current.find((t) => t.id === activeIdRef.current)?.path ?? null,
        layout: { sidebar_width: sidebarW, bottom_height: bottomH },
      };
      await invoke("save_project", { path, project });
      setProjectPath(path);
      setProjectName(name);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `save project: ${String(e)}` }]);
    }
  }, [projectPath, projectName, sidebarW, bottomH]);

  // ── editor mount ────────────────────────────────────────────────────
  const handleMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    registerSasLanguage(monaco);
    if (editor.getModel()) monaco.editor.setModelLanguage(editor.getModel()!, "sas");
    editor.onDidChangeCursorPosition((e) =>
      setCursor({ line: e.position.lineNumber, col: e.position.column }),
    );

    editor.addCommand(monaco.KeyCode.F3, () => submit());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => submit());
    editor.addCommand(monaco.KeyCode.F4, () => cancel());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => saveActiveTab());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyN, () => newTab());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyO, () => openFile());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW, () =>
      closeTab(activeIdRef.current),
    );
  };

  // Reapply SAS language on tab switch (each model needs registration).
  useEffect(() => {
    const monaco = monacoRef.current;
    const editor = editorRef.current;
    if (!monaco || !editor) return;
    const model = editor.getModel();
    if (model) monaco.editor.setModelLanguage(model, "sas");
  }, [activeId]);

  const openDataset = useCallback((ds: DatasetRef) => {
    setActiveDataset(ds);
    setPane("dataset");
  }, []);

  const tabMeta = useMemo(
    () =>
      tabs.map((t) => ({
        id: t.id,
        title: t.title,
        path: t.path,
        dirty: t.content !== t.saved_content,
      })),
    [tabs],
  );

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">PAS</span>
        <span className="muted">v0.6</span>
        <div className="topbar-divider" />
        <button onClick={newTab} title="New tab (Ctrl+N)">New</button>
        <button onClick={openFile} title="Open file (Ctrl+O)">Open</button>
        <button onClick={saveActiveTab} title="Save (Ctrl+S)">Save</button>
        <div className="topbar-divider" />
        <button onClick={newProject} title="New project">Project: New</button>
        <button onClick={openProject} title="Open project">Open</button>
        <button onClick={saveProject} title="Save project">Save</button>
        <div className="spacer" />
        <button onClick={submit} disabled={running} className="primary">
          {running ? "Running…" : "Submit (F3)"}
        </button>
        <button
          onClick={cancel}
          disabled={!running}
          className="cancel-btn"
          title="Cancel (F4)"
        >
          Cancel (F4)
        </button>
      </header>

      <main
        className="main"
        style={{ gridTemplateColumns: `${sidebarW}px 4px 1fr` }}
      >
        <aside className="sidebar">
          <LibraryTree refreshToken={refreshToken} onOpenDataset={openDataset} />
        </aside>

        <Splitter
          direction="horizontal"
          onResize={(d) => setSidebarW((w) => Math.max(140, Math.min(600, w + d)))}
        />

        <section
          className="workspace"
          style={{ gridTemplateRows: `auto 1fr 4px ${bottomH}px` }}
        >
          <EditorTabs
            tabs={tabMeta}
            activeId={activeId}
            onSelect={setActiveId}
            onClose={closeTab}
            onNew={newTab}
          />

          <section className="editor-pane">
            {activeTab ? (
              <Editor
                height="100%"
                path={activeTab.id}
                defaultLanguage="sas"
                theme="vs-dark"
                value={activeTab.content}
                onChange={(v) => updateTabContent(activeTab.id, v ?? "")}
                onMount={handleMount}
                options={{
                  fontFamily: "JetBrains Mono, monospace",
                  fontSize: 13,
                  minimap: { enabled: false },
                  renderWhitespace: "selection",
                  tabSize: 4,
                }}
              />
            ) : null}
          </section>

          <Splitter
            direction="vertical"
            onResize={(d) => setBottomH((h) => Math.max(120, Math.min(900, h - d)))}
          />

          <section className="bottom-pane">
            <div className="tabs">
              <button
                className={pane === "log" ? "tab active" : "tab"}
                onClick={() => setPane("log")}
              >
                Log
              </button>
              <button
                className={pane === "output" ? "tab active" : "tab"}
                onClick={() => setPane("output")}
              >
                Output {outputs.length > 0 ? `(${outputs.length})` : ""}
              </button>
              <button
                className={pane === "dataset" ? "tab active" : "tab"}
                onClick={() => setPane("dataset")}
                disabled={!activeDataset}
              >
                {activeDataset
                  ? `${activeDataset.libref.toUpperCase()}.${activeDataset.name}`
                  : "Dataset"}
              </button>
            </div>
            <div className="tab-body">
              {pane === "log" && <LogView lines={log} />}
              {pane === "output" && <OutputView blocks={outputs} />}
              {pane === "dataset" && activeDataset && (
                <DatasetViewer
                  key={`${activeDataset.libref}.${activeDataset.name}`}
                  ds={activeDataset}
                />
              )}
              {pane === "dataset" && !activeDataset && (
                <div className="empty">Double-click a dataset in the library tree to open it.</div>
              )}
            </div>
          </section>
        </section>
      </main>

      <StatusBar
        engineState={running ? "running" : "idle"}
        activeFile={activeTab?.path ?? null}
        cursor={cursor}
        libraryCount={libCount}
        projectName={projectName}
      />
    </div>
  );
}

function applyEvent(
  event: EngineEvent,
  setLog: React.Dispatch<React.SetStateAction<LogLine[]>>,
  setOutputs: React.Dispatch<React.SetStateAction<ResultBlock[]>>,
  setRunning: React.Dispatch<React.SetStateAction<boolean>>,
  setPane: React.Dispatch<React.SetStateAction<Pane>>,
  bumpRefresh: () => void,
) {
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
}

function LogView({ lines }: { lines: LogLine[] }) {
  if (lines.length === 0) {
    return <div className="empty">Submit a program with F3 to see log output here.</div>;
  }
  return (
    <pre className="log">
      {lines.map((line, i) => (
        <div key={i} className={`log-line log-${line.level}`}>
          {line.text}
        </div>
      ))}
    </pre>
  );
}

function OutputView({ blocks }: { blocks: ResultBlock[] }) {
  if (blocks.length === 0) return <div className="empty">No output yet.</div>;
  return (
    <div className="output">
      {blocks.map((block, i) => (
        <BlockTable key={i} block={block} index={i} />
      ))}
    </div>
  );
}

function BlockTable({ block, index }: { block: ResultBlock; index: number }) {
  return (
    <div className="block">
      <div className="block-header">
        Result #{index + 1} — {block.rows.length} row(s)
        {block.truncated ? " (truncated)" : ""}
      </div>
      <div className="grid-scroll">
        <table className="grid">
          <thead>
            <tr>
              {block.columns.map((c) => (
                <th key={c.name}>
                  <div className="col-name">{c.name}</div>
                  <div className="col-type">{c.ty}</div>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {block.rows.map((row, ri) => (
              <tr key={ri}>
                {row.map((cell, ci) => (
                  <td key={ci} className={cell === null ? "null" : ""}>
                    {cell === null ? "·" : String(cell)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
