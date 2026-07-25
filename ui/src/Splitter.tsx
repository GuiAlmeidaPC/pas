import { useCallback } from "react";

interface Props {
  direction: "horizontal" | "vertical";
  onResize: (delta: number) => void;
}

/// Thin draggable handle. `horizontal` direction means it sits between two
/// columns and drags left/right; `vertical` sits between rows and drags
/// up/down.
export function Splitter({ direction, onResize }: Props) {
  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      let last = direction === "horizontal" ? e.clientX : e.clientY;
      document.body.style.cursor = direction === "horizontal" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
      const onMove = (ev: MouseEvent) => {
        const cur = direction === "horizontal" ? ev.clientX : ev.clientY;
        onResize(cur - last);
        last = cur;
      };
      const onUp = () => {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    },
    [direction, onResize],
  );

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const delta =
        e.key === "ArrowLeft" || e.key === "ArrowUp"
          ? -10
          : e.key === "ArrowRight" || e.key === "ArrowDown"
            ? 10
            : 0;
      if (delta !== 0) {
        e.preventDefault();
        onResize(delta);
      }
    },
    [onResize],
  );

  return (
    <div
      className={`splitter splitter-${direction}`}
      onMouseDown={onMouseDown}
      onKeyDown={onKeyDown}
      role="separator"
      tabIndex={0}
      aria-label={`Resize ${direction === "horizontal" ? "columns" : "rows"}`}
      aria-orientation={direction === "horizontal" ? "vertical" : "horizontal"}
    />
  );
}
