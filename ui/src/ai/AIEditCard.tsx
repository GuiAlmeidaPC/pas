import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { applyPatch, applyPatchBestEffort, type ProposedEdit } from "./editProtocol";
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
  | { state: "stale"; reason: string; before: string; after: string; hunks: Hunk[] }
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
        // Reject create when a file already exists at the target path.
        try {
          await invoke<string>("read_file", { path: edit.path });
          if (cancelled) return;
          setResolved({
            state: "error",
            reason: `${edit.path} already exists. Use mode="replace" or mode="patch" instead.`,
          });
          return;
        } catch {
          if (cancelled) return;
        }
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
          const after = applyPatchBestEffort(before, edit.hunks);
          setResolved({
            state: "stale",
            reason: r.error,
            before,
            after,
            hunks: computeHunks(before, after),
          });
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
    if (resolved.state === "ready" || resolved.state === "stale") {
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
            disabled={resolved.state !== "ready" && resolved.state !== "stale"}
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
          File changed since proposal: {resolved.reason}. Use "Review in editor" to inspect.
        </div>
      )}
      {(resolved.state === "ready" || resolved.state === "stale") && (
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
