export type Value = null | boolean | number | string;

export interface Column {
  name: string;
  ty: string;
}

export interface ResultBlock {
  title?: string;
  columns: Column[];
  rows: Value[][];
  truncated: boolean;
}

export type EngineEvent =
  | { kind: "source"; text: string }
  | { kind: "note"; text: string }
  | { kind: "warning"; text: string }
  | { kind: "error"; text: string; source_span: SourceSpan | null }
  | { kind: "output"; block: ResultBlock }
  | { kind: "done" };

export interface SubmitEventPayload {
  submission_id: string;
  event: EngineEvent;
}

export interface LogLine {
  level: "source" | "note" | "warning" | "error";
  text: string;
}

export interface SourceSpan {
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
}

export interface Library {
  name: string;
  kind: "memory" | "duckdb" | "dir";
  path: string;
  format: "parquet" | "csv" | null;
}

export interface DatasetInfo {
  libref: string;
  name: string;
  rows: number | null;
}

export interface ColumnInfo {
  name: string;
  ty: string;
}

export interface DatasetPage {
  columns: Column[];
  rows: Value[][];
  total_rows: number;
}

export interface DatasetRef {
  libref: string;
  name: string;
}

export interface ProjectLibname {
  name: string;
  kind: "memory" | "duckdb" | "dir";
  path: string;
  format?: "parquet" | "csv" | null;
}

export interface TabConfig {
  path: string;
  content?: string;
}

export interface Layout {
  sidebar_width?: number | null;
  bottom_height?: number | null;
  bottom_width?: number | null;
  orientation?: "vertical" | "horizontal" | null;
}

export interface ProjectConfig {
  version: number;
  name: string;
  libnames: ProjectLibname[];
  /// Files belonging to the project (regardless of whether they're open).
  programs: TabConfig[];
  /// Snapshot of currently-open editor tabs (subset of programs).
  open_tabs: TabConfig[];
  active_tab?: string | null;
  layout: Layout;
}
