import { Suspense, lazy, useEffect } from "react";
import { api, on } from "../lib/api";
import { handleEscape, shouldCloseOnBlur, useStore } from "../lib/store";
import { Edge } from "./Edge";

// Settings (dnd, forms) loads right after the tab paints.
const loadPanel = () => import("./Panel").then((m) => ({ default: m.Panel }));
const Panel = lazy(loadPanel);

/** The main window: the tab and dock on the edge, or the settings panel. */
export function MainWindow() {
  const surface = useStore((s) => s.surface);
  // While unfolding into the panel the dock's tiles still fade out on the edge.
  const expanding = useStore((s) => s.shell.mode === "expanding");

  useEffect(() => {
    setTimeout(loadPanel, 400);
    const offs = [
      on("shell-blur", () => {
        if (shouldCloseOnBlur()) api.close();
      }),
    ];
    // Escape in the focused panel. (The dock never has focus; the shell's
    // keyboard hook handles Escape there.)
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (handleEscape()) return;
      api.close();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      offs.forEach((off) => off());
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  return (
    <>
      {(surface === "edge" || expanding) && <Edge />}
      <div className="panel-host" style={{ display: surface === "panel" ? undefined : "none" }}>
        <Suspense fallback={null}>
          <Panel />
        </Suspense>
      </div>
    </>
  );
}
