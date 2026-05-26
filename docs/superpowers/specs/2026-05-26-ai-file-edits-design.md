# AI File Edits — Copilot-style diff & apply

**Date:** 2026-05-26
**Status:** Approved (design); awaiting implementation plan
**Touches:** `ui/src/AIChatPanel.tsx`, `ui/src/App.tsx`, new `ui/src/ai/*` modules, `ui/src/styles.css`, `ui/package.json`

## Goal

Upgrade the PAS AI Assistant so that it can propose multi-file changes to the
open project, and so the user can review those changes as red/green diff hunks
(GitHub Copilot Chat style) inline in the chat — with a one-click escape to
Monaco's full diff editor — and accept or reject them per file.

Today's assistant is read-only: it emits ` ```sas ` / ` ```sql ` code blocks
that the user can Insert, Replace, New-Tab, or Add to Project via buttons.
There is no diff view and no concept of an "edit" addressed at a specific
file.

## Requirements

1. The assistant can target **any `.sas` file in the open project**, not just
   the active tab.
2. The assistant can **create new files** as well as edit existing ones.
3. Each proposed file change is rendered as a **diff card** in the chat with
   `-` removed (red) and `+` added (green) lines and line numbers.
4. Each card has **Accept**, **Reject**, and **Review in editor** actions.
   Accept is per-file; rejecting file A does not affect file B.
5. "Review in editor" opens **Monaco's `DiffEditor`** (original vs. proposed)
   with Accept/Reject controls that proxy back to the same apply path.
6. The existing plain ` ```sas ` / ` ```sql ` code-snippet card behavior is
   preserved unchanged for non-edit code samples.
7. All filesystem writes go through the existing sandboxed `write_file`
   Tauri command. No new privileged surface.

## Non-Goals (YAGNI)

- Per-hunk accept/reject within a file.
- Editing non-`.sas` files (project JSON, libname configs, etc.).
- Streaming partial edit application as the model token-streams (cards
  materialise once the response completes).
- Custom undo stack beyond what Monaco gives the user on a re-edited buffer.
- Provider-native tool-calling (OpenAI/Anthropic tool-use). Markdown-fenced
  protocol works across every provider already wired into `pas-app`.

## Edit Protocol (LLM ↔ UI wire format)

The model emits edits as fenced code blocks with an info string of
`pas-edit` and key="value" attributes. Three modes:

### `mode="patch"` — search/replace hunks (default)

````
```pas-edit path="programs/foo.sas" mode="patch"
<<<<<<< SEARCH
data want; set have; run;
=======
data want; set have; where x > 0; run;
>>>>>>> REPLACE
```
````

- A single block may contain **multiple** SEARCH/REPLACE hunks separated by
  the standard `<<<<<<< SEARCH` / `=======` / `>>>>>>> REPLACE` markers.
- SEARCH text must match the **current on-disk file contents exactly** (byte
  for byte, including whitespace). The parser does not perform fuzzy
  matching; failed matches surface as an error on the card and disable
  Accept.
- Multiple hunks targeting the same file are applied **atomically** when the
  user accepts — either all succeed or none are written.

### `mode="create"` — new file

````
```pas-edit path="programs/new_clean.sas" mode="create"
data clean; set raw; run;
```
````

- Path must not already exist in the project. If it does, the card renders
  as an error and Accept is disabled.
- On accept: file is written, then appended to `ProjectConfig.programs` via
  the existing `save_project` flow, then opened as a new tab.

### `mode="replace"` — full overwrite

````
```pas-edit path="programs/big.sas" mode="replace"
<full new file contents>
```
````

- Last-resort mode for refactors where SEARCH/REPLACE is impractical.
- The diff view computes `current_contents → block_contents` with `diffLines`
  and renders the full delta.

### Validity rules

| Condition                                                       | Outcome                       |
| --------------------------------------------------------------- | ----------------------------- |
| Unknown `mode`                                                  | Error card, Accept disabled   |
| Missing `path`                                                  | Error card, Accept disabled   |
| `path` resolves outside the project sandbox                     | `write_file` returns Err; surfaced |
| `path` extension is not `.sas`                                  | Error card, Accept disabled   |
| `mode="patch"` SEARCH text not found in current file            | Error card, "File changed since proposal — review in editor" hint |
| `mode="create"` and path already exists                         | Error card, Accept disabled   |
| `mode="replace"` on a non-existent path                         | Error card, "Use mode=create" hint |

Plain ` ```sas ` and ` ```sql ` blocks remain handled by the existing
`ai-code-snippet` renderer.

## System Prompt Changes

`AIChatPanel.tsx`'s `systemPrompt` gains a new "File Edit Protocol" section
that documents the three modes verbatim, lists the open project's program
paths (from `workspaceContext`), and tells the model:

- Prefer `mode="patch"` for surgical edits.
- Prefer `mode="create"` for genuinely new programs.
- Use `mode="replace"` only when a patch would be larger than the file.
- For pure explanation/snippets that the user will copy by hand, keep using
  plain ` ```sas ` blocks — do **not** use `pas-edit` for non-applicable
  code samples.

## Components & Files

### New

- **`ui/src/ai/editProtocol.ts`**
  Pure functions: `parseEditBlocks(markdown: string): ProposedEdit[]`,
  `applyPatch(currentContents: string, hunks: PatchHunk[]): Result<string>`.
  Exports the `ProposedEdit` discriminated union (`{ kind: "patch" | "create" | "replace", path, ... }`).

- **`ui/src/ai/diff.ts`**
  Thin wrapper around `diff.diffLines` producing a view-model:
  `Hunk[] = { oldStart, newStart, lines: { kind: "add"|"del"|"ctx", text }[] }`.
  Collapses unchanged runs to context windows (3 lines above/below).

- **`ui/src/ai/AIEditCard.tsx`**
  React component rendering one `ProposedEdit`. States: `pending` → `applied` | `rejected` | `error`. Resolves the current on-disk
  contents via `invoke("read_file", { path })` on mount (or treats as empty
  for `create`). Emits `onApply(edit)` / `onReview(edit)` / `onReject()`.

- **`ui/src/__tests__/editProtocol.test.ts`**
  Unit tests for parser (all three modes, multi-hunk, malformed inputs) and
  `applyPatch` (success, SEARCH not found, overlapping hunks).

### Modified

- **`ui/src/AIChatPanel.tsx`**
  - Extend `renderMessageContent` to detect `pas-edit` info strings before
    the existing `(?:sas|sql)?` regex; route those to `AIEditCard`.
  - Append the File Edit Protocol section to `systemPrompt`.
  - New props: `onApplyEdit(edit: ProposedEdit): Promise<void>`,
    `onReviewEdit(edit: ProposedEdit): void`.

- **`ui/src/App.tsx`**
  - Implement `handleApplyEdit`:
    - patch/replace → `write_file`; if path is an open tab, update tab
      content + Monaco model in place.
    - create → `write_file`; append to `project.programs`; persist via
      existing `save_project` invoke; open as a new tab.
  - Implement `handleReviewEdit`: open a modal containing a Monaco
    `DiffEditor` (original = `read_file` result, modified = proposed
    contents) with Accept / Reject buttons that proxy to `handleApplyEdit`.
  - Pass both handlers into `<AIChatPanel>`.

- **`ui/src/styles.css`**
  New classes: `.ai-edit-card`, `.ai-edit-card-header`, `.ai-edit-card-actions`,
  `.diff-hunk`, `.diff-hunk-header`, `.diff-add`, `.diff-del`, `.diff-ctx`,
  `.ai-edit-applied`, `.ai-edit-rejected`, `.ai-edit-error`.

- **`ui/package.json`**
  Add `diff` (jsdiff) runtime dep and `@types/diff` dev dep.

### Untouched

- `crates/pas-app/src/lib.rs` — existing `read_file` / `write_file` /
  `save_project` commands cover everything. No new Tauri commands.
- `crates/pas-engine/*` — entirely unrelated.

## Data Flow

```
LLM response (markdown)
   │
   ▼
parseEditBlocks() → ProposedEdit[]
   │
   ▼
AIChatPanel.renderMessageContent → AIEditCard (one per edit)
   │
   ├─ on mount → invoke("read_file", path) → current contents
   ├─ applyPatch() or full-replace → proposed contents
   ├─ diff.diffLines() → Hunk[] (red/green render)
   │
   ├─ Accept ─► onApplyEdit(edit) ─► App.handleApplyEdit
   │                                  ├─ write_file
   │                                  ├─ update open tab (if any)
   │                                  └─ create→append-to-project+open-tab
   │
   ├─ Reject ─► local card state → "rejected"
   │
   └─ Review in editor ─► onReviewEdit(edit) ─► App opens Monaco DiffEditor modal
                                                with same Accept/Reject buttons
```

## Guardrails

- **Sandbox**: `write_file`'s project-root + `.sas`-only enforcement is the
  security boundary. The UI does not duplicate it; it only renders error
  cards when the backend returns Err.
- **Stale-base detection**: before applying a `patch`, the card re-reads
  the file and re-verifies SEARCH matches. If not, Accept is disabled with
  a "File changed since proposal — review in editor" message.
- **No project open**: cards render with all actions disabled and a hint to
  open a project.
- **Atomicity**: multi-hunk patches build the full new contents in memory
  first, then write once. A single failing hunk aborts the whole apply with
  no FS write.

## Testing

- `editProtocol.test.ts` — parser + applier unit tests (Vitest).
- Manual smoke: prompt the assistant with "add a where clause to programs/foo.sas filtering qty > 10"; verify card renders red/del + green/add lines, Accept writes file, open tab updates.
- Manual smoke: prompt "create a new program programs/agg.sas that summarises sales by region"; verify create card, Accept adds to project tree and opens tab.
- Manual smoke: edit the file manually between proposal and Accept; verify stale-base warning appears.

## Open Implementation Decisions (defer to plan)

- **Modal vs. transient tab** for Monaco DiffEditor. Recommendation: modal
  for MVP — less invasive, no tab-state plumbing. Revisit if multi-edit
  review benefits from a tab-per-review.
- **Where `AIEditCard` fetches `read_file`** — in the card itself (simple,
  re-fetches on remount) vs. hoisted into a small per-message cache. MVP:
  in the card.
