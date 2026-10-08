import { AnimatePresence, motion, useAnimationControls } from "framer-motion";
import { useCallback, useEffect, useRef } from "react";
import { api, on } from "../lib/api";
import { tileColors } from "../lib/palette";
import { useStore } from "../lib/store";
import { Dock } from "./Dock";

const DRAG_THRESHOLD = 4;

// Ambient tint that arrived during a fullscreen app; shown once it's gone.
let queuedTint: string | null = null;

/** The edge of the screen: the frosted tab, and after a short hover, the
 *  dock. The window bounds are the visible shape; this view adds depth, the
 *  grip, the ambient tint and the dock's content. */
export function Edge() {
  const mode = useStore((s) => s.shell.mode);
  const fullscreen = useStore((s) => s.shell.fullscreen);
  const dockDelay = useStore((s) => s.settings?.delays.dock ?? 120);
  const tint = useAnimationControls();
  const gesture = useRef<{ y: number; dragging: boolean; frame: number } | null>(null);
  const intent = useRef<number | null>(null);

  const open = mode === "opening" || mode === "dock";
  const clearIntent = () => {
    if (intent.current) window.clearTimeout(intent.current);
    intent.current = null;
  };
  useEffect(() => clearIntent, []);
  useEffect(() => {
    if (fullscreen || mode !== "closed") clearIntent();
  }, [fullscreen, mode]);

  const playTint = useCallback(
    (color: string) => {
      tint.set({ backgroundColor: tileColors(color).strong, opacity: 0 });
      tint.start({
        opacity: [0, 0.75, 0.75, 0],
        transition: { duration: 2.2, times: [0, 0.12, 0.35, 1], ease: "easeInOut" },
      });
    },
    [tint],
  );

  useEffect(
    () =>
      on<{ id: string; color: string }>("signal", (e) => {
        const st = useStore.getState().shell;
        // An open dock pulses the app's tile instead (see Dock).
        if (st.mode !== "closed" && st.mode !== "closing") return;
        if (st.fullscreen) queuedTint = e.color;
        else playTint(e.color);
      }),
    [playTint],
  );

  // Leaving the game: once the tab has slid back out, show what was missed.
  useEffect(() => {
    if (fullscreen || !queuedTint) return;
    const color = queuedTint;
    const t = window.setTimeout(() => {
      queuedTint = null;
      playTint(color);
    }, 450);
    return () => window.clearTimeout(t);
  }, [fullscreen, playTint]);

  // ---- tab: hover opens the dock, vertical drag moves it, click opens at once ----
  const onEnter = () => {
    if (mode !== "closed" || fullscreen) return;
    api.hover(true);
    clearIntent();
    intent.current = window.setTimeout(() => {
      intent.current = null;
      if (!gesture.current) api.openDock();
    }, dockDelay);
  };
  const onLeave = () => {
    clearIntent();
    if (mode === "closed" && !gesture.current) api.hover(false);
  };
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || mode !== "closed" || fullscreen) return;
    clearIntent();
    (e.target as Element).setPointerCapture(e.pointerId);
    gesture.current = { y: e.screenY, dragging: false, frame: 0 };
    api.drag("start");
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const g = gesture.current;
    if (!g) return;
    if (!g.dragging && Math.abs(e.screenY - g.y) > DRAG_THRESHOLD) g.dragging = true;
    if (g.dragging && !g.frame) {
      g.frame = requestAnimationFrame(() => {
        if (gesture.current) gesture.current.frame = 0;
        api.drag("move");
      });
    }
  };
  const onPointerUp = async () => {
    const g = gesture.current;
    gesture.current = null;
    if (!g) return;
    if (g.frame) cancelAnimationFrame(g.frame);
    const moved = await api.drag("end");
    if (!g.dragging && !moved) api.openDock();
  };

  return (
    <div
      className={`tab ${open ? "open" : ""} ${fullscreen && mode === "closed" ? "tucked" : ""}`}
      onPointerEnter={onEnter}
      onPointerLeave={onLeave}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <motion.div className="tab-tint" animate={tint} initial={{ opacity: 0 }} />
      <div className="tab-sheen" />
      <div className="tab-veil" />
      <div className="tab-grip" />
      <AnimatePresence>{mode === "dock" && <Dock key="dock" />}</AnimatePresence>
    </div>
  );
}
