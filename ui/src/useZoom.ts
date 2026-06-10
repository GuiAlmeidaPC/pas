import { useEffect, useState } from "react";

/**
 * VS Code-style window zoom (Ctrl+= / Ctrl+- / Ctrl+0), persisted across
 * sessions in localStorage.
 */
export function useZoom() {
  const [zoomPercent, setZoomPercent] = useState<number>(() => {
    const saved = typeof localStorage !== "undefined" ? localStorage.getItem("pas.zoom") : null;
    const parsed = saved ? parseInt(saved, 10) : NaN;
    return Number.isFinite(parsed) && parsed >= 50 && parsed <= 300 ? parsed : 100;
  });

  // Apply window zoom and persist across sessions.
  useEffect(() => {
    // The non-standard `zoom` CSS property is honored by WebKit (and
    // therefore by the Tauri webview on macOS / Linux) and Chromium —
    // covers every Tauri target. Cast through `any` because the DOM
    // typings don't declare it.
    (document.body.style as unknown as Record<string, string>).zoom = `${zoomPercent}%`;
    try {
      localStorage.setItem("pas.zoom", String(zoomPercent));
    } catch { /* ignore — private mode */ }
  }, [zoomPercent]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const cmd = e.ctrlKey || e.metaKey;
      if (!cmd) return;
      // Ctrl+= (also Ctrl+Plus on numeric keypad) zooms in.
      if (e.key === "=" || e.key === "+") {
        e.preventDefault();
        setZoomPercent((z) => Math.min(300, z + 10));
      } else if (e.key === "-" || e.key === "_") {
        e.preventDefault();
        setZoomPercent((z) => Math.max(50, z - 10));
      } else if (e.key === "0") {
        e.preventDefault();
        setZoomPercent(100);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, []);

  return { zoomPercent, setZoomPercent };
}
