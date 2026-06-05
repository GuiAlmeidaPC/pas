import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Editor, { OnMount } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

import { LibraryTree } from "./LibraryTree";
import { ProjectTree } from "./ProjectTree";
import { DatasetViewer } from "./DatasetViewer";
import { Splitter } from "./Splitter";
import { EditorTabs } from "./Tabs";
import { StatusBar } from "./StatusBar";
import { MenuBar, type MenuDef } from "./MenuBar";
import { Modal } from "./Modal";
import { registerSasLanguage } from "./sasLang";
import { AIChatPanel } from "./AIChatPanel";
import { applyPatch, type EditFileSnapshot, type ProposedEdit, type ResolvedEdit } from "./ai/editProtocol";
import { DiffReviewModal } from "./ai/DiffReviewModal";
import {
  DEFAULT_UNSAVED_PROGRAM_PATH,
  createUnsavedProjectWorkspace,
  defaultAgentPanelOpen,
  isProjectOpen,
} from "./projectWorkspace";
import type {
  ColumnInfo,
  DatasetInfo,
  DatasetRef,
  EngineEvent,
  LogLine,
  Library,
  ProjectConfig,
  ResultBlock,
  SubmitEventPayload,
  TabConfig,
} from "./types";


const STARTER_PROGRAM = `/* ==================================================================== */
/* PAS (Practical Analytics Studio) — Welcome & Interactive Guide       */
/* ==================================================================== */
/* Press F3 or Cmd+Enter to execute the selected code or the whole file. */
/* Press F4 to cancel execution. Logs and datasets populate below.      */

/* STEP 1: Assigning Libraries (LIBNAME) */
/* The WORK library is always present as a temporary in-memory database. */
/* You can map folders or database files to custom libraries like this:   */
/*                                                                        */
/*   libname mydb  duckdb "path/to/my_database.duckdb";                   */
/*   libname files dir    "path/to/csv_and_parquet_folder" format=csv;    */
/*                                                                        */
/* Once mapped, you can refer to tables as 'mydb.tablename' or            */
/* read/write CSV/Parquet files directly as datasets!                     */

/* STEP 2: Basic PROC SQL (Data Generation) */
proc sql;
    create table raw_employees as
        select 'Jane Doe' as name, 'Sales' as dept, 4500 as salary union all
        select 'John Smith',       'IT',    6200 union all
        select 'Grace Hopper',     'IT',    8500 union all
        select 'Alan Turing',      'Sales', 5200;
quit;

/* STEP 3: Basic DATA Step (Filtering and Derived Columns) */
data high_earners;
    set raw_employees;
    /* Basic arithmetic and string concatenation */
    bonus = salary * 0.10;
    total_comp = salary + bonus;
    
    /* Conditional logic */
    if total_comp > 6000 then status = "High Comp";
    else status = "Standard";
run;

/* STEP 4: Advanced DATA Step (Accumulators, BY-Group Processing, FIRST/LAST) */
proc sort data=high_earners out=sorted_employees;
    by dept descending total_comp;
run;

data dept_summaries;
    set sorted_employees;
    by dept;
    
    /* Keep a running total for each department using RETAIN */
    retain dept_total_comp 0;
    if first.dept then dept_total_comp = 0;
    
    dept_total_comp = dept_total_comp + total_comp;
    
    /* Only output the final consolidated row per department */
    if last.dept;
run;

/* STEP 5: Macro Variables, Functions, & Definitions (Advanced Metaprogramming) */
%macro evaluate_bonuses(title_text, multiplier=0.15);
    %put NOTE: --- Executing macro %upcase(evaluate_bonuses) ---;
    %put NOTE: Title: &title_text;
    %put NOTE: Multiplier parameter value is: &multiplier;
    
    data macro_results;
        set raw_employees;
        /* Using macro parameters inside program statements */
        new_bonus = salary * &multiplier;
        label = "%upcase(&title_text) RESULTS";
    run;
%mend evaluate_bonuses;

/* Invoke the macro with custom positional and keyword parameters */
%evaluate_bonuses(Q2 Compensation Evaluation, multiplier=0.18);

/* STEP 6: Dynamic Macro Binding with CALL SYMPUTX */
data _null_;
    set raw_employees;
    if name = 'Grace Hopper' then do;
        /* Dynamically write to a macro variable at runtime */
        call symputx('top_employee', name);
    end;
run;

/* Print the dynamically bound value to the log pane */
%put NOTE: Top employee resolved dynamically via SYMPUTX: "&top_employee";
`;

const INITIAL_WORKSPACE = createUnsavedProjectWorkspace(STARTER_PROGRAM);

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
  const sidebarRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const [tabs, setTabs] = useState<Tab[]>(() => [
    makeTab({
      path: DEFAULT_UNSAVED_PROGRAM_PATH,
      title: basename(DEFAULT_UNSAVED_PROGRAM_PATH),
      content: STARTER_PROGRAM,
    }),
  ]);
  const [activeId, setActiveId] = useState<string>(() => tabs[0].id);
  const [log, setLog] = useState<LogLine[]>([]);
  const [outputs, setOutputs] = useState<ResultBlock[]>([]);
  const [pane, setPane] = useState<Pane>("log");
  const [running, setRunning] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const [activeDataset, setActiveDataset] = useState<DatasetRef | null>(null);
  const [sidebarW, setSidebarW] = useState(240);
  const [bottomH, setBottomH] = useState<number | null>(null);
  const [bottomW, setBottomW] = useState<number | null>(null);
  const [layoutOrientation, setLayoutOrientation] = useState<"vertical" | "horizontal">("vertical");
  const [showBottomPane, setShowBottomPane] = useState(true);
  const [cursor, setCursor] = useState<{ line: number; col: number } | null>(null);
  const [projectPath, setProjectPath] = useState<string | null>(INITIAL_WORKSPACE.projectPath);
  const [projectName, setProjectName] = useState<string | null>(INITIAL_WORKSPACE.projectName);
  const [projectPrograms, setProjectPrograms] = useState<TabConfig[]>(INITIAL_WORKSPACE.programs);
  const [projectSplit, setProjectSplit] = useState<number | null>(null);
  const [libCount, setLibCount] = useState(1);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [zoomPercent, setZoomPercent] = useState<number>(() => {
    const saved = typeof localStorage !== "undefined" ? localStorage.getItem("pas.zoom") : null;
    const parsed = saved ? parseInt(saved, 10) : NaN;
    return Number.isFinite(parsed) && parsed >= 50 && parsed <= 300 ? parsed : 100;
  });

  const [showAIPanel, setShowAIPanel] = useState<boolean>(() => {
    return defaultAgentPanelOpen(typeof localStorage !== "undefined" ? localStorage : null);
  });
  const [aiPanelW, setAiPanelW] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("pas.ai_panel_width");
      const parsed = saved ? parseInt(saved, 10) : NaN;
      return Number.isFinite(parsed) && parsed >= 200 && parsed <= 800 ? parsed : 320;
    } catch {
      return 320;
    }
  });
  const [activeSelection, setActiveSelection] = useState("");
  const [aiTrigger, setAiTrigger] = useState<{ prompt: string; timestamp: number } | null>(null);
  const [diffReview, setDiffReview] = useState<
    | { edit: ProposedEdit; resolved: ResolvedEdit }
    | null
  >(null);
  const [appliedEditPaths, setAppliedEditPaths] = useState<Set<string>>(new Set());

  const [schemaContext, setSchemaContext] = useState<string>("");

  useEffect(() => {
    let active = true;
    const fetchSchema = async () => {
      try {
        const libs = await invoke<Library[]>("list_libraries");
        const schemas: string[] = [];
        for (const lib of libs) {
          if (!active) return;
          try {
            const datasets = await invoke<DatasetInfo[]>("list_datasets", { libref: lib.name });
            if (datasets.length === 0) {
              schemas.push(`- Library: ${lib.name.toUpperCase()} (${lib.kind})${lib.path ? ` at ${lib.path}` : ""} (empty)`);
              continue;
            }
            schemas.push(`- Library: ${lib.name.toUpperCase()} (${lib.kind})${lib.path ? ` at ${lib.path}` : ""}`);
            for (const ds of datasets) {
              if (!active) return;
              try {
                const cols = await invoke<ColumnInfo[]>("dataset_schema", { libref: lib.name, name: ds.name });
                const colStr = cols.map(c => `      - ${c.name}: ${c.ty}`).join("\n");
                schemas.push(`  - Dataset: ${ds.name} (${ds.rows ?? 0} rows)\n${colStr}`);
              } catch (e) {
                schemas.push(`  - Dataset: ${ds.name} (${ds.rows ?? 0} rows) (Error reading columns: ${String(e)})`);
              }
            }
          } catch (e) {
            schemas.push(`- Library: ${lib.name.toUpperCase()} (${lib.kind}) (Error listing datasets: ${String(e)})`);
          }
        }
        if (active) {
          setSchemaContext(schemas.join("\n"));
        }
      } catch (e) {
        console.error("Failed to build schema context", e);
      }
    };
    fetchSchema();
    return () => {
      active = false;
    };
  }, [refreshToken]);


  const currentSubmissionRef = useRef<string | null>(null);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
  const projectPathRef = useRef(projectPath);
  projectPathRef.current = projectPath;
  const projectNameRef = useRef(projectName);
  projectNameRef.current = projectName;
  const projectProgramsRef = useRef(projectPrograms);
  projectProgramsRef.current = projectPrograms;
  const sidebarWRef = useRef(sidebarW);
  sidebarWRef.current = sidebarW;
  const bottomHRef = useRef(bottomH);
  bottomHRef.current = bottomH;
  const bottomWRef = useRef(bottomW);
  bottomWRef.current = bottomW;
  const layoutOrientationRef = useRef(layoutOrientation);
  layoutOrientationRef.current = layoutOrientation;

  const activeTab = tabs.find((t) => t.id === activeId) ?? null;
  const hasProject = isProjectOpen(projectName);

  const workspaceContext = useMemo(() => {
    let xmlParts: string[] = [];

    // 1. Active file path
    if (activeTab && activeTab.path) {
      xmlParts.push(`  <active_file path="${activeTab.path}" />`);
    } else {
      xmlParts.push('  <active_file path="untitled.sas" />');
    }

    // 2. Project structure and file contents
    if (projectName) {
      xmlParts.push(`  <active_project name="${projectName}">`);
      if (projectPrograms.length > 0) {
        const programXml = projectPrograms.map(p => {
          const openTab = tabs.find(t => t.path === p.path);
          const code = openTab ? openTab.content : (p.content || "");
          // Escape standard XML chars to ensure robust structural parsing by the LLM
          const escapedCode = code
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;");
          return `    <file path="${p.path}">\n${escapedCode}\n    </file>`;
        }).join("\n");
        xmlParts.push(programXml);
      }
      xmlParts.push("  </active_project>");
    }

    // 3. Database schemas
    if (schemaContext) {
      xmlParts.push("  <database_schema>");
      xmlParts.push(schemaContext.split("\n").map(line => `    ${line}`).join("\n"));
      xmlParts.push("  </database_schema>");
    }

    // 4. Execution Diagnostics
    const diagnostics = log.filter(line => line.level === "error" || line.level === "warning");
    if (diagnostics.length > 0) {
      const recentDiag = diagnostics.slice(-10);
      xmlParts.push("  <execution_diagnostics>");
      recentDiag.forEach(d => {
        // Escape content inside diagnostics to prevent malformed XML tags
        const escapedDiagText = d.text
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;");
        xmlParts.push(`    <diagnostic level="${d.level}">${escapedDiagText}</diagnostic>`);
      });
      xmlParts.push("  </execution_diagnostics>");
    }

    return xmlParts.join("\n");
  }, [activeTab, tabs, projectName, projectPrograms, schemaContext, log]);



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

  // Apply window zoom (VS Code-style Ctrl+= / Ctrl+- / Ctrl+0) and
  // persist across sessions.
  useEffect(() => {
    // The non-standard `zoom` CSS property is honored by WebKit (and
    // therefore by the Tauri webview on macOS / Linux) and Chromium —
    // covers every Tauri target. Cast through `any` because the DOM
    // typings don't declare it.
    (document.body.style as unknown as Record<string, string>).zoom = `${zoomPercent}%`;
    try {
      localStorage.setItem("pas.zoom", String(zoomPercent));
    } catch { /* ignore — private mode */ }
  }, [zoomPercent]);

  useEffect(() => {
    try {
      localStorage.setItem("pas.show_ai_panel", String(showAIPanel));
    } catch { /* ignore */ }
  }, [showAIPanel]);

  useEffect(() => {
    try {
      localStorage.setItem("pas.ai_panel_width", String(aiPanelW));
    } catch { /* ignore */ }
  }, [aiPanelW]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const cmd = e.ctrlKey || e.metaKey;
      if (!cmd) return;
      // Ctrl+= (also Ctrl+Plus on numeric keypad) zooms in.
      if (e.key === "=" || e.key === "+") {
        e.preventDefault();
        setZoomPercent((z) => Math.min(300, z + 10));
      } else if (e.key === "-" || e.key === "_") {
        e.preventDefault();
        setZoomPercent((z) => Math.max(50, z - 10));
      } else if (e.key === "0") {
        e.preventDefault();
        setZoomPercent(100);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
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

  const runProject = useCallback(async () => {
    if (projectPrograms.length === 0) return;

    const id = crypto.randomUUID();
    currentSubmissionRef.current = id;
    setLog([]);
    setOutputs([]);
    setRunning(true);
    setPane("log");
    setShowBottomPane(true);

    try {
      await invoke("submit_files", {
        programs: projectPrograms,
        submissionId: id,
      });
    } catch (e) {
      setLog((p) => [
        ...p,
        { level: "error", text: `run project failed: ${String(e)}` },
      ]);
      setRunning(false);
    }
  }, [projectPrograms]);

  // ── tab management ───────────────────────────────────────────────────
  const newTab = useCallback(() => {
    setTabs((prev) => {
      const t = makeTab({ content: "" });
      setActiveId(t.id);
      return [...prev, t];
    });
  }, []);

  const newTabWithContent = useCallback((code: string) => {
    setTabs((prev) => {
      const t = makeTab({ content: code });
      setActiveId(t.id);
      return [...prev, t];
    });
  }, []);

  const handleInsertCode = useCallback((code: string) => {
    const editor = editorRef.current;
    if (!editor) return;
    const position = editor.getPosition();
    if (position && monacoRef.current) {
      const range = new monacoRef.current.Range(
        position.lineNumber,
        position.column,
        position.lineNumber,
        position.column
      );
      editor.executeEdits("ai-chat", [{
        range,
        text: code,
        forceMoveMarkers: true
      }]);
    }
  }, []);

  const handleReplaceCode = useCallback((code: string) => {
    const editor = editorRef.current;
    if (!editor) return;
    const selection = editor.getSelection();
    if (selection) {
      editor.executeEdits("ai-chat", [{
        range: selection,
        text: code,
        forceMoveMarkers: true
      }]);
    }
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

  /// Open a file from a known path (no dialog). Reuses an existing tab
  /// if one is already open for this path.
  const openFromPath = useCallback(async (path: string, embeddedContent?: string) => {
    try {
      const existing = tabsRef.current.find((t) => t.path === path);
      if (existing) {
        setActiveId(existing.id);
        return;
      }
      const content = embeddedContent ?? (await invoke<string>("read_file", { path }));
      const t = makeTab({ path, title: basename(path), content });
      setTabs((prev) => [...prev, t]);
      setActiveId(t.id);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `open: ${String(e)}` }]);
    }
  }, []);

  const openFile = useCallback(async () => {
    const path = await invoke<string | null>("pick_sas_file");
    if (!path) return;
    await openFromPath(path);
  }, [openFromPath]);

  // ── project program registry ────────────────────────────────────────
  const addProgramToProject = useCallback(async () => {
    const path = await invoke<string | null>("pick_sas_file");
    if (!path) return;
    setProjectPrograms((prev) =>
      prev.some((p) => p.path === path) ? prev : [...prev, { path }],
    );
  }, []);

  const removeProgramFromProject = useCallback((path: string) => {
    setProjectPrograms((prev) => prev.filter((p) => p.path !== path));
  }, []);

  const moveProgram = useCallback((path: string, direction: "up" | "down") => {
    setProjectPrograms((prev) => {
      const idx = prev.findIndex((p) => p.path === path);
      if (idx === -1) return prev;
      const nextIdx = direction === "up" ? idx - 1 : idx + 1;
      if (nextIdx < 0 || nextIdx >= prev.length) return prev;
      const next = [...prev];
      [next[idx], next[nextIdx]] = [next[nextIdx], next[idx]];
      return next;
    });
  }, []);

  const writeTabTo = useCallback(async (tab: Tab, path: string) => {
    try {
      await invoke("write_file", { path, content: tab.content });
      setTabs((prev) =>
        prev.map((t) =>
          t.id === tab.id ? { ...t, path, title: basename(path), saved_content: t.content } : t,
        ),
      );
      if (isProjectOpen(projectNameRef.current)) {
        setProjectPrograms((prev) =>
          prev.some((p) => p.path === path) ? prev : [...prev, { path }],
        );
      }
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `save: ${String(e)}` }]);
    }
  }, []);

  const getDefaultSavePath = (title: string, existingPath: string | null) => {
    if (existingPath) return existingPath;
    if (projectPathRef.current) {
      const parts = projectPathRef.current.split(/[\\/]/);
      parts.pop();
      const dir = parts.join("/");
      return dir ? `${dir}/${title}` : title;
    }
    return title;
  };

  const performSaveProject = useCallback(async (
    forceDialog: boolean,
    overrideTabs?: Tab[],
    overridePrograms?: TabConfig[]
  ) => {
    let path = forceDialog ? null : projectPathRef.current;
    let name = projectNameRef.current;
    if (!path) {
      const defaultPath = projectPathRef.current || (name ? `${name}.pas.json` : "project.pas.json");
      const chosen = await invoke<string | null>("pick_save_project_file", { defaultPath });
      if (!chosen) return;
      path = chosen;
      name = basename(chosen).replace(/\.pas\.json$|\.json$/, "");
    }
    try {
      const libs = await invoke<Library[]>("list_libraries");
      // Merge currently-open tab paths into the program list so saving
      // captures any files the user opened without explicitly adding to
      // the project. Order: existing project programs first, then any
      // open tabs not already in that list.
      const activeTabs = overrideTabs ?? tabsRef.current;
      const activePrograms = overridePrograms ?? projectProgramsRef.current;

      const openTabPaths = activeTabs
        .filter((t) => t.path)
        .map((t) => t.path!);
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

      // Fetch content for each program (embedded project feature)
      const programs: TabConfig[] = [];
      const openTabs = activeTabs;
      for (const pPath of programPaths) {
        let content: string | undefined;
        const tab = openTabs.find((t) => t.path === pPath);
        if (tab) {
          content = tab.content;
        } else {
          try {
            content = await invoke<string>("read_file", { path: pPath });
          } catch (e) {
            console.warn(`Failed to read content to embed for ${pPath}`, e);
          }
        }
        programs.push({ path: pPath, content });
      }

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
        programs,
        open_tabs: openTabPaths.map((p) => ({ path: p })),
        active_tab: activeTabs.find((t) => t.id === activeIdRef.current)?.path ?? null,
        layout: {
          sidebar_width: sidebarWRef.current,
          bottom_height: bottomHRef.current,
          bottom_width: bottomWRef.current,
          orientation: layoutOrientationRef.current,
        },
      };
      await invoke("save_project", { path, project });
      setProjectPath(path);
      setProjectName(name);
      setProjectPrograms(programs);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `save project: ${String(e)}` }]);
    }
  }, []);

  const saveProject = useCallback(() => performSaveProject(false), [performSaveProject]);
  const saveProjectAs = useCallback(() => performSaveProject(true), [performSaveProject]);

  const handleAddToProject = useCallback(async (code: string) => {
    if (!isProjectOpen(projectNameRef.current)) return;

    // 1. Prompt for program name inside the project
    const defaultTitle = "ai_program.sas";
    const name = window.prompt("Enter a name for this AI program in the project:", defaultTitle);
    if (!name) return; // User cancelled
    const filename = name.endsWith(".sas") ? name : `${name}.sas`;

    try {
      // 2. Open a new clean tab in the editor
      const newTab = makeTab({ path: filename, title: basename(filename), content: code });
      newTab.saved_content = code; // Baseline for dirty check
      
      setTabs((prev) => [...prev, newTab]);
      setActiveId(newTab.id);

      // 3. Append the new program virtual path to the project registry
      const updatedPrograms = projectProgramsRef.current.some((p) => p.path === filename)
        ? projectProgramsRef.current
        : [...projectProgramsRef.current, { path: filename, content: code }];
      setProjectPrograms(updatedPrograms);

      // 4. Saved projects auto-persist; unsaved projects stay in memory until
      // the user chooses where to save the project JSON.
      if (projectPathRef.current) {
        await performSaveProject(false, [...tabsRef.current, newTab], updatedPrograms);
      }

      setLog((p) => [...p, { level: "note", text: `NOTE: Program added to project: ${basename(filename)}` }]);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `Failed to add program to project: ${String(e)}` }]);
    }
  }, [performSaveProject]);

  const readEditFile = useCallback(async (path: string): Promise<EditFileSnapshot> => {
    const openTab = tabsRef.current.find((t) => t.path === path);
    if (openTab) {
      return { content: openTab.content, source: "tab" };
    }
    const projectProgram = projectProgramsRef.current.find((p) => p.path === path);
    if (projectProgram?.content !== undefined) {
      return { content: projectProgram.content, source: "project" };
    }
    return { content: await invoke<string>("read_file", { path }), source: "disk" };
  }, []);

  const handleApplyEdit = useCallback(
    async (edit: ProposedEdit, resolved: ResolvedEdit) => {
      if (edit.kind === "error") return;
      if (resolved.status !== "ready") {
        setLog((p) => [...p, { level: "error", text: "AI edit failed: stale edits must be regenerated before applying" }]);
        throw new Error("stale AI edit cannot be applied");
      }
      if (!isProjectOpen(projectNameRef.current)) {
        setLog((p) => [...p, { level: "error", text: "AI edit: open a project first" }]);
        throw new Error("no project open");
      }
      const path = edit.path;
      try {
        let contentToWrite = resolved.after;
        if (edit.kind === "patch") {
          const current = await readEditFile(path);
          const reapplied = applyPatch(current.content, edit.hunks);
          if (!reapplied.ok) {
            throw new Error(`current file changed since review: ${reapplied.error}`);
          }
          contentToWrite = reapplied.value;
        } else if (edit.kind === "replace") {
          const current = await readEditFile(path);
          if (current.content !== resolved.before) {
            throw new Error("current file changed since review; regenerate the AI edit before applying");
          }
        } else if (edit.kind === "create") {
          try {
            await readEditFile(path);
            throw new Error(`${path} already exists. Use mode=\"replace\" or mode=\"patch\" instead.`);
          } catch (e) {
            const message = String(e).toLowerCase();
            if (!message.includes("not found") && !message.includes("no such file") && !message.includes("os error 2")) {
              throw e;
            }
          }
        }

        if (projectPathRef.current) {
          await invoke("write_file", { path, content: contentToWrite });
        }

        // Sync any open tab pointing at this path.
        const matchTab = tabsRef.current.find((t) => t.path === path);
        if (matchTab) {
          setTabs((prev) =>
            prev.map((t) =>
              t.id === matchTab.id
                ? { ...t, content: contentToWrite, saved_content: contentToWrite }
                : t,
            ),
          );
        }

        let updatedTabs = tabsRef.current;
        if (edit.kind === "create") {
          if (matchTab) {
            setActiveId(matchTab.id);
          } else {
            const newTab = makeTab({ path, title: basename(path), content: contentToWrite });
            newTab.saved_content = contentToWrite;
            updatedTabs = [...tabsRef.current, newTab];
            setTabs(updatedTabs);
            setActiveId(newTab.id);
          }
        }
        const updatedPrograms = projectProgramsRef.current.some((p) => p.path === path)
          ? projectProgramsRef.current.map((p) =>
              p.path === path ? { ...p, content: contentToWrite } : p,
            )
          : [...projectProgramsRef.current, { path, content: contentToWrite }];
        setProjectPrograms(updatedPrograms);
        if (projectPathRef.current && edit.kind === "create") {
          await performSaveProject(false, updatedTabs, updatedPrograms);
        }
        setAppliedEditPaths((p) => new Set([...p, path]));
        setLog((p) => [
          ...p,
          { level: "note", text: `NOTE: AI edit applied to ${basename(path)}` },
        ]);
      } catch (e) {
        setLog((p) => [...p, { level: "error", text: `AI edit failed: ${String(e)}` }]);
        throw e;
      }
    },
    [performSaveProject, readEditFile],
  );

  const handleReviewEdit = useCallback(
    (edit: ProposedEdit, resolved: ResolvedEdit) => {
      if (edit.kind === "error") return;
      setDiffReview({ edit, resolved });
    },
    [],
  );

  const reorderPrograms = useCallback((srcIdx: number, destIdx: number) => {
    let updated: TabConfig[] = [];
    setProjectPrograms((prev) => {
      const result = [...prev];
      const [removed] = result.splice(srcIdx, 1);
      result.splice(destIdx, 0, removed);
      updated = result;
      return result;
    });
    if (projectPathRef.current) {
      performSaveProject(false, tabsRef.current, updated);
    }
  }, [performSaveProject]);

  const saveActiveTab = useCallback(async () => {
    const tab = tabsRef.current.find((t) => t.id === activeIdRef.current);
    if (!tab) return;

    if (isProjectOpen(projectNameRef.current)) {
      // If project is open, save directly inside project file
      let path = tab.path;
      if (!path) {
        const name = window.prompt("Enter a name for this program inside the project:", tab.title);
        if (!name) return; // User cancelled
        path = name.endsWith(".sas") ? name : `${name}.sas`;
      }

      const updatedTabs = tabsRef.current.map((t) =>
        t.id === tab.id
          ? { ...t, path, title: basename(path), saved_content: t.content }
          : t,
      );
      setTabs(updatedTabs);

      const updatedPrograms = projectProgramsRef.current.some((p) => p.path === path)
        ? projectProgramsRef.current
        : [...projectProgramsRef.current, { path }];
      setProjectPrograms(updatedPrograms);

      await performSaveProject(false, updatedTabs, updatedPrograms);
    } else {
      // Standard local standalone file saving
      let path = tab.path;
      if (!path) {
        const chosen = await invoke<string | null>("pick_save_sas_file", {
          defaultPath: getDefaultSavePath(tab.title, null),
        });
        if (!chosen) return;
        path = chosen;
      }
      await writeTabTo(tab, path);
    }
  }, [performSaveProject, writeTabTo]);

  const saveActiveTabAs = useCallback(async () => {
    const tab = tabsRef.current.find((t) => t.id === activeIdRef.current);
    if (!tab) return;
    const chosen = await invoke<string | null>("pick_save_sas_file", {
      defaultPath: getDefaultSavePath(tab.title, tab.path),
    });
    if (!chosen) return;
    await writeTabTo(tab, chosen);
  }, [writeTabTo]);

  /// Invoke a Monaco action by ID on the active editor. Returns true if
  /// the action existed (for menu enable/disable hints).
  const runEditorAction = useCallback((id: string) => {
    const editor = editorRef.current;
    if (!editor) return false;
    const action = editor.getAction(id);
    if (action) {
      void action.run();
      return true;
    }
    // Fallback for trigger-only commands (undo / redo / type / paste).
    editor.trigger("menu", id, null);
    return true;
  }, []);

  const clearLog = useCallback(() => setLog([]), []);
  const clearOutputs = useCallback(() => setOutputs([]), []);

  // ── project file operations ──────────────────────────────────────────
  const newProject = useCallback(() => {
    setProjectPath(null);
    setProjectName(INITIAL_WORKSPACE.projectName);
    setProjectPrograms([]);
  }, []);

  const openProject = useCallback(async () => {
    const path = await invoke<string | null>("pick_project_file");
    if (!path) return;
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
      // Project program registry. Fall back to `open_tabs` for older
      // project files that predate the `programs` field.
      const programs: TabConfig[] =
        project.programs && project.programs.length > 0
          ? project.programs
          : project.open_tabs ?? [];
      setProjectPrograms(programs);

      // Open the editor working set.
      const newTabs: Tab[] = [];
      for (const t of project.open_tabs) {
        try {
          const embedded = programs.find((p) => p.path === t.path)?.content;
          const content = embedded ?? (await invoke<string>("read_file", { path: t.path }));
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
      if (project.layout.bottom_width) setBottomW(project.layout.bottom_width);
      if (project.layout.orientation) setLayoutOrientation(project.layout.orientation);
      setProjectPath(path);
      setProjectName(project.name);
      setRefreshToken((t) => t + 1);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `open project: ${String(e)}` }]);
    }
  }, []);



  // ── editor mount ────────────────────────────────────────────────────
  const handleMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    registerSasLanguage(monaco);
    if (editor.getModel()) monaco.editor.setModelLanguage(editor.getModel()!, "sas");
    editor.onDidChangeCursorPosition((e) =>
      setCursor({ line: e.position.lineNumber, col: e.position.column }),
    );
    editor.onDidChangeCursorSelection((e) => {
      const model = editor.getModel();
      if (model) {
        setActiveSelection(model.getValueInRange(e.selection));
      } else {
        setActiveSelection("");
      }
    });

    editor.addCommand(monaco.KeyCode.F3, () => submit());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => submit());
    editor.addCommand(monaco.KeyCode.F4, () => cancel());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => saveActiveTab());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyN, () => newTab());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyO, () => openFile());
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW, () =>
      closeTab(activeIdRef.current),
    );

    editor.addAction({
      id: "ai-explain-code",
      label: "Agent: Explain Selection",
      contextMenuGroupId: "1_modification",
      contextMenuOrder: 1,
      precondition: "editorHasSelection",
      run: (ed) => {
        const selection = ed.getSelection();
        const model = ed.getModel();
        if (selection && model) {
          const text = model.getValueInRange(selection);
          setShowAIPanel(true);
          setAiTrigger({
            prompt: `Explain this code:\n\n\`\`\`sas\n${text}\n\`\`\``,
            timestamp: Date.now(),
          });
        }
      }
    });

    editor.addAction({
      id: "ai-refactor-code",
      label: "Agent: Refactor/Optimize Selection",
      contextMenuGroupId: "1_modification",
      contextMenuOrder: 2,
      precondition: "editorHasSelection",
      run: (ed) => {
        const selection = ed.getSelection();
        const model = ed.getModel();
        if (selection && model) {
          const text = model.getValueInRange(selection);
          setShowAIPanel(true);
          setAiTrigger({
            prompt: `Refactor and optimize this code segment:\n\n\`\`\`sas\n${text}\n\`\`\``,
            timestamp: Date.now(),
          });
        }
      }
    });
  };

  // Reapply SAS language on tab switch (each model needs registration).
  useEffect(() => {
    const monaco = monacoRef.current;
    const editor = editorRef.current;
    if (!monaco || !editor) return;
    const model = editor.getModel();
    if (model) monaco.editor.setModelLanguage(model, "sas");
    
    // Read current selection on tab switch
    const selection = editor.getSelection();
    if (selection && model) {
      setActiveSelection(model.getValueInRange(selection));
    } else {
      setActiveSelection("");
    }
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

  const openTabPaths = useMemo(
    () => new Set(tabs.map((t) => t.path).filter((p): p is string => !!p)),
    [tabs],
  );

  const hasOutput = outputs.length > 0;
  const hasLog = log.length > 0;

  const menus: MenuDef[] = useMemo(
    () => [
      {
        label: "File",
        items: [
          { label: "New Tab", shortcut: "Ctrl+N", onClick: newTab },
          { label: "Open File…", shortcut: "Ctrl+O", onClick: openFile },
          { separator: true },
          { label: "Save", shortcut: "Ctrl+S", onClick: saveActiveTab },
          { label: "Save As…", onClick: saveActiveTabAs },
          ...(hasProject
            ? [{ label: "Save to Standalone SAS File…", onClick: saveActiveTabAs }]
            : []),
          { separator: true },
          {
            label: "Close Tab",
            shortcut: "Ctrl+W",
            onClick: () => closeTab(activeIdRef.current),
          },
        ],
      },
      {
        label: "Project",
        items: [
          { label: "New Project", onClick: newProject },
          { label: "Open Project…", onClick: openProject },
          { label: "Save Project", onClick: saveProject },
          { label: "Save Project As…", onClick: saveProjectAs },
          { separator: true },
          { label: "Add Program to Project…", onClick: addProgramToProject },
        ],
      },
      {
        label: "Edit",
        items: [
          {
            label: "Undo",
            shortcut: "Ctrl+Z",
            onClick: () => runEditorAction("undo"),
          },
          {
            label: "Redo",
            shortcut: "Ctrl+Shift+Z",
            onClick: () => runEditorAction("redo"),
          },
          { separator: true },
          {
            label: "Find",
            shortcut: "Ctrl+F",
            onClick: () => runEditorAction("actions.find"),
          },
          {
            label: "Replace",
            shortcut: "Ctrl+H",
            onClick: () =>
              runEditorAction("editor.action.startFindReplaceAction"),
          },
          { separator: true },
          {
            label: "Select All",
            shortcut: "Ctrl+A",
            onClick: () => runEditorAction("editor.action.selectAll"),
          },
        ],
      },
      {
        label: "View",
        items: [
          {
            label: "Zoom In",
            shortcut: "Ctrl+=",
            onClick: () => setZoomPercent((z) => Math.min(300, z + 10)),
          },
          {
            label: "Zoom Out",
            shortcut: "Ctrl+-",
            onClick: () => setZoomPercent((z) => Math.max(50, z - 10)),
          },
          {
            label: "Reset Zoom",
            shortcut: "Ctrl+0",
            onClick: () => setZoomPercent(100),
          },
          { separator: true },
          {
            label: showBottomPane ? "Hide Bottom Panel" : "Show Bottom Panel",
            onClick: () => setShowBottomPane((s) => !s),
          },
          {
            label: layoutOrientation === "vertical" ? "Split Side-by-Side" : "Split Stacked",
            onClick: () => setLayoutOrientation((prev) => prev === "vertical" ? "horizontal" : "vertical"),
          },
          { separator: true },
          {
            label: showAIPanel ? "Hide Agent" : "Show Agent",
            onClick: () => setShowAIPanel((s) => !s),
          },
          { separator: true },
          { label: "Show Log", onClick: () => { setPane("log"); setShowBottomPane(true); } },
          {
            label: "Show Output",
            onClick: () => { setPane("output"); setShowBottomPane(true); },
            disabled: !hasOutput,
          },
          {
            label: "Show Dataset",
            onClick: () => { setPane("dataset"); setShowBottomPane(true); },
            disabled: !activeDataset,
          },
        ],
      },
      {
        label: "Run",
        items: [
          {
            label: running ? "Running…" : "Submit",
            shortcut: "F3",
            onClick: submit,
            disabled: running,
          },
          {
            label: "Run Project",
            onClick: runProject,
            disabled: running || projectPrograms.length === 0,
          },
          {
            label: "Cancel",
            shortcut: "F4",
            onClick: cancel,
            disabled: !running,
          },
          { separator: true },
          { label: "Clear Log", onClick: clearLog, disabled: !hasLog },
          { label: "Clear Output", onClick: clearOutputs, disabled: !hasOutput },
        ],
      },
      {
        label: "Help",
        items: [
          { label: "Keyboard Shortcuts…", onClick: () => setShowShortcuts(true) },
          { separator: true },
          { label: "About PAS…", onClick: () => setShowAbout(true) },
        ],
      },
    ],
    [
      newTab,
      openFile,
      saveActiveTab,
      saveActiveTabAs,
      closeTab,
      newProject,
      openProject,
      saveProject,
      saveProjectAs,
      addProgramToProject,
      runEditorAction,
      hasOutput,
      hasLog,
      activeDataset,
      running,
      submit,
      runProject,
      cancel,
      clearLog,
      clearOutputs,
      showBottomPane,
      showAIPanel,
      hasProject,
      projectPrograms,
    ],
  );

  return (
    <div className="app">
      <header className="menubar-row">
        <span className="brand">PAS</span>
        <MenuBar menus={menus} />
      </header>
      <div className="toolbar">
        <button
          className="toolbar-btn primary"
          onClick={submit}
          disabled={running}
          title="Submit (F3) — selection if any, else whole buffer"
        >
          <span className="toolbar-icon">▶</span>
          {running ? "Running…" : "Submit"}
        </button>
        <button
          className="toolbar-btn danger"
          onClick={cancel}
          disabled={!running}
          title="Cancel (F4)"
        >
          <span className="toolbar-icon">■</span>
          Cancel
        </button>
        <div className="toolbar-divider" />
        <button
          className="toolbar-btn"
          onClick={saveActiveTab}
          title={hasProject ? "Save to Project (Ctrl+S)" : "Save (Ctrl+S)"}
        >
          <span className="toolbar-icon">💾</span>
          {hasProject ? "Save to Project" : "Save"}
        </button>
        <button
          className="toolbar-btn"
          onClick={openFile}
          title="Open file (Ctrl+O)"
        >
          <span className="toolbar-icon">📂</span>
          Open
        </button>
        <div className="toolbar-divider" />
        <button
          className="toolbar-btn"
          onClick={() => runEditorAction("actions.find")}
          title="Find (Ctrl+F)"
        >
          <span className="toolbar-icon">🔍</span>
          Find
        </button>
        <button
          className="toolbar-btn"
          onClick={() => setShowAIPanel((s) => !s)}
          title={showAIPanel ? "Hide Agent Panel" : "Show Agent Panel"}
          aria-pressed={showAIPanel}
        >
          <span className="toolbar-icon">▣</span>
          Agent Panel
        </button>
      </div>

      {showShortcuts && (
        <Modal title="Keyboard shortcuts" onClose={() => setShowShortcuts(false)}>
          <table className="shortcuts">
            <tbody>
              <tr><th colSpan={2}>Run</th></tr>
              <tr><td>Submit (selection or buffer)</td><td><kbd>F3</kbd> / <kbd>Ctrl+Enter</kbd></td></tr>
              <tr><td>Cancel</td><td><kbd>F4</kbd></td></tr>
              <tr><th colSpan={2}>File</th></tr>
              <tr><td>New tab</td><td><kbd>Ctrl+N</kbd></td></tr>
              <tr><td>Open file</td><td><kbd>Ctrl+O</kbd></td></tr>
              <tr><td>Save</td><td><kbd>Ctrl+S</kbd></td></tr>
              <tr><td>Close tab</td><td><kbd>Ctrl+W</kbd></td></tr>
              <tr><th colSpan={2}>Edit (in editor)</th></tr>
              <tr><td>Undo / Redo</td><td><kbd>Ctrl+Z</kbd> / <kbd>Ctrl+Shift+Z</kbd></td></tr>
              <tr><td>Find / Replace</td><td><kbd>Ctrl+F</kbd> / <kbd>Ctrl+H</kbd></td></tr>
              <tr><td>Select all</td><td><kbd>Ctrl+A</kbd></td></tr>
              <tr><th colSpan={2}>View</th></tr>
              <tr><td>Zoom in / out / reset</td><td><kbd>Ctrl+=</kbd> / <kbd>Ctrl+-</kbd> / <kbd>Ctrl+0</kbd></td></tr>
            </tbody>
          </table>
        </Modal>
      )}
      {showAbout && (
        <Modal title="About PAS" onClose={() => setShowAbout(false)}>
          <p>
            <strong>PAS</strong> — a cross-platform clone of the data-wrangling
            subset of SAS Enterprise Guide. Editor on top of Monaco; engine in
            Rust over DuckDB.
          </p>
          <p className="muted">
            Statistical procedures and <code>.sas7bdat</code> interop are
            intentionally out of scope; everything else from the SAS DATA
            step and PROC SQL is on the menu.
          </p>
        </Modal>
      )}

      <main
        className="main"
        style={{
          gridTemplateColumns: showAIPanel
            ? `${sidebarW}px 4px minmax(0, 1fr) 4px ${aiPanelW}px`
            : `${sidebarW}px 4px minmax(0, 1fr)`,
        }}
      >
        <aside
          ref={sidebarRef}
          className="sidebar"
          style={{
            gridTemplateRows:
              projectSplit !== null
                ? `${projectSplit}px 4px 1fr`
                : "1fr 4px 1fr",
          }}
        >
          <div className="sidebar-section">
            <ProjectTree
              projectName={projectName}
              programs={projectPrograms}
              openPaths={openTabPaths}
              onOpenProgram={openFromPath}
              onAddProgram={addProgramToProject}
              onRemoveProgram={removeProgramFromProject}
              onMoveProgram={moveProgram}
              onReorderPrograms={reorderPrograms}
              onRunProject={runProject}
              running={running}
            />
          </div>
          <Splitter
            direction="vertical"
            onResize={(d) =>
              setProjectSplit((h) => {
                let startH = h;
                if (startH === null && sidebarRef.current) {
                  const firstSection = sidebarRef.current.firstElementChild;
                  if (firstSection) {
                    startH = firstSection.getBoundingClientRect().height;
                  }
                }
                const currentH = startH ?? 260;
                return Math.max(80, Math.min(800, currentH + d));
              })
            }
          />
          <div className="sidebar-section">
            <LibraryTree refreshToken={refreshToken} onOpenDataset={openDataset} />
          </div>
        </aside>

        <Splitter
          direction="horizontal"
          onResize={(d) => setSidebarW((w) => Math.max(140, Math.min(600, w + d)))}
        />

        <section
          ref={workspaceRef}
          className="workspace"
          style={
            layoutOrientation === "horizontal"
              ? {
                  gridTemplateColumns: showBottomPane
                    ? bottomW !== null
                      ? `minmax(0, 1fr) 4px ${bottomW}px`
                      : "minmax(0, 1fr) 4px minmax(0, 1fr)"
                    : "minmax(0, 1fr)",
                  gridTemplateRows: "minmax(0, 1fr)",
                }
              : {
                  gridTemplateRows: showBottomPane
                    ? bottomH !== null
                      ? `minmax(0, 1fr) 4px ${bottomH}px`
                      : "minmax(0, 1fr) 4px minmax(0, 1fr)"
                    : "minmax(0, 1fr)",
                  gridTemplateColumns: "minmax(0, 1fr)",
                }
          }
        >
          <div className="editor-panel">
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

            {!showBottomPane && (
              <button
                className="floating-restore-btn"
                title="Restore Bottom Panel"
                onClick={() => setShowBottomPane(true)}
              >
                {layoutOrientation === "horizontal" ? "◀" : "▲"} Restore Panel
              </button>
            )}
          </div>

          {showBottomPane && (
            <>
              <Splitter
                direction={layoutOrientation === "horizontal" ? "horizontal" : "vertical"}
                onResize={(d) => {
                  if (layoutOrientation === "horizontal") {
                    setBottomW((w) => {
                      let startW = w;
                      if (startW === null && workspaceRef.current) {
                        const bottomPane = workspaceRef.current.querySelector(".bottom-pane");
                        if (bottomPane) {
                          startW = bottomPane.getBoundingClientRect().width;
                        }
                      }
                      const currentW = startW ?? 300;
                      return Math.max(150, Math.min(1200, currentW - d));
                    });
                  } else {
                    setBottomH((h) => {
                      let startH = h;
                      if (startH === null && workspaceRef.current) {
                        const bottomPane = workspaceRef.current.querySelector(".bottom-pane");
                        if (bottomPane) {
                          startH = bottomPane.getBoundingClientRect().height;
                        }
                      }
                      const currentH = startH ?? 180;
                      return Math.max(120, Math.min(900, currentH - d));
                    });
                  }
                }}
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
              <button
                className="layout-toggle-btn layout-right-start"
                title={layoutOrientation === "vertical" ? "Split Side-by-Side (Horizontal)" : "Split Stacked (Vertical)"}
                onClick={() => setLayoutOrientation((prev) => prev === "vertical" ? "horizontal" : "vertical")}
              >
                {layoutOrientation === "vertical" ? "◧ Side-by-Side" : "◰ Stacked"}
              </button>
              <button
                className="layout-toggle-btn"
                title="Collapse Bottom Panel"
                onClick={() => setShowBottomPane(false)}
              >
                {layoutOrientation === "horizontal" ? "▶" : "▼"} Collapse
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
            </>
          )}
        </section>

        {showAIPanel && (
          <>
            <Splitter
              direction="horizontal"
              onResize={(d) => {
                setAiPanelW((w) => {
                  const currentW = w ?? 320;
                  return Math.max(200, Math.min(800, currentW - d));
                });
              }}
            />
            <AIChatPanel
              activeContent={activeTab ? activeTab.content : ""}
              activeSelection={activeSelection}
              onInsertCode={handleInsertCode}
              onReplaceCode={handleReplaceCode}
              onNewTab={newTabWithContent}
              onAddToProject={handleAddToProject}
              readEditFile={readEditFile}
              onApplyEdit={handleApplyEdit}
              onReviewEdit={handleReviewEdit}
              isProjectOpen={hasProject}
              customTrigger={aiTrigger}
              workspaceContext={workspaceContext}
              appliedEditPaths={appliedEditPaths}
            />
          </>
        )}
      </main>

      <StatusBar
        engineState={running ? "running" : "idle"}
        activeFile={activeTab?.path ?? null}
        cursor={cursor}
        libraryCount={libCount}
        projectName={projectName}
        zoomPercent={zoomPercent}
      />
      {diffReview && (
        <DiffReviewModal
          edit={diffReview.edit}
          before={diffReview.resolved.before}
          after={diffReview.resolved.after}
          canAccept={diffReview.resolved.status === "ready"}
          onAccept={() =>
            handleApplyEdit(diffReview.edit, diffReview.resolved)
          }
          onClose={() => setDiffReview(null)}
        />
      )}
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
      {block.title && <div className="block-title">{block.title}</div>}
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
