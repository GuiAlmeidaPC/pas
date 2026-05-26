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
