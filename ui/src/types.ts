export type Value = null | boolean | number | string;

export interface Column {
  name: string;
  ty: string;
}

export interface ResultBlock {
  columns: Column[];
  rows: Value[][];
  truncated: boolean;
}

export type EngineEvent =
  | { kind: "source"; text: string }
  | { kind: "note"; text: string }
  | { kind: "warning"; text: string }
  | { kind: "error"; text: string; source_line: number | null }
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
}

export interface Layout {
  sidebar_width?: number | null;
  bottom_height?: number | null;
}

export interface ProjectConfig {
  version: number;
  name: string;
  libnames: ProjectLibname[];
  open_tabs: TabConfig[];
  active_tab?: string | null;
  layout: Layout;
}
