import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DatasetInfo, DatasetRef, Library } from "./types";

interface Props {
  refreshToken: number;
  onOpenDataset: (ref: DatasetRef) => void;
}

export function LibraryTree({ refreshToken, onOpenDataset }: Props) {
  const [libs, setLibs] = useState<Library[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set(["work"]));
  const [datasets, setDatasets] = useState<Record<string, DatasetInfo[]>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  const loadLibs = useCallback(async () => {
    try {
      const list = await invoke<Library[]>("list_libraries");
      setLibs(list);
    } catch (e) {
      console.error("list_libraries failed", e);
    }
  }, []);

  const loadDatasets = useCallback(async (libref: string) => {
    try {
      const list = await invoke<DatasetInfo[]>("list_datasets", { libref });
      setDatasets((p) => ({ ...p, [libref]: list }));
      setErrors((p) => {
        const n = { ...p };
        delete n[libref];
        return n;
      });
    } catch (e) {
      setErrors((p) => ({ ...p, [libref]: String(e) }));
    }
  }, []);

  // Initial + refresh on token bump.
  useEffect(() => {
    loadLibs();
  }, [loadLibs, refreshToken]);

  // Refresh datasets of expanded libraries when token changes.
  useEffect(() => {
    for (const name of expanded) loadDatasets(name);
  }, [loadDatasets, expanded, refreshToken]);

  const toggle = (name: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else {
        next.add(name);
        loadDatasets(name);
      }
      return next;
    });
  };

  return (
    <div className="tree">
      <div className="tree-header">Libraries</div>
      {libs.length === 0 && <div className="tree-empty">No libraries yet.</div>}
      {libs.map((lib) => {
        const isOpen = expanded.has(lib.name);
        const items = datasets[lib.name] ?? [];
        const err = errors[lib.name];
        return (
          <div key={lib.name} className="tree-lib">
            <div
              className={`tree-row tree-libref ${isOpen ? "open" : ""}`}
              onClick={() => toggle(lib.name)}
              title={lib.path || lib.kind}
            >
              <span className="caret">{isOpen ? "▾" : "▸"}</span>
              <span className="libname">{lib.name.toUpperCase()}</span>
              <span className="kind">{lib.kind}</span>
            </div>
            {isOpen && (
              <div className="tree-children">
                {err && <div className="tree-error">{err}</div>}
                {!err && items.length === 0 && (
                  <div className="tree-empty">(empty)</div>
                )}
                {items.map((ds) => (
                  <div
                    key={ds.name}
                    className="tree-row tree-dataset"
                    onDoubleClick={() =>
                      onOpenDataset({ libref: lib.name, name: ds.name })
                    }
                    title="Double-click to open"
                  >
                    <span className="dataset-icon">▦</span>
                    {ds.name}
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
