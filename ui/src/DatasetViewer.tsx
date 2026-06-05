import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { tableFromIPC, Type, type Table } from "apache-arrow";
import type { DatasetRef } from "./types";

const PAGE_SIZE = 200;
const FILTER_DEBOUNCE_MS = 250;

interface Props {
  ds: DatasetRef;
}

interface PageView {
  columns: { name: string; ty: string; format?: string }[];
  rows: unknown[][];
  totalRows: number;
}

export function DatasetViewer({ ds }: Props) {
  const [page, setPage] = useState<PageView | null>(null);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [filters, setFilters] = useState<Record<string, string>>({});
  const debounceRef = useRef<number | null>(null);
  const requestSeqRef = useRef(0);

  const activeFilters = useMemo(() => {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(filters)) {
      if (v.trim() !== "") out[k] = v;
    }
    return out;
  }, [filters]);

  const load = useCallback(
    async (newOffset: number, currentFilters: Record<string, string>) => {
      const requestSeq = ++requestSeqRef.current;
      setLoading(true);
      setErr(null);
      try {
        const buf = await invoke<ArrayBuffer>("dataset_page_arrow", {
          libref: ds.libref,
          name: ds.name,
          offset: newOffset,
          limit: PAGE_SIZE,
          filters: Object.keys(currentFilters).length > 0 ? currentFilters : null,
        });
        const view = decodePage(buf);
        if (requestSeq !== requestSeqRef.current) return;
        setPage(view);
        setOffset(newOffset);
      } catch (e) {
        if (requestSeq !== requestSeqRef.current) return;
        setErr(String(e));
      } finally {
        if (requestSeq === requestSeqRef.current) setLoading(false);
      }
    },
    [ds.libref, ds.name],
  );

  // Initial load + debounced reload when filters change.
  useEffect(() => {
    if (debounceRef.current) window.clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(() => {
      load(0, activeFilters);
    }, FILTER_DEBOUNCE_MS);
    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
    };
  }, [load, activeFilters]);

  const setFilter = (col: string, v: string) =>
    setFilters((p) => ({ ...p, [col]: v }));
  const clearFilters = () => setFilters({});

  if (err) return <div className="empty">Error: {err}</div>;
  if (!page) return <div className="empty">{loading ? "Loading…" : ""}</div>;

  const total = page.totalRows;
  const last = Math.max(0, total - 1);
  const shownEnd = Math.min(offset + page.rows.length, total);
  const canPrev = offset > 0;
  const canNext = offset + PAGE_SIZE < total;
  const hasFilters = Object.keys(activeFilters).length > 0;

  return (
    <div className="ds-viewer">
      <div className="ds-toolbar">
        <span className="ds-name">
          {ds.libref.toUpperCase()}.{ds.name}
        </span>
        <span className="ds-stats">
          rows {total > 0 ? offset + 1 : 0}–{shownEnd} of {total}
        </span>
        <div className="spacer" />
        <button onClick={() => load(0, activeFilters)} disabled={!canPrev || loading}>
          ⏮
        </button>
        <button
          onClick={() => load(Math.max(0, offset - PAGE_SIZE), activeFilters)}
          disabled={!canPrev || loading}
        >
          ◀
        </button>
        <button onClick={() => load(offset + PAGE_SIZE, activeFilters)} disabled={!canNext || loading}>
          ▶
        </button>
        <button
          onClick={() => load(Math.max(0, Math.floor(last / PAGE_SIZE) * PAGE_SIZE), activeFilters)}
          disabled={!canNext || loading}
        >
          ⏭
        </button>
        <button onClick={() => load(offset, activeFilters)} disabled={loading} title="Refresh">
          ↻
        </button>
        {hasFilters && (
          <button onClick={clearFilters} title="Clear all column filters">
            Clear filters
          </button>
        )}
      </div>
      <div className="grid-scroll ds-grid-scroll">
        <table className="grid">
          <thead>
            <tr>
              <th className="rownum">#</th>
              {page.columns.map((c) => (
                <th key={c.name}>
                  <div className="col-name">{c.name}</div>
                  <div className="col-type">{c.ty}</div>
                </th>
              ))}
            </tr>
            <tr className="filter-row">
              <th className="rownum" />
              {page.columns.map((c) => (
                <th key={c.name}>
                  <input
                    className="filter-input"
                    type="text"
                    value={filters[c.name] ?? ""}
                    placeholder="filter…"
                    onChange={(e) => setFilter(c.name, e.target.value)}
                  />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {page.rows.map((row, ri) => (
              <tr key={ri}>
                <td className="rownum">{offset + ri + 1}</td>
                {row.map((cell, ci) => (
                  <td key={ci} className={cell === null || cell === undefined ? "null" : ""}>
                    {formatCell(cell, page.columns[ci]?.format)}
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

function decodePage(buf: ArrayBuffer): PageView {
  const table: Table = tableFromIPC(new Uint8Array(buf));
  const meta = table.schema.metadata;
  const totalRows = parseInt(meta.get("total_rows") ?? "0", 10);

  const columns = table.schema.fields.map((f) => ({
    name: f.name,
    ty: typeLabel(f.typeId),
    format: f.metadata?.get("pas_format") ?? undefined,
  }));

  const rowCount = table.numRows;
  const colCount = table.numCols;
  const rows: unknown[][] = new Array(rowCount);
  // Pre-materialize columns to typed arrays / vectors for speed.
  const vectors = table.schema.fields.map((_, i) => table.getChildAt(i));
  for (let r = 0; r < rowCount; r++) {
    const row = new Array(colCount);
    for (let c = 0; c < colCount; c++) {
      const v = vectors[c]?.get(r);
      row[c] = v;
    }
    rows[r] = row;
  }
  return { columns, rows, totalRows };
}

function typeLabel(typeId: Type): string {
  // A compact subset is fine for v0.2 — full type rendering can come later.
  switch (typeId) {
    case Type.Int: return "int";
    case Type.Float: return "float";
    case Type.Decimal: return "decimal";
    case Type.Utf8: return "varchar";
    case Type.LargeUtf8: return "varchar";
    case Type.Bool: return "bool";
    case Type.Date: return "date";
    case Type.Time: return "time";
    case Type.Timestamp: return "timestamp";
    case Type.Binary:
    case Type.LargeBinary: return "binary";
    case Type.List: return "list";
    case Type.Struct: return "struct";
    case Type.Null: return "null";
    default: return Type[typeId]?.toLowerCase() ?? "?";
  }
}

function formatCell(v: unknown, sasFormat?: string): string {
  if (v === null || v === undefined) return "·";
  if (sasFormat) {
    const formatted = formatSasCell(v, sasFormat);
    if (formatted !== null) return formatted;
  }
  if (typeof v === "bigint") return v.toString();
  if (v instanceof Date) return v.toISOString();
  if (typeof v === "object") {
    try {
      return JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x));
    } catch {
      return String(v);
    }
  }
  return String(v);
}

function formatSasCell(v: unknown, spec: string): string | null {
  const fmt = parseSasFormat(spec);
  if (!fmt) return null;
  const n = typeof v === "number" ? v : typeof v === "bigint" ? Number(v) : Number(v);
  if (!Number.isFinite(n)) return null;

  switch (fmt.name) {
    case "date":
      return formatSasDate(n);
    case "comma":
      return formatNumber(n, fmt.decimals, false);
    case "dollar":
      return formatNumber(n, fmt.decimals, true);
    case "":
    case "best":
      return fmt.decimals === undefined ? String(v) : n.toFixed(fmt.decimals);
    default:
      return null;
  }
}

function parseSasFormat(spec: string): { name: string; decimals?: number } | null {
  const trimmed = spec.trim().replace(/\.$/, "").toLowerCase();
  const match = /^([a-z]*)(?:\d+)?(?:\.(\d+))?$/.exec(trimmed);
  if (!match) return null;
  return {
    name: match[1],
    decimals: match[2] === undefined ? undefined : parseInt(match[2], 10),
  };
}

function formatSasDate(serial: number): string {
  const base = Date.UTC(1960, 0, 1);
  const d = new Date(base + Math.trunc(serial) * 86_400_000);
  const months = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
  return `${String(d.getUTCDate()).padStart(2, "0")}${months[d.getUTCMonth()]}${d.getUTCFullYear()}`;
}

function formatNumber(n: number, decimals: number | undefined, dollar: boolean): string {
  const fractionDigits = decimals ?? 0;
  const body = new Intl.NumberFormat("en-US", {
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  }).format(n);
  return dollar ? `$${body}` : body;
}
