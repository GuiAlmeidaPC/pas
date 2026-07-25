interface Tab {
  id: string;
  title: string;
  path: string | null;
  dirty: boolean;
}

interface Props {
  tabs: Tab[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
}

export function EditorTabs({ tabs, activeId, onSelect, onClose, onNew }: Props) {
  return (
    <div className="tab-bar" role="tablist" aria-label="Open programs">
      {tabs.map((t) => (
        <div
          key={t.id}
          className={`editor-tab${t.id === activeId ? " active" : ""}`}
          onClick={() => onSelect(t.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              onSelect(t.id);
            }
          }}
          role="tab"
          tabIndex={t.id === activeId ? 0 : -1}
          aria-selected={t.id === activeId}
          title={t.path ?? "(unsaved)"}
        >
          <span className="dirty-dot" style={{ visibility: t.dirty ? "visible" : "hidden" }}>
            ●
          </span>
          <span className="tab-title">{t.title}</span>
          <button
            className="close-btn"
            onClick={(e) => {
              e.stopPropagation();
              onClose(t.id);
            }}
            title="Close (Ctrl+W)"
            aria-label={`Close ${t.title}`}
          >
            ×
          </button>
        </div>
      ))}
      <button
        className="new-tab-btn"
        onClick={onNew}
        title="New tab (Ctrl+N)"
        aria-label="New program tab"
      >
        +
      </button>
    </div>
  );
}
