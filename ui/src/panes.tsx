import type { LogLine, ResultBlock } from "./types";

export function LogView({ lines }: { lines: LogLine[] }) {
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

export function OutputView({ blocks }: { blocks: ResultBlock[] }) {
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
