import { useState } from "react";
import type { TabConfig } from "./types";

interface Props {
  projectName: string | null;
  programs: TabConfig[];
  /// Paths currently open in editor tabs (used to render an indicator).
  openPaths: Set<string>;
  onOpenProgram: (path: string) => void;
  onAddProgram: () => void;
  onRemoveProgram: (path: string) => void;
}

function basename(p: string): string {
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

export function ProjectTree({
  projectName,
  programs,
  openPaths,
  onOpenProgram,
  onAddProgram,
  onRemoveProgram,
}: Props) {
  const [contextFor, setContextFor] = useState<string | null>(null);

  return (
    <div className="tree" onClick={() => setContextFor(null)}>
      <div className="tree-header">
        <span>Project</span>
        <button
          className="tree-action"
          onClick={(e) => {
            e.stopPropagation();
            onAddProgram();
          }}
          title="Add an existing .sas file to the project"
        >
          +
        </button>
      </div>
      <div className="tree-row tree-libref open">
        <span className="caret">▾</span>
        <span className="libname">{projectName ?? "(unsaved project)"}</span>
      </div>
      <div className="tree-children">
        {programs.length === 0 && (
          <div className="tree-empty">
            No programs yet. Click <code>+</code> to add one.
          </div>
        )}
        {programs.map((p) => {
          const isOpen = openPaths.has(p.path);
          const showCtx = contextFor === p.path;
          return (
            <div
              key={p.path}
              className={`tree-row tree-dataset${isOpen ? " open" : ""}`}
              onDoubleClick={() => onOpenProgram(p.path)}
              onContextMenu={(e) => {
                e.preventDefault();
                setContextFor(p.path);
              }}
              title={p.path}
            >
              <span className="dataset-icon">{isOpen ? "●" : "○"}</span>
              <span>{basename(p.path)}</span>
              {showCtx && (
                <div className="ctx-menu" onClick={(e) => e.stopPropagation()}>
                  <button
                    onClick={() => {
                      onOpenProgram(p.path);
                      setContextFor(null);
                    }}
                  >
                    Open
                  </button>
                  <button
                    onClick={() => {
                      onRemoveProgram(p.path);
                      setContextFor(null);
                    }}
                  >
                    Remove from project
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
