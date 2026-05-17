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
    <div className="tab-bar">
      {tabs.map((t) => (
        <div
          key={t.id}
          className={`editor-tab${t.id === activeId ? " active" : ""}`}
          onClick={() => onSelect(t.id)}
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
          >
            ×
          </button>
        </div>
      ))}
      <button className="new-tab-btn" onClick={onNew} title="New tab (Ctrl+N)">
        +
      </button>
    </div>
  );
}
