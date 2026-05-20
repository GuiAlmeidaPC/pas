import { useEffect, useRef, useState } from "react";

export interface MenuItemDef {
  label: string;
  /// Displayed shortcut hint (right-aligned, e.g. "Ctrl+S").
  shortcut?: string;
  onClick?: () => void;
  disabled?: boolean;
}

export interface MenuSeparator {
  separator: true;
}

export type MenuEntry = MenuItemDef | MenuSeparator;

export interface MenuDef {
  label: string;
  items: MenuEntry[];
}

function isSeparator(e: MenuEntry): e is MenuSeparator {
  return (e as MenuSeparator).separator === true;
}

interface Props {
  menus: MenuDef[];
}

/// A classic horizontal menu bar with click-to-open dropdowns and
/// hover-to-switch behavior once any menu is open. Click outside, Esc,
/// or selecting an item closes the dropdown.
export function MenuBar({ menus }: Props) {
  const [openIdx, setOpenIdx] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (openIdx === null) return;
    const onDocClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpenIdx(null);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpenIdx(null);
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [openIdx]);

  return (
    <div className="menubar" ref={ref}>
      {menus.map((m, i) => (
        <div
          key={m.label}
          className={`menubar-item${openIdx === i ? " open" : ""}`}
          onMouseDown={(e) => {
            // Use mousedown so we beat the outside-click handler.
            e.preventDefault();
            setOpenIdx(openIdx === i ? null : i);
          }}
          onMouseEnter={() => {
            if (openIdx !== null && openIdx !== i) setOpenIdx(i);
          }}
        >
          <span className="menubar-label">{m.label}</span>
          {openIdx === i && (
            <div className="menubar-dropdown" onMouseDown={(e) => e.stopPropagation()}>
              {m.items.map((entry, j) =>
                isSeparator(entry) ? (
                  <div key={`sep-${j}`} className="menubar-sep" />
                ) : (
                  <button
                    key={entry.label}
                    className="menubar-entry"
                    disabled={entry.disabled}
                    onClick={() => {
                      setOpenIdx(null);
                      entry.onClick?.();
                    }}
                  >
                    <span className="entry-label">{entry.label}</span>
                    {entry.shortcut && (
                      <span className="entry-shortcut">{entry.shortcut}</span>
                    )}
                  </button>
                ),
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
