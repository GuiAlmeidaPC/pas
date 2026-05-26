import { diffLines } from "diff";

export type DiffLine =
  | { kind: "add"; text: string; newLine: number }
  | { kind: "del"; text: string; oldLine: number }
  | { kind: "ctx"; text: string; oldLine: number; newLine: number };

export interface Hunk {
  oldStart: number;
  newStart: number;
  lines: DiffLine[];
}

const CONTEXT = 3;

export function computeHunks(before: string, after: string): Hunk[] {
  const parts = diffLines(before, after);
  // Flatten into a per-line stream with running old/new line numbers.
  type FlatLine =
    | { kind: "ctx"; text: string; oldLine: number; newLine: number }
    | { kind: "add"; text: string; newLine: number }
    | { kind: "del"; text: string; oldLine: number };
  const flat: FlatLine[] = [];
  let oldNo = 1;
  let newNo = 1;
  for (const part of parts) {
    const lines = part.value.split("\n");
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
    for (const text of lines) {
      if (part.added) flat.push({ kind: "add", text, newLine: newNo++ });
      else if (part.removed) flat.push({ kind: "del", text, oldLine: oldNo++ });
      else {
        flat.push({ kind: "ctx", text, oldLine: oldNo++, newLine: newNo++ });
      }
    }
  }

  // Group into hunks: each hunk is a run of non-ctx lines extended by CONTEXT
  // ctx lines before and after, collapsing larger ctx gaps between hunks.
  if (!flat.some(l => l.kind !== "ctx")) return [];

  const isChange = (i: number) => flat[i] && flat[i].kind !== "ctx";

  const hunks: Hunk[] = [];
  let i = 0;
  while (i < flat.length) {
    if (!isChange(i)) { i++; continue; }
    // Walk backwards CONTEXT lines.
    const start = Math.max(0, i - CONTEXT);
    // Walk forward, extending while another change occurs within 2*CONTEXT lines.
    let end = i;
    while (end < flat.length) {
      if (isChange(end)) {
        end++;
        continue;
      }
      // Look ahead to see whether the next change is within CONTEXT*2.
      let lookahead = end;
      let foundChange = -1;
      while (lookahead < Math.min(flat.length, end + CONTEXT * 2)) {
        if (isChange(lookahead)) { foundChange = lookahead; break; }
        lookahead++;
      }
      if (foundChange !== -1) {
        end = foundChange;
      } else {
        break;
      }
    }
    const tail = Math.min(flat.length, end + CONTEXT);
    const slice = flat.slice(start, tail);
    const first = slice[0];
    hunks.push({
      oldStart: "oldLine" in first ? first.oldLine : 1,
      newStart: "newLine" in first ? first.newLine : 1,
      lines: slice as DiffLine[],
    });
    i = tail;
  }
  return hunks;
}
