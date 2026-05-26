# AI File Edits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Copilot-style multi-file AI edits to the PAS AI Assistant: the LLM emits `pas-edit` fenced blocks, the chat renders them as red/green diff cards, the user accepts or rejects per file, with an escape hatch into Monaco's `DiffEditor`.

**Architecture:** Pure parsing/diff/apply logic in `ui/src/ai/*` (unit-tested with Vitest). A new `AIEditCard` React component renders one card per proposed edit; `AIChatPanel` routes `pas-edit` fenced blocks to it instead of the existing `ai-code-snippet`. `App.tsx` owns FS writes via the existing sandboxed `write_file` Tauri command, project-registry updates, open-tab synchronisation, and a Monaco `DiffEditor` modal for "Review in editor". No new Tauri commands.

**Tech Stack:** React 18 + TypeScript, Vite, Vitest + @testing-library/react, `@monaco-editor/react`, Tauri 2 `invoke`, `diff` (jsdiff) for line diffs.

**Spec:** `docs/superpowers/specs/2026-05-26-ai-file-edits-design.md`

---

## File Map

| Path | Status | Responsibility |
| --- | --- | --- |
| `ui/package.json` | modify | Add `diff` + `@types/diff` |
| `ui/src/ai/editProtocol.ts` | create | Parse `pas-edit` blocks; apply SEARCH/REPLACE hunks |
| `ui/src/ai/diff.ts` | create | jsdiff wrapper → `Hunk[]` view-model |
| `ui/src/ai/AIEditCard.tsx` | create | Diff card UI: per-file Accept/Reject/Review |
| `ui/src/ai/DiffReviewModal.tsx` | create | Monaco `DiffEditor` modal |
| `ui/src/AIChatPanel.tsx` | modify | System prompt, route `pas-edit` blocks, new props |
| `ui/src/App.tsx` | modify | `handleApplyEdit`, `handleReviewEdit`, modal state, pass props |
| `ui/src/styles.css` | modify | `.ai-edit-card`, `.diff-add/.del/.ctx`, modal |
| `ui/src/__tests__/editProtocol.test.ts` | create | Parser + applier unit tests |
| `ui/src/__tests__/diff.test.ts` | create | Hunk view-model unit tests |
| `ui/src/__tests__/AIEditCard.test.tsx` | create | Component behaviour test |

---

## Task 1: Add `diff` dep and define the protocol parser

**Files:**
- Modify: `ui/package.json`
- Create: `ui/src/ai/editProtocol.ts`
- Test: `ui/src/__tests__/editProtocol.test.ts`

- [ ] **Step 1: Add the `diff` dependency**

Run from `ui/`:

```bash
pnpm add diff
pnpm add -D @types/diff
```

Expected: `package.json` gains `"diff": "^7.x"` under `dependencies` and `"@types/diff": "^5.x"` under `devDependencies`. Lockfile updates.

- [ ] **Step 2: Write the failing parser test**

Create `ui/src/__tests__/editProtocol.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { parseEditBlocks } from "../ai/editProtocol";

describe("parseEditBlocks", () => {
  it("returns empty array when no pas-edit blocks present", () => {
    expect(parseEditBlocks("just text\n```sas\ndata x;\n```\n")).toEqual([]);
  });

  it("parses a single patch block with one hunk", () => {
    const md = [
      '```pas-edit path="programs/foo.sas" mode="patch"',
      "<<<<<<< SEARCH",
      "data want; set have; run;",
      "=======",
      "data want; set have; where x>0; run;",
      ">>>>>>> REPLACE",
      "```",
    ].join("\n");
    const edits = parseEditBlocks(md);
    expect(edits).toHaveLength(1);
    expect(edits[0]).toMatchObject({
      kind: "patch",
      path: "programs/foo.sas",
      hunks: [
        {
          search: "data want; set have; run;",
          replace: "data want; set have; where x>0; run;",
        },
      ],
    });
  });

  it("parses multiple hunks in one patch block", () => {
    const md = [
      '```pas-edit path="a.sas" mode="patch"',
      "<<<<<<< SEARCH",
      "old1",
      "=======",
      "new1",
      ">>>>>>> REPLACE",
      "<<<<<<< SEARCH",
      "old2",
      "=======",
      "new2",
      ">>>>>>> REPLACE",
      "```",
    ].join("\n");
    const edits = parseEditBlocks(md);
    expect(edits[0]).toMatchObject({
      kind: "patch",
      hunks: [
        { search: "old1", replace: "new1" },
        { search: "old2", replace: "new2" },
      ],
    });
  });

  it("parses create blocks", () => {
    const md = [
      '```pas-edit path="programs/new.sas" mode="create"',
      "data clean; set raw; run;",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toEqual({
      kind: "create",
      path: "programs/new.sas",
      contents: "data clean; set raw; run;",
    });
  });

  it("parses replace blocks", () => {
    const md = [
      '```pas-edit path="big.sas" mode="replace"',
      "line1",
      "line2",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toEqual({
      kind: "replace",
      path: "big.sas",
      contents: "line1\nline2",
    });
  });

  it("returns an error edit when path attribute is missing", () => {
    const md = '```pas-edit mode="create"\nx\n```';
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error", reason: /path/ });
  });

  it("returns an error edit when mode is unknown", () => {
    const md = '```pas-edit path="a.sas" mode="rewrite"\nx\n```';
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error", reason: /mode/ });
  });

  it("returns an error edit for malformed patch markers", () => {
    const md = [
      '```pas-edit path="a.sas" mode="patch"',
      "<<<<<<< SEARCH",
      "no separator or closer here",
      "```",
    ].join("\n");
    expect(parseEditBlocks(md)[0]).toMatchObject({ kind: "error" });
  });
});
```

- [ ] **Step 3: Run the test, expect failure**

Run from `ui/`: `pnpm test editProtocol`
Expected: FAIL — `Cannot find module '../ai/editProtocol'`.

- [ ] **Step 4: Implement `parseEditBlocks`**

Create `ui/src/ai/editProtocol.ts`:

```ts
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
  const edits: ProposedEdit[] = [];
  for (const m of markdown.matchAll(FENCE_RE)) {
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
```

- [ ] **Step 5: Run the test, expect pass**

Run: `pnpm test editProtocol`
Expected: PASS — all 7 tests green.

- [ ] **Step 6: Commit**

```bash
git add ui/package.json ui/pnpm-lock.yaml ui/src/ai/editProtocol.ts ui/src/__tests__/editProtocol.test.ts
git commit -m "feat(ui): parse pas-edit fenced blocks from LLM responses"
```

---

## Task 2: Implement `applyPatch`

**Files:**
- Modify: `ui/src/ai/editProtocol.ts`
- Modify: `ui/src/__tests__/editProtocol.test.ts`

- [ ] **Step 1: Add the failing applier tests**

Append to `ui/src/__tests__/editProtocol.test.ts`:

```ts
import { applyPatch } from "../ai/editProtocol";

describe("applyPatch", () => {
  it("applies a single hunk", () => {
    const r = applyPatch("a\nb\nc\n", [{ search: "b", replace: "BB" }]);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value).toBe("a\nBB\nc\n");
  });

  it("applies multiple hunks in order", () => {
    const r = applyPatch("a\nb\nc\n", [
      { search: "a", replace: "A" },
      { search: "c", replace: "C" },
    ]);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.value).toBe("A\nb\nC\n");
  });

  it("fails when SEARCH text is not found", () => {
    const r = applyPatch("a\nb\nc\n", [{ search: "missing", replace: "x" }]);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toMatch(/not found/i);
  });

  it("fails when SEARCH text matches more than once (ambiguous)", () => {
    const r = applyPatch("a\na\n", [{ search: "a", replace: "B" }]);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toMatch(/ambiguous|multiple/i);
  });
});
```

- [ ] **Step 2: Run, expect failure**

Run: `pnpm test editProtocol`
Expected: FAIL — `applyPatch` is not exported.

- [ ] **Step 3: Implement `applyPatch`**

Append to `ui/src/ai/editProtocol.ts`:

```ts
export type PatchResult =
  | { ok: true; value: string }
  | { ok: false; error: string; hunkIndex: number };

export function applyPatch(contents: string, hunks: PatchHunk[]): PatchResult {
  let current = contents;
  for (let i = 0; i < hunks.length; i++) {
    const { search, replace } = hunks[i];
    const first = current.indexOf(search);
    if (first === -1) {
      return { ok: false, error: `hunk ${i + 1}: SEARCH text not found in file`, hunkIndex: i };
    }
    const second = current.indexOf(search, first + 1);
    if (second !== -1) {
      return {
        ok: false,
        error: `hunk ${i + 1}: SEARCH text is ambiguous (matches multiple locations)`,
        hunkIndex: i,
      };
    }
    current = current.slice(0, first) + replace + current.slice(first + search.length);
  }
  return { ok: true, value: current };
}
```

- [ ] **Step 4: Run, expect pass**

Run: `pnpm test editProtocol`
Expected: PASS — all 11 tests.

- [ ] **Step 5: Commit**

```bash
git add ui/src/ai/editProtocol.ts ui/src/__tests__/editProtocol.test.ts
git commit -m "feat(ui): apply SEARCH/REPLACE hunks atomically with ambiguity check"
```

---

## Task 3: Diff view-model (`diff.ts`)

**Files:**
- Create: `ui/src/ai/diff.ts`
- Test: `ui/src/__tests__/diff.test.ts`

- [ ] **Step 1: Write the failing test**

Create `ui/src/__tests__/diff.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { computeHunks } from "../ai/diff";

describe("computeHunks", () => {
  it("returns a single all-add hunk when original is empty", () => {
    const hunks = computeHunks("", "a\nb\n");
    expect(hunks).toHaveLength(1);
    expect(hunks[0].lines.every(l => l.kind === "add")).toBe(true);
    expect(hunks[0].lines.map(l => l.text)).toEqual(["a", "b"]);
  });

  it("returns a single all-del hunk when proposed is empty", () => {
    const hunks = computeHunks("a\nb\n", "");
    expect(hunks[0].lines.every(l => l.kind === "del")).toBe(true);
  });

  it("returns empty hunks when identical", () => {
    expect(computeHunks("a\nb\n", "a\nb\n")).toEqual([]);
  });

  it("collapses unchanged regions and keeps context around changes", () => {
    const before = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    const after  = "1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n10\n";
    const hunks = computeHunks(before, after);
    expect(hunks).toHaveLength(1);
    const kinds = hunks[0].lines.map(l => l.kind);
    expect(kinds).toContain("del");
    expect(kinds).toContain("add");
    // Should not include far-away unchanged lines 1 and 10
    expect(hunks[0].lines.map(l => l.text)).not.toContain("1");
    expect(hunks[0].lines.map(l => l.text)).not.toContain("10");
  });

  it("numbers context lines with both old and new line numbers", () => {
    const hunks = computeHunks("a\nb\nc\n", "a\nB\nc\n");
    const ctx = hunks[0].lines.filter(l => l.kind === "ctx");
    for (const l of ctx) {
      expect(typeof l.oldLine).toBe("number");
      expect(typeof l.newLine).toBe("number");
    }
  });
});
```

- [ ] **Step 2: Run, expect failure**

Run: `pnpm test diff`
Expected: FAIL — `Cannot find module '../ai/diff'`.

- [ ] **Step 3: Implement `computeHunks`**

Create `ui/src/ai/diff.ts`:

```ts
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
```

- [ ] **Step 4: Run, expect pass**

Run: `pnpm test diff`
Expected: PASS — all 5 tests.

- [ ] **Step 5: Commit**

```bash
git add ui/src/ai/diff.ts ui/src/__tests__/diff.test.ts
git commit -m "feat(ui): line-diff view-model with collapsed context"
```

---

## Task 4: `AIEditCard` component

**Files:**
- Create: `ui/src/ai/AIEditCard.tsx`
- Test: `ui/src/__tests__/AIEditCard.test.tsx`

- [ ] **Step 1: Write the failing component test**

Create `ui/src/__tests__/AIEditCard.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AIEditCard } from "../ai/AIEditCard";
import * as tauriCore from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("AIEditCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders a create edit with all green lines and calls onApply", async () => {
    const onApply = vi.fn().mockResolvedValue(undefined);
    render(
      <AIEditCard
        edit={{ kind: "create", path: "programs/new.sas", contents: "data x;\nrun;" }}
        isProjectOpen
        onApply={onApply}
        onReview={vi.fn()}
      />
    );
    expect(screen.getByText("programs/new.sas")).toBeInTheDocument();
    expect(screen.getByText("new")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /accept/i }));
    expect(onApply).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText(/applied/i)).toBeInTheDocument());
  });

  it("disables actions when no project is open", () => {
    render(
      <AIEditCard
        edit={{ kind: "create", path: "x.sas", contents: "x" }}
        isProjectOpen={false}
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
    expect(screen.getByText(/open a project/i)).toBeInTheDocument();
  });

  it("renders a patch edit by fetching current contents and showing -/+ lines", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockResolvedValue("data want; set have; run;\n");
    render(
      <AIEditCard
        edit={{
          kind: "patch",
          path: "programs/foo.sas",
          hunks: [{ search: "data want; set have; run;", replace: "data want; set have; where x>0; run;" }],
        }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => {
      expect(screen.getByText(/data want; set have; run;/)).toBeInTheDocument();
      expect(screen.getByText(/data want; set have; where x>0; run;/)).toBeInTheDocument();
    });
  });

  it("surfaces a stale-base error when SEARCH no longer matches", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockResolvedValue("UNRELATED\n");
    render(
      <AIEditCard
        edit={{
          kind: "patch",
          path: "programs/foo.sas",
          hunks: [{ search: "data want;", replace: "data x;" }],
        }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => {
      expect(screen.getByText(/file changed since proposal/i)).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
  });

  it("renders a protocol error edit without contacting the backend", () => {
    render(
      <AIEditCard
        edit={{ kind: "error", path: "a.sas", reason: "bad mode", raw: "" }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    expect(screen.getByText(/bad mode/)).toBeInTheDocument();
    expect(tauriCore.invoke).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run, expect failure**

Run: `pnpm test AIEditCard`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `AIEditCard.tsx`**

Create `ui/src/ai/AIEditCard.tsx`:

```tsx
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { applyPatch, type ProposedEdit } from "./editProtocol";
import { computeHunks, type Hunk } from "./diff";

interface Props {
  edit: ProposedEdit;
  isProjectOpen: boolean;
  onApply: (edit: ProposedEdit, resolved: { before: string; after: string }) => Promise<void>;
  onReview: (edit: ProposedEdit, resolved: { before: string; after: string }) => void;
}

type Resolved =
  | { state: "loading" }
  | { state: "ready"; before: string; after: string; hunks: Hunk[] }
  | { state: "stale"; reason: string }
  | { state: "error"; reason: string };

type CardStatus = "pending" | "applying" | "applied" | "rejected";

export function AIEditCard({ edit, isProjectOpen, onApply, onReview }: Props) {
  const [resolved, setResolved] = useState<Resolved>({ state: "loading" });
  const [status, setStatus] = useState<CardStatus>("pending");

  const modeBadge = edit.kind === "create" ? "new" : edit.kind === "error" ? "error" : "modified";

  useEffect(() => {
    let cancelled = false;
    async function resolve() {
      if (edit.kind === "error") return;
      if (edit.kind === "create") {
        const after = edit.contents;
        setResolved({ state: "ready", before: "", after, hunks: computeHunks("", after) });
        return;
      }
      try {
        const before = await invoke<string>("read_file", { path: edit.path });
        if (cancelled) return;
        if (edit.kind === "replace") {
          setResolved({
            state: "ready",
            before,
            after: edit.contents,
            hunks: computeHunks(before, edit.contents),
          });
          return;
        }
        // patch
        const r = applyPatch(before, edit.hunks);
        if (!r.ok) {
          setResolved({ state: "stale", reason: r.error });
          return;
        }
        setResolved({
          state: "ready",
          before,
          after: r.value,
          hunks: computeHunks(before, r.value),
        });
      } catch (e) {
        if (cancelled) return;
        setResolved({ state: "error", reason: String(e) });
      }
    }
    resolve();
    return () => { cancelled = true; };
  }, [edit]);

  const canAccept = useMemo(() => {
    if (!isProjectOpen) return false;
    if (status !== "pending") return false;
    return resolved.state === "ready";
  }, [isProjectOpen, status, resolved]);

  const handleAccept = async () => {
    if (resolved.state !== "ready") return;
    setStatus("applying");
    try {
      await onApply(edit, { before: resolved.before, after: resolved.after });
      setStatus("applied");
    } catch {
      setStatus("pending");
    }
  };

  const handleReject = () => setStatus("rejected");

  const handleReview = () => {
    if (resolved.state === "ready") {
      onReview(edit, { before: resolved.before, after: resolved.after });
    }
  };

  return (
    <div className={`ai-edit-card ai-edit-${status}`}>
      <div className="ai-edit-card-header">
        <span className={`ai-edit-badge ai-edit-badge-${modeBadge}`}>{modeBadge}</span>
        <span className="ai-edit-path">{edit.kind === "error" ? (edit.path ?? "(no path)") : edit.path}</span>
        <div className="ai-edit-card-actions">
          <button onClick={handleAccept} disabled={!canAccept} title="Apply this edit to the file">
            {status === "applying" ? "Applying…" : status === "applied" ? "Applied ✓" : "Accept"}
          </button>
          <button onClick={handleReject} disabled={status !== "pending"} title="Discard this edit">
            Reject
          </button>
          <button
            onClick={handleReview}
            disabled={resolved.state !== "ready"}
            title="Open in Monaco diff editor"
          >
            Review in editor
          </button>
        </div>
      </div>
      {!isProjectOpen && (
        <div className="ai-edit-hint">Open a project first to apply edits.</div>
      )}
      {edit.kind === "error" && (
        <div className="ai-edit-error-body">Protocol error: {edit.reason}</div>
      )}
      {resolved.state === "loading" && <div className="ai-edit-hint">Loading current contents…</div>}
      {resolved.state === "error" && (
        <div className="ai-edit-error-body">Failed to read file: {resolved.reason}</div>
      )}
      {resolved.state === "stale" && (
        <div className="ai-edit-error-body">
          File changed since proposal: {resolved.reason}. Use “Review in editor” to inspect.
        </div>
      )}
      {resolved.state === "ready" && (
        <div className="ai-edit-diff">
          {resolved.hunks.length === 0 && <div className="ai-edit-hint">(no changes)</div>}
          {resolved.hunks.map((h, hi) => (
            <div className="diff-hunk" key={hi}>
              <div className="diff-hunk-header">@@ -{h.oldStart} +{h.newStart} @@</div>
              {h.lines.map((l, li) => {
                const oldNo = "oldLine" in l ? l.oldLine : "";
                const newNo = "newLine" in l ? l.newLine : "";
                const sign = l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ";
                return (
                  <div key={li} className={`diff-line diff-${l.kind}`}>
                    <span className="diff-lineno">{oldNo}</span>
                    <span className="diff-lineno">{newNo}</span>
                    <span className="diff-sign">{sign}</span>
                    <span className="diff-text">{l.text}</span>
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      )}
      {status === "rejected" && <div className="ai-edit-hint">Rejected — no changes applied.</div>}
    </div>
  );
}
```

- [ ] **Step 4: Run, expect pass**

Run: `pnpm test AIEditCard`
Expected: PASS — all 5 tests.

- [ ] **Step 5: Commit**

```bash
git add ui/src/ai/AIEditCard.tsx ui/src/__tests__/AIEditCard.test.tsx
git commit -m "feat(ui): AIEditCard renders per-file diff with Accept/Reject/Review"
```

---

## Task 5: `DiffReviewModal` with Monaco DiffEditor

**Files:**
- Create: `ui/src/ai/DiffReviewModal.tsx`

(No unit test — Monaco isn't trivially testable in jsdom. Verified manually in Task 8.)

- [ ] **Step 1: Implement the modal**

Create `ui/src/ai/DiffReviewModal.tsx`:

```tsx
import { DiffEditor } from "@monaco-editor/react";
import type { ProposedEdit } from "./editProtocol";

interface Props {
  edit: ProposedEdit;
  before: string;
  after: string;
  onAccept: () => void;
  onClose: () => void;
}

export function DiffReviewModal({ edit, before, after, onAccept, onClose }: Props) {
  if (edit.kind === "error") return null;
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className="modal diff-review-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <span>Review change: <code>{edit.path}</code></span>
          <button className="icon-btn" onClick={onClose} aria-label="Close">×</button>
        </div>
        <div className="diff-review-body">
          <DiffEditor
            language="sas"
            original={before}
            modified={after}
            options={{
              readOnly: true,
              renderSideBySide: true,
              minimap: { enabled: false },
              automaticLayout: true,
            }}
            height="60vh"
          />
        </div>
        <div className="modal-footer">
          <button className="btn-secondary" onClick={onClose}>Cancel</button>
          <button className="btn-primary" onClick={() => { onAccept(); onClose(); }}>
            Accept change
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Type-check**

Run from `ui/`: `pnpm build`
Expected: tsc passes; vite build completes.

- [ ] **Step 3: Commit**

```bash
git add ui/src/ai/DiffReviewModal.tsx
git commit -m "feat(ui): Monaco DiffEditor modal for full-screen edit review"
```

---

## Task 6: Wire `AIChatPanel` to render edit cards

**Files:**
- Modify: `ui/src/AIChatPanel.tsx`

- [ ] **Step 1: Extend the system prompt**

In `ui/src/AIChatPanel.tsx`, locate `systemPrompt` (around line 131) and append a new section before the closing backtick:

```ts
const systemPrompt = `You are an expert SAS and PAS (Practical Analytics Studio) database programmer.
...existing content unchanged through item 6...

File Edit Protocol:
When the user asks you to modify or create program files, propose edits using \`pas-edit\` fenced
code blocks. The UI will render them as red/green diff cards with Accept/Reject/Review buttons.

Three modes (always include both \`path\` and \`mode\` as quoted attributes):

1. Surgical edit (preferred):
\`\`\`pas-edit path="programs/foo.sas" mode="patch"
<<<<<<< SEARCH
exact existing text, byte-for-byte
=======
new text
>>>>>>> REPLACE
\`\`\`
You may include multiple SEARCH/REPLACE hunks in one block; they apply atomically.

2. New file:
\`\`\`pas-edit path="programs/new.sas" mode="create"
<full file contents>
\`\`\`

3. Full overwrite (only when a patch would be larger than the file):
\`\`\`pas-edit path="programs/big.sas" mode="replace"
<full file contents>
\`\`\`

Rules:
- The SEARCH text must match the current on-disk file contents exactly (whitespace included).
- Use file paths from the <active_project> listing in the workspace context.
- Only .sas files can be edited.
- For explanation-only snippets the user will copy by hand, continue to use plain \`\`\`sas blocks — do not use pas-edit for non-applicable code samples.

Context Information:
... (rest unchanged) ...`;
```

(Apply the addition with a minimal edit; preserve the existing items 1-6 and the `Context Information` paragraph below them.)

- [ ] **Step 2: Add new props and route `pas-edit` blocks**

At the top of the file add imports:

```ts
import { parseEditBlocks, type ProposedEdit } from "./ai/editProtocol";
import { AIEditCard } from "./ai/AIEditCard";
```

Extend the `Props` interface:

```ts
interface Props {
  activeContent: string;
  activeSelection: string;
  onInsertCode: (code: string) => void;
  onReplaceCode: (code: string) => void;
  onNewTab: (code: string) => void;
  onAddToProject: (code: string) => void;
  isProjectOpen: boolean;
  customTrigger?: { prompt: string; timestamp: number } | null;
  workspaceContext: string;
  onApplyEdit: (edit: ProposedEdit, resolved: { before: string; after: string }) => Promise<void>;
  onReviewEdit: (edit: ProposedEdit, resolved: { before: string; after: string }) => void;
}
```

Destructure the two new props in the component signature.

Replace the body of `renderMessageContent` so it handles `pas-edit` blocks first, then falls back to the existing `(?:sas|sql)?` code-block regex. The simplest replacement:

```ts
const renderMessageContent = (content: string) => {
  const parts: React.ReactNode[] = [];

  // 1. Slice off pas-edit blocks and render each as an AIEditCard.
  const editFence = /```pas-edit\b[^\n]*\n[\s\S]*?\n```/g;
  let cursor = 0;
  let match: RegExpExecArray | null;
  const segments: Array<{ kind: "text" | "edit"; text: string }> = [];
  while ((match = editFence.exec(content)) !== null) {
    if (match.index > cursor) {
      segments.push({ kind: "text", text: content.slice(cursor, match.index) });
    }
    segments.push({ kind: "edit", text: match[0] });
    cursor = match.index + match[0].length;
  }
  if (cursor < content.length) {
    segments.push({ kind: "text", text: content.slice(cursor) });
  }

  segments.forEach((seg, segIdx) => {
    if (seg.kind === "edit") {
      const [edit] = parseEditBlocks(seg.text);
      if (edit) {
        parts.push(
          <AIEditCard
            key={`edit-${segIdx}`}
            edit={edit}
            isProjectOpen={isProjectOpen}
            onApply={onApplyEdit}
            onReview={onReviewEdit}
          />
        );
      }
      return;
    }
    // Existing sas/sql snippet rendering for plain text segments.
    const codeBlockRegex = /```(?:sas|sql)?([\s\S]*?)```/g;
    let lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = codeBlockRegex.exec(seg.text)) !== null) {
      const textBefore = seg.text.substring(lastIndex, m.index);
      if (textBefore.trim()) {
        parts.push(...parseMarkdownToReact(textBefore, `text-${segIdx}-${m.index}`));
      }
      const code = m[1].trim();
      parts.push(
        <div key={`code-${segIdx}-${m.index}`} className="ai-code-snippet">
          <pre><code>{code}</code></pre>
          <div className="snippet-actions">
            <button onClick={() => onInsertCode(code)} title="Insert at cursor position in editor">Insert</button>
            <button onClick={() => onReplaceCode(code)} title="Replace highlighted selection in editor" disabled={!activeSelection}>Replace</button>
            <button onClick={() => onNewTab(code)} title="Write to a new tab">New Tab</button>
            <button onClick={() => onAddToProject(code)} title={isProjectOpen ? "Add this program to the current project JSON" : "Open a project to enable adding programs"} disabled={!isProjectOpen}>Add to Project</button>
          </div>
        </div>
      );
      lastIndex = codeBlockRegex.lastIndex;
    }
    const remainingText = seg.text.substring(lastIndex);
    if (remainingText.trim() || lastIndex === 0) {
      parts.push(...parseMarkdownToReact(remainingText || seg.text, `text-end-${segIdx}`));
    }
  });

  return parts;
};
```

(Delete the old `renderMessageContent` body it replaced.)

- [ ] **Step 3: Type-check & run existing tests**

Run from `ui/`: `pnpm build && pnpm test`
Expected: tsc passes; all previously-green tests still pass.

- [ ] **Step 4: Commit**

```bash
git add ui/src/AIChatPanel.tsx
git commit -m "feat(ui): route pas-edit blocks to AIEditCard and teach the prompt"
```

---

## Task 7: `App.tsx` handlers + modal state

**Files:**
- Modify: `ui/src/App.tsx`

- [ ] **Step 1: Add imports and modal state**

Near the top of `App.tsx`, add:

```ts
import type { ProposedEdit } from "./ai/editProtocol";
import { DiffReviewModal } from "./ai/DiffReviewModal";
```

Inside `App()`, alongside other `useState`s (e.g. near `showAIPanel`), add:

```ts
const [diffReview, setDiffReview] = useState<
  | { edit: ProposedEdit; before: string; after: string }
  | null
>(null);
```

- [ ] **Step 2: Implement `handleApplyEdit`**

Add the handler near `handleAddToProject` (around line 731):

```ts
const handleApplyEdit = useCallback(
  async (edit: ProposedEdit, resolved: { before: string; after: string }) => {
    if (edit.kind === "error") return;
    if (!projectPathRef.current) {
      setLog((p) => [...p, { level: "error", text: "AI edit: open a project first" }]);
      throw new Error("no project open");
    }
    const path = edit.path;
    try {
      await invoke("write_file", { path, content: resolved.after });

      // Sync any open tab pointing at this path.
      const matchTab = tabsRef.current.find((t) => t.path === path);
      if (matchTab) {
        setTabs((prev) =>
          prev.map((t) =>
            t.id === matchTab.id
              ? { ...t, content: resolved.after, saved_content: resolved.after }
              : t,
          ),
        );
      }

      if (edit.kind === "create") {
        const newTab = makeTab({ path, title: basename(path), content: resolved.after });
        newTab.saved_content = resolved.after;
        const updatedTabs = [...tabsRef.current, newTab];
        setTabs(updatedTabs);
        setActiveId(newTab.id);
        const updatedPrograms = projectProgramsRef.current.some((p) => p.path === path)
          ? projectProgramsRef.current
          : [...projectProgramsRef.current, { path, content: resolved.after }];
        setProjectPrograms(updatedPrograms);
        await performSaveProject(false, updatedTabs, updatedPrograms);
      }
      setLog((p) => [
        ...p,
        { level: "note", text: `NOTE: AI edit applied to ${basename(path)}` },
      ]);
    } catch (e) {
      setLog((p) => [...p, { level: "error", text: `AI edit failed: ${String(e)}` }]);
      throw e;
    }
  },
  [performSaveProject],
);
```

- [ ] **Step 3: Implement `handleReviewEdit`**

Just below `handleApplyEdit`:

```ts
const handleReviewEdit = useCallback(
  (edit: ProposedEdit, resolved: { before: string; after: string }) => {
    if (edit.kind === "error") return;
    setDiffReview({ edit, before: resolved.before, after: resolved.after });
  },
  [],
);
```

- [ ] **Step 4: Pass the new props and render the modal**

Update the `<AIChatPanel>` JSX (around line 1495):

```tsx
<AIChatPanel
  activeContent={activeTab?.content ?? ""}
  activeSelection={selectionText}
  onInsertCode={handleInsertCode}
  onReplaceCode={handleReplaceCode}
  onNewTab={newTabWithContent}
  onAddToProject={handleAddToProject}
  isProjectOpen={!!projectPath}
  customTrigger={aiTrigger}
  workspaceContext={workspaceContext}
  onApplyEdit={handleApplyEdit}
  onReviewEdit={handleReviewEdit}
/>
```

(Keep the existing prop values; only the two new lines are added. Use the existing local names where they differ from this snippet.)

Render the modal once, near the end of the JSX tree before the closing fragment:

```tsx
{diffReview && (
  <DiffReviewModal
    edit={diffReview.edit}
    before={diffReview.before}
    after={diffReview.after}
    onAccept={() => handleApplyEdit(diffReview.edit, { before: diffReview.before, after: diffReview.after })}
    onClose={() => setDiffReview(null)}
  />
)}
```

- [ ] **Step 5: Type-check**

Run from `ui/`: `pnpm build`
Expected: tsc passes; vite build completes.

- [ ] **Step 6: Commit**

```bash
git add ui/src/App.tsx
git commit -m "feat(ui): wire AI edit apply/review handlers with Monaco diff modal"
```

---

## Task 8: Styles

**Files:**
- Modify: `ui/src/styles.css`

- [ ] **Step 1: Append diff/card styles**

Append to `ui/src/styles.css`:

```css
.ai-edit-card {
  border: 1px solid var(--border, #2d2d2d);
  border-radius: 6px;
  margin: 8px 0;
  background: var(--panel-bg, #1e1e1e);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}
.ai-edit-card-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border, #2d2d2d);
  background: rgba(255,255,255,0.02);
}
.ai-edit-badge {
  padding: 1px 6px;
  border-radius: 3px;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.ai-edit-badge-new { background: #1f5b2a; color: #d4f7d4; }
.ai-edit-badge-modified { background: #4a3c0d; color: #ffe699; }
.ai-edit-badge-error { background: #5b1f1f; color: #ffd4d4; }
.ai-edit-path { flex: 1; font-family: inherit; }
.ai-edit-card-actions { display: flex; gap: 4px; }
.ai-edit-card-actions button {
  font-size: 11px;
  padding: 2px 8px;
}
.ai-edit-hint { padding: 8px 10px; color: var(--muted, #888); font-size: 11px; }
.ai-edit-error-body { padding: 8px 10px; color: #ff9b9b; font-size: 11px; }
.ai-edit-diff { max-height: 320px; overflow: auto; }
.diff-hunk + .diff-hunk { border-top: 1px dashed var(--border, #2d2d2d); }
.diff-hunk-header {
  padding: 2px 10px;
  color: var(--muted, #888);
  background: rgba(255,255,255,0.03);
  font-size: 11px;
}
.diff-line {
  display: grid;
  grid-template-columns: 36px 36px 14px 1fr;
  white-space: pre;
  padding: 0 6px;
  line-height: 1.45;
}
.diff-lineno { color: var(--muted, #666); text-align: right; padding-right: 6px; }
.diff-sign { text-align: center; }
.diff-add  { background: rgba(40, 167, 69, 0.18); color: #b6f0c1; }
.diff-del  { background: rgba(220, 53, 69, 0.18); color: #f5b6bb; }
.diff-ctx  { color: var(--fg, #c8c8c8); }
.ai-edit-applied { opacity: 0.65; }
.ai-edit-rejected { opacity: 0.45; }

.diff-review-modal { width: min(1100px, 90vw); }
.diff-review-body { padding: 0; }
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 8px 12px;
  border-top: 1px solid var(--border, #2d2d2d);
}
```

- [ ] **Step 2: Commit**

```bash
git add ui/src/styles.css
git commit -m "style(ui): diff card and review-modal styling"
```

---

## Task 9: End-to-end verification

**Files:** none — runtime smoke tests.

- [ ] **Step 1: Run the full UI test suite**

Run from `ui/`: `pnpm test`
Expected: all editProtocol, diff, AIEditCard, and pre-existing tests pass.

- [ ] **Step 2: Production build**

Run from `ui/`: `pnpm build`
Expected: clean tsc + vite build, no type errors.

- [ ] **Step 3: Manual smoke — patch flow**

Launch the app: `cd crates/pas-app && cargo tauri dev`

1. Open or create a project containing at least one `.sas` program (e.g. `programs/foo.sas` with `data want; set have; run;`).
2. Open the AI Assistant panel; configure a provider.
3. Prompt: *"Add a `where qty > 10` filter to programs/foo.sas."*
4. Verify the response contains a `pas-edit` card with the file path, a yellow `modified` badge, red `-` lines and green `+` lines with line numbers.
5. Click **Accept** → confirm: file on disk updated, open tab updates in Monaco, card shows "Applied ✓", log shows the NOTE line.

- [ ] **Step 4: Manual smoke — create flow**

1. Prompt: *"Create programs/agg.sas that summarises sales by region from the sales dataset."*
2. Verify a card with `new` badge, all-green diff, path `programs/agg.sas`.
3. Click **Accept** → confirm: new tab opens, project tree shows the new program, `.pas.json` on disk includes it.

- [ ] **Step 5: Manual smoke — Review-in-editor flow**

1. Re-trigger the patch flow from Step 3 (without accepting).
2. Click **Review in editor** → confirm the Monaco DiffEditor modal opens with original on the left and proposed on the right.
3. Click **Accept change** → confirm the modal closes and the file is written.

- [ ] **Step 6: Manual smoke — stale-base detection**

1. Trigger a patch proposal. Before accepting, switch to the editor tab and type a character, then save (Ctrl+S).
2. Re-open the chat: the card should still be there, but the SEARCH text the model proposed no longer matches the saved file.
3. *(Note: cards resolve on mount; for this smoke step, ask the model to re-propose so the freshly-mounted card observes the changed file.)* Verify the card switches to "File changed since proposal" and Accept is disabled, while **Review in editor** remains enabled.

- [ ] **Step 7: Commit any incidental fixes from smoke testing**

If issues are found in steps 3-6, fix in place and commit as `fix(ui): …`. If none, skip.

---

## Self-Review

**Spec coverage:**
- Multi-file edits over any project `.sas` file → Tasks 1, 6, 7 (parser supports any path; `handleApplyEdit` writes via the same sandboxed `write_file`).
- Create new files → Tasks 1 (parser), 7 (project-registry append + tab open).
- Red/green diff cards with line numbers → Tasks 3, 4, 8.
- Per-file Accept/Reject → Task 4 (component-local status state).
- "Review in editor" with Monaco DiffEditor → Tasks 5, 7, 8.
- Legacy `sas/sql` snippet buttons preserved → Task 6's `renderMessageContent` retains the original branch.
- Sandbox via existing `write_file` → Task 7 (no new commands).
- Stale-base detection → Task 4 (`applyPatch` failure → `state: "stale"`); smoke-tested in Task 9 step 6.
- "Open a project" guard → Task 4 (`canAccept` false; hint shown).
- Atomic multi-hunk apply → Task 2 (`applyPatch` builds new contents in one pass, returns Result).

**Placeholder scan:** No "TBD"/"TODO"/"similar to". Each step contains the actual code or the actual command.

**Type consistency:**
- `ProposedEdit` discriminant `kind` used consistently as `"patch" | "create" | "replace" | "error"` in `editProtocol.ts`, `AIEditCard.tsx`, `App.tsx`, `DiffReviewModal.tsx`.
- `applyPatch` returns `PatchResult` with `ok: boolean` discriminant — consumed in `AIEditCard` via `r.ok` / `r.error`.
- `computeHunks(before, after): Hunk[]` — same arity in tests and in `AIEditCard`.
- `onApply` / `onReview` props share the `(edit, { before, after }) => …` signature across `AIEditCard`, `AIChatPanel`, `App.tsx`, `DiffReviewModal`.

---

Plan complete and saved to `docs/superpowers/plans/2026-05-26-ai-file-edits.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
