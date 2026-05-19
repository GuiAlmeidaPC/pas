interface Props {
  engineState: "idle" | "running";
  activeFile: string | null;
  cursor: { line: number; col: number } | null;
  libraryCount: number;
  projectName: string | null;
  zoomPercent: number;
}

export function StatusBar({
  engineState,
  activeFile,
  cursor,
  libraryCount,
  projectName,
  zoomPercent,
}: Props) {
  return (
    <footer className="status-bar">
      <span className={`engine-state ${engineState}`}>
        <span className="dot" /> {engineState === "running" ? "Running" : "Idle"}
      </span>
      <span className="sep">·</span>
      <span>{projectName ?? "no project"}</span>
      <span className="sep">·</span>
      <span title={activeFile ?? "untitled"}>{shortPath(activeFile) ?? "untitled"}</span>
      <div className="spacer" />
      <span>{libraryCount} libraries</span>
      <span className="sep">·</span>
      <span>{cursor ? `Ln ${cursor.line}, Col ${cursor.col}` : ""}</span>
      {zoomPercent !== 100 && (
        <>
          <span className="sep">·</span>
          <span title="Ctrl+= to zoom in, Ctrl+- to zoom out, Ctrl+0 to reset">
            {zoomPercent}%
          </span>
        </>
      )}
    </footer>
  );
}

function shortPath(p: string | null): string | null {
  if (!p) return null;
  // Show last two segments for context but bounded.
  const parts = p.split(/[\\/]/);
  if (parts.length <= 2) return p;
  return `…/${parts.slice(-2).join("/")}`;
}
