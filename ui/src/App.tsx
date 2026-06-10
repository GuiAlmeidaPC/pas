import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Editor, { OnMount } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";

import { LibraryTree } from "./LibraryTree";
import { ProjectTree } from "./ProjectTree";
import { DatasetViewer } from "./DatasetViewer";
import { Splitter } from "./Splitter";
import { EditorTabs } from "./Tabs";
import { StatusBar } from "./StatusBar";
import { MenuBar, type MenuDef } from "./MenuBar";
import { Modal } from "./Modal";
import { AIChatPanel } from "./AIChatPanel";
import { applyPatch, type EditFileSnapshot, type ProposedEdit, type ResolvedEdit } from "./ai/editProtocol";
import { DiffReviewModal } from "./ai/DiffReviewModal";
import { createEditorMount, type AiTrigger } from "./editorSetup";
import { buildMenus } from "./menus";
import { LogView, OutputView } from "./panes";
import {
  DEFAULT_UNSAVED_PROGRAM_PATH,
  buildProjectConfig,
  createUnsavedProjectWorkspace,
  defaultAgentPanelOpen,
  isProjectOpen,
  mergeProgramPaths,
} from "./projectWorkspace";
import { STARTER_PROGRAM } from "./starterProgram";
import { makeTab, basename, type Tab } from "./tabState";
import { useRunner } from "./useRunner";
import { useZoom } from "./useZoom";
import { buildWorkspaceContext, useSchemaContext } from "./workspaceContext";
import type {
  DatasetRef,
  Library,
  ProjectConfig,
  TabConfig,
} from "./types";

const INITIAL_WORKSPACE = createUnsavedProjectWorkspace(STARTER_PROGRAM);

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
  const {
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
  } = useRunner(editorRef, monacoRef);
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
  const { zoomPercent, setZoomPercent } = useZoom();

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
  const [aiTrigger, setAiTrigger] = useState<AiTrigger | null>(null);
  const [diffReview, setDiffReview] = useState<
    | { edit: ProposedEdit; resolved: ResolvedEdit }
    | null
  >(null);
  const [appliedEditPaths, setAppliedEditPaths] = useState<Set<string>>(new Set());

  const schemaContext = useSchemaContext(refreshToken);

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

  const workspaceContext = useMemo(
    () =>
      buildWorkspaceContext({
        activeTab,
        tabs,
        projectName,
        projectPrograms,
        schemaContext,
        log,
      }),
    [activeTab, tabs, projectName, projectPrograms, schemaContext, log],
  );

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

  const runActiveProject = useCallback(() => {
    setShowBottomPane(true);
    void runProject(projectPrograms);
  }, [runProject, projectPrograms]);

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
  }, [setLog]);

  const openFile = useCallback(async () => {
    const path = await invoke<string | null>("pick_pas_file");
    if (!path) return;
    await openFromPath(path);
  }, [openFromPath]);

  // ── project program registry ────────────────────────────────────────
  const addProgramToProject = useCallback(async () => {
    const path = await invoke<string | null>("pick_pas_file");
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
  }, [setLog]);

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
      const activeTabs = overrideTabs ?? tabsRef.current;
      const activePrograms = overridePrograms ?? projectProgramsRef.current;

      const openTabPaths = activeTabs
        .filter((t) => t.path)
        .map((t) => t.path!);
      const programPaths = mergeProgramPaths(activePrograms, openTabPaths);

      // Fetch content for each program (embedded project feature)
      const programs: TabConfig[] = [];
      for (const pPath of programPaths) {
        let content: string | undefined;
        const tab = activeTabs.find((t) => t.path === pPath);
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

      const project: ProjectConfig = buildProjectConfig({
        name,
        libraries: libs,
        programs,
        openTabPaths,
        activeTabPath: activeTabs.find((t) => t.id === activeIdRef.current)?.path ?? null,
        layout: {
          sidebar_width: sidebarWRef.current,
          bottom_height: bottomHRef.current,
          bottom_width: bottomWRef.current,
          orientation: layoutOrientationRef.current,
        },
      });
      await invoke("save_project", { path, project });
      setProjectPath(path);
      setProjectName(name);
      setProjectPrograms(programs);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `save project: ${String(e)}` }]);
    }
  }, [setLog]);

  const saveProject = useCallback(() => performSaveProject(false), [performSaveProject]);
  const saveProjectAs = useCallback(() => performSaveProject(true), [performSaveProject]);

  const handleAddToProject = useCallback(async (code: string) => {
    if (!isProjectOpen(projectNameRef.current)) return;

    // 1. Prompt for program name inside the project
    const defaultTitle = "ai_program.pas";
    const name = window.prompt("Enter a name for this AI program in the project:", defaultTitle);
    if (!name) return; // User cancelled
    const filename = name.endsWith(".pas") ? name : `${name}.pas`;

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
  }, [performSaveProject, setLog]);

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
    [performSaveProject, readEditFile, setLog],
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
        path = name.endsWith(".pas") ? name : `${name}.pas`;
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
        const chosen = await invoke<string | null>("pick_save_pas_file", {
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
    const chosen = await invoke<string | null>("pick_save_pas_file", {
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
      bumpRefresh();
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `open project: ${String(e)}` }]);
    }
  }, [bumpRefresh, currentSubmissionRef, setLog, setRunning]);

  // ── editor mount ────────────────────────────────────────────────────
  const handleMount: OnMount = createEditorMount({
    editorRef,
    monacoRef,
    activeIdRef,
    setCursor,
    setActiveSelection,
    submit,
    cancel,
    saveActiveTab,
    newTab,
    openFile,
    closeTab,
    setShowAIPanel,
    setAiTrigger,
  });

  // Reapply PAS language on tab switch (each model needs registration).
  useEffect(() => {
    const monaco = monacoRef.current;
    const editor = editorRef.current;
    if (!monaco || !editor) return;
    const model = editor.getModel();
    if (model) monaco.editor.setModelLanguage(model, "pas");

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
  }, [setPane]);

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
    () =>
      buildMenus(
        {
          newTab,
          openFile,
          saveActiveTab,
          saveActiveTabAs,
          closeActiveTab: () => closeTab(activeIdRef.current),
          newProject,
          openProject,
          saveProject,
          saveProjectAs,
          addProgramToProject,
          runEditorAction,
          setZoomPercent,
          resetZoom: () => setZoomPercent(100),
          toggleBottomPane: () => setShowBottomPane((s) => !s),
          toggleLayoutOrientation: () =>
            setLayoutOrientation((prev) => (prev === "vertical" ? "horizontal" : "vertical")),
          toggleAIPanel: () => setShowAIPanel((s) => !s),
          showPane: (p) => {
            setPane(p);
            setShowBottomPane(true);
          },
          submit,
          runProject: runActiveProject,
          cancel,
          clearLog,
          clearOutputs,
          showShortcuts: () => setShowShortcuts(true),
          showAbout: () => setShowAbout(true),
        },
        {
          hasProject,
          hasOutput,
          hasLog,
          activeDataset,
          running,
          projectProgramCount: projectPrograms.length,
          showBottomPane,
          showAIPanel,
          layoutOrientation,
        },
      ),
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
      setZoomPercent,
      setPane,
      hasOutput,
      hasLog,
      activeDataset,
      running,
      submit,
      runActiveProject,
      cancel,
      clearLog,
      clearOutputs,
      showBottomPane,
      showAIPanel,
      layoutOrientation,
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
            <strong>PAS</strong> — a cross-platform data-wrangling studio.
            Editor on top of Monaco; engine in Rust over DuckDB.
          </p>
          <p className="muted">
            Statistical procedures and proprietary binary dataset interop are
            intentionally out of scope; DATA step and PROC SQL workflows are
            supported.
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
              onRunProject={runActiveProject}
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
                  defaultLanguage="pas"
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
