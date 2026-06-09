import { useState } from "react";
import type { TabConfig } from "./types";

interface Props {
  projectName: string | null;
  programs: TabConfig[];
  /// Paths currently open in editor tabs (used to render an indicator).
  openPaths: Set<string>;
  onOpenProgram: (path: string, content?: string) => void;
  onAddProgram: () => void;
  onRemoveProgram: (path: string) => void;
  onMoveProgram: (path: string, direction: "up" | "down") => void;
  onReorderPrograms?: (srcIdx: number, destIdx: number) => void;
  onRunProject: () => void;
  running?: boolean;
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
  onMoveProgram,
  onReorderPrograms,
  onRunProject,
  running,
}: Props) {
  const [contextFor, setContextFor] = useState<string | null>(null);
  const [draggedIndex, setDraggedIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
  const [dragPosition, setDragPosition] = useState<"top" | "bottom" | null>(null);

  const handleDragOver = (e: React.DragEvent<HTMLDivElement>, index: number) => {
    e.preventDefault();
    if (draggedIndex === null || draggedIndex === index) return;

    const rect = e.currentTarget.getBoundingClientRect();
    const relativeY = e.clientY - rect.top;
    const isTopHalf = relativeY < rect.height / 2;

    setDragOverIndex(index);
    setDragPosition(isTopHalf ? "top" : "bottom");
  };

  const handleDrop = (e: React.DragEvent<HTMLDivElement>, index: number) => {
    e.preventDefault();
    if (draggedIndex === null || draggedIndex === index) return;

    const targetIndex = dragPosition === "bottom" ? index + 1 : index;
    let destIdx = targetIndex;
    if (draggedIndex < targetIndex) {
      destIdx = targetIndex - 1;
    }

    if (onReorderPrograms) {
      onReorderPrograms(draggedIndex, destIdx);
    }

    setDraggedIndex(null);
    setDragOverIndex(null);
    setDragPosition(null);
  };

  return (
    <div className="tree" onClick={() => setContextFor(null)}>
      <div className="tree-header">
        <span>Project</span>
        <div className="tree-actions">
          <button
            className="tree-action"
            onClick={(e) => {
              e.stopPropagation();
              onRunProject();
            }}
            disabled={running || programs.length === 0}
            title="Run all programs in order (Process Flow)"
          >
            ▶
          </button>
          <button
            className="tree-action"
            onClick={(e) => {
              e.stopPropagation();
              onAddProgram();
            }}
            title="Add an existing .pas file to the project"
          >
            +
          </button>
        </div>
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
        {programs.map((p, i) => {
          const isOpen = openPaths.has(p.path);
          const showCtx = contextFor === p.path;
          const isDragging = draggedIndex === i;
          const isDragOver = dragOverIndex === i;
          const dragClass = isDragging
            ? " dragging"
            : isDragOver
              ? dragPosition === "top"
                ? " drag-over-top"
                : " drag-over-bottom"
              : "";

          return (
            <div
              key={p.path}
              className={`tree-row tree-dataset${isOpen ? " open" : ""}${dragClass}`}
              draggable
              onDragStart={() => setDraggedIndex(i)}
              onDragEnd={() => {
                setDraggedIndex(null);
                setDragOverIndex(null);
                setDragPosition(null);
              }}
              onDragOver={(e) => handleDragOver(e, i)}
              onDragLeave={() => {
                setDragOverIndex(null);
                setDragPosition(null);
              }}
              onDrop={(e) => handleDrop(e, i)}
              onDoubleClick={() => onOpenProgram(p.path, p.content)}
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
                      onOpenProgram(p.path, p.content);
                      setContextFor(null);
                    }}
                  >
                    Open
                  </button>
                  <button
                    disabled={i === 0}
                    onClick={() => {
                      onMoveProgram(p.path, "up");
                      setContextFor(null);
                    }}
                  >
                    Move Up
                  </button>
                  <button
                    disabled={i === programs.length - 1}
                    onClick={() => {
                      onMoveProgram(p.path, "down");
                      setContextFor(null);
                    }}
                  >
                    Move Down
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
