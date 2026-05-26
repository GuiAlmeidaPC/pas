export interface PatchHunk {
  search: string;
  replace: string;
}

export type ProposedEdit =
  | { kind: "patch"; path: string; hunks: PatchHunk[] }
  | { kind: "create"; path: string; contents: string }
  | { kind: "replace"; path: string; contents: string }
  | { kind: "error"; path: string | null; reason: string; raw: string };

const FENCE_RE = /^```pas-edit\b([^\n]*)\n([\s\S]*?)\n```/gm;

function parseAttrs(header: string): Record<string, string> {
  const attrs: Record<string, string> = {};
  const re = /(\w+)\s*=\s*"([^"]*)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(header)) !== null) {
    attrs[m[1]] = m[2];
  }
  return attrs;
}

function parsePatchBody(body: string): PatchHunk[] | { error: string } {
  const hunks: PatchHunk[] = [];
  const lines = body.split("\n");
  let i = 0;
  while (i < lines.length) {
    if (lines[i].trim() === "") { i++; continue; }
    if (lines[i] !== "<<<<<<< SEARCH") {
      return { error: `expected "<<<<<<< SEARCH" at line ${i + 1}` };
    }
    const sepIdx = lines.indexOf("=======", i + 1);
    if (sepIdx === -1) return { error: "missing =======" };
    const endIdx = lines.indexOf(">>>>>>> REPLACE", sepIdx + 1);
    if (endIdx === -1) return { error: "missing >>>>>>> REPLACE" };
    hunks.push({
      search: lines.slice(i + 1, sepIdx).join("\n"),
      replace: lines.slice(sepIdx + 1, endIdx).join("\n"),
    });
    i = endIdx + 1;
  }
  if (hunks.length === 0) return { error: "no hunks found" };
  return hunks;
}

export function parseEditBlocks(markdown: string): ProposedEdit[] {
  const normalized = markdown.replace(/\r\n/g, "\n");
  const edits: ProposedEdit[] = [];
  for (const m of normalized.matchAll(FENCE_RE)) {
    const attrs = parseAttrs(m[1]);
    const body = m[2];
    const raw = m[0];
    const path = attrs.path ?? null;
    const mode = attrs.mode ?? "patch";

    if (!path) {
      edits.push({ kind: "error", path: null, reason: "missing path attribute", raw });
      continue;
    }
    if (mode === "create") {
      edits.push({ kind: "create", path, contents: body });
    } else if (mode === "replace") {
      edits.push({ kind: "replace", path, contents: body });
    } else if (mode === "patch") {
      const parsed = parsePatchBody(body);
      if ("error" in parsed) {
        edits.push({ kind: "error", path, reason: parsed.error, raw });
      } else {
        edits.push({ kind: "patch", path, hunks: parsed });
      }
    } else {
      edits.push({ kind: "error", path, reason: `unknown mode "${mode}"`, raw });
    }
  }
  return edits;
}
