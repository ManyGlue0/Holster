import { AnimatePresence, motion, useSpring } from "framer-motion";
import { Plus } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, on, type AppView, type Rect } from "../lib/api";
import { AppIcon } from "../lib/icons";
import { DOCK_PAD_L, DOCK_PAD_Y, DOCK_W, TILE, TILE_GAP, TILE_STEP, VISIBLE_TILES } from "../lib/layout";
import { pageSpring, rowSpring } from "../lib/motion";
import { tileVars } from "../lib/palette";
import { useDockApps, useStore } from "../lib/store";

const DRAG_THRESHOLD = 4;
/** Room around the tiles for their hover ring, inside the clipped column. */
const RING_ROOM = 6;

/** The open dock: a column of app tiles (5 visible, wheel-scrolled a whole
 *  tile at a time), the grip, and the hover logic for previews. */
export function Dock() {
  const apps = useDockApps();
  const loaded = useStore((s) => s.appsLoaded);
  const overhang = useStore((s) => s.shell.overhang);
  const height = useStore((s) => s.shell.dockHeight);
  const [first, setFirst] = useState(0);
  const [launching, setLaunching] = useState<Set<string>>(new Set());
  const [pulse, setPulse] = useState<{ id: string; n: number } | null>(null);
  const preview = usePreviewController();

  const maxFirst = Math.max(0, apps.length - VISIBLE_TILES);
  const top = Math.min(first, maxFirst);
  const y = useSpring(0, pageSpring);
  useEffect(() => y.set(-top * TILE_STEP), [top, y]);

  // ---- wheel: one tile per notch; trackpads accumulate to a tile ----
  const acc = useRef(0);
  const onWheel = (e: React.WheelEvent) => {
    if (maxFirst === 0) return;
    const d = e.deltaMode === 1 ? e.deltaY * 32 : e.deltaY;
    let step = 0;
    if (Math.abs(d) >= 50) {
      step = Math.sign(d);
      acc.current = 0;
    } else {
      acc.current += d;
      if (Math.abs(acc.current) >= 44) {
        step = Math.sign(acc.current);
        acc.current = 0;
      }
    }
    if (!step) return;
    const next = Math.max(0, Math.min(maxFirst, top + step));
    if (next !== top) {
      setFirst(next);
      preview.afterScroll();
    }
  };

  useEffect(
    () =>
      on<{ id: string; on: boolean }>("launching", ({ id, on: active }) =>
        setLaunching((s) => {
          const n = new Set(s);
          if (active) n.add(id);
          else n.delete(id);
          return n;
        }),
      ),
    [],
  );
  useEffect(() => on<{ id: string }>("signal", ({ id }) => setPulse((p) => ({ id, n: (p?.n ?? 0) + 1 }))), []);

  const openApp = (app: AppView, el: HTMLElement) => {
    preview.cancelIntent();
    api.openApp(app.id, centerY(el)).catch(() => {});
  };

  return (
    <motion.div
      className="dock"
      style={{ width: DOCK_W, height, right: overhang }}
      exit={{ opacity: 0, transition: { duration: 0.07 } }}
      onPointerMove={preview.move}
      onPointerLeave={preview.leaveDock}
    >
      <Grip />
      <div
        className={`dock-column ${top > 0 ? "fade-top" : ""} ${top < maxFirst ? "fade-bottom" : ""}`}
        style={{ left: DOCK_PAD_L - RING_ROOM, height }}
        onWheel={onWheel}
      >
        <motion.div className="dock-tiles" style={{ y, paddingTop: DOCK_PAD_Y, paddingLeft: RING_ROOM, gap: TILE_GAP }}>
          {loaded && apps.length === 0 && <AddTile />}
          {apps.map((app, i) => (
            <Tile
              key={app.id}
              app={app}
              order={Math.max(0, i - top)}
              active={preview.shownId === app.id}
              launching={launching.has(app.id)}
              pulse={pulse?.id === app.id ? pulse.n : 0}
              onEnter={preview.enter}
              onLeave={preview.leave}
              onOpen={openApp}
            />
          ))}
        </motion.div>
      </div>
    </motion.div>
  );
}

function centerY(el: Element) {
  const r = el.getBoundingClientRect();
  return r.top + r.height / 2;
}

// ---- tiles --------------------------------------------------------------------

function Tile({
  app,
  order,
  active,
  launching,
  pulse,
  onEnter,
  onLeave,
  onOpen,
}: {
  app: AppView;
  order: number;
  active: boolean;
  launching: boolean;
  pulse: number;
  onEnter: (id: string, el: HTMLElement) => void;
  onLeave: (id: string) => void;
  onOpen: (app: AppView, el: HTMLElement) => void;
}) {
  return (
    <motion.button
      className={`tile ${active ? "active" : ""} ${launching ? "launching" : ""}`}
      data-app={app.id}
      aria-label={`${app.name}${app.status === "running" ? ", running" : ""}`}
      style={{ ...tileVars(app.color), width: TILE, height: TILE }}
      initial={{ opacity: 0, x: 14, scale: 0.98 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      whileTap={{ scale: 0.94 }}
      transition={{ ...rowSpring, delay: Math.min(order, VISIBLE_TILES) * 0.04 }}
      onPointerEnter={(e) => onEnter(app.id, e.currentTarget)}
      onPointerLeave={() => onLeave(app.id)}
      onClick={(e) => onOpen(app, e.currentTarget)}
    >
      <span className="tile-face">
        <AppIcon app={app} size={22} tile={TILE} />
      </span>
      <AnimatePresence>
        {launching && (
          <motion.svg
            key="spin"
            className="tile-spinner"
            viewBox="0 0 54 54"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          >
            {/* The outline stays put, on the hover ring's line; only the arc
                travels along it, so it follows the tile's shape. */}
            <rect x="1" y="1" width="52" height="52" rx="17" pathLength="100" />
          </motion.svg>
        )}
      </AnimatePresence>
      {pulse > 0 && (
        <motion.span
          key={pulse}
          className="tile-pulse"
          initial={{ opacity: 0 }}
          animate={{ opacity: [0, 0.9, 0.9, 0] }}
          transition={{ duration: 2.2, times: [0, 0.12, 0.35, 1], ease: "easeInOut" }}
        />
      )}
      <motion.span
        className="tile-dot"
        initial={false}
        animate={{ opacity: app.status === "running" ? 1 : 0, scale: app.status === "running" ? 1 : 0.4 }}
        transition={rowSpring}
      />
    </motion.button>
  );
}

/** No apps yet: one quiet tile that leads to Settings. */
function AddTile() {
  return (
    <motion.button
      className="tile add-tile"
      aria-label="Add an app"
      style={{ width: TILE, height: TILE }}
      initial={{ opacity: 0, x: 14, scale: 0.98 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      whileTap={{ scale: 0.94 }}
      transition={rowSpring}
      onClick={() => api.openPanel()}
    >
      <Plus size={18} />
    </motion.button>
  );
}

// ---- grip: drag moves the dock, click closes it ---------------------------------

function Grip() {
  const g = useRef<{ y: number; dragging: boolean; frame: number } | null>(null);
  const down = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    g.current = { y: e.screenY, dragging: false, frame: 0 };
    api.drag("start");
  };
  const move = (e: React.PointerEvent) => {
    const s = g.current;
    if (!s) return;
    if (!s.dragging && Math.abs(e.screenY - s.y) > DRAG_THRESHOLD) s.dragging = true;
    if (s.dragging && !s.frame) {
      s.frame = requestAnimationFrame(() => {
        if (g.current) g.current.frame = 0;
        api.drag("move");
      });
    }
  };
  const up = async () => {
    const s = g.current;
    g.current = null;
    if (!s) return;
    if (s.frame) cancelAnimationFrame(s.frame);
    const moved = await api.drag("end");
    if (!s.dragging && !moved) api.close();
  };
  return (
    <button
      className="grip-hit"
      aria-label="Close the dock (drag to move it)"
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
    />
  );
}

// ---- previews: hover intent, grace, and the corridor towards the card -------------

interface Sample {
  x: number;
  y: number;
  t: number;
}

function inTriangle(p: Sample, a: Sample | { x: number; y: number }, b: { x: number; y: number }, c: { x: number; y: number }) {
  const s = (p1: { x: number; y: number }, p2: { x: number; y: number }, p3: { x: number; y: number }) =>
    (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y);
  const d1 = s(p, a, b);
  const d2 = s(p, b, c);
  const d3 = s(p, c, a);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

function usePreviewController() {
  const shownId = useStore((s) => s.preview.appId);
  const delays = useStore((s) => s.settings?.delays) ?? { dock: 300, preview: 150, previewGrace: 250 };
  const r = useRef({
    hovered: null as { id: string; el: HTMLElement } | null,
    overPreview: false,
    intent: 0,
    grace: 0,
    samples: [] as Sample[],
    shownId: null as string | null,
    rel: null as Rect | null,
    delays,
  });
  r.current.shownId = shownId;
  r.current.rel = useStore((s) => s.preview.rel);
  r.current.delays = delays;

  const cancelIntent = useCallback(() => {
    window.clearTimeout(r.current.intent);
    r.current.intent = 0;
  }, []);

  const scheduleHide = useCallback(() => {
    const st = r.current;
    window.clearTimeout(st.grace);
    st.grace = window.setTimeout(() => {
      if (!st.hovered && !st.overPreview && st.shownId) api.previewHide();
    }, st.delays.previewGrace);
  }, []);

  /** Is the cursor travelling into the preview card right now? */
  const headingToPreview = useCallback(() => {
    const { samples, rel } = r.current;
    const last = samples[samples.length - 1];
    if (!rel || !last || performance.now() - last.t > 90) return false;
    const apex = samples.find((s) => last.t - s.t <= 110) ?? samples[0];
    if (last.x >= apex.x - 0.5) return false;
    return inTriangle(last, apex, { x: rel.r, y: rel.t - 8 }, { x: rel.r, y: rel.b + 8 });
  }, []);

  const fire = useCallback(() => {
    const st = r.current;
    st.intent = 0;
    const h = st.hovered;
    if (!h) return;
    if (st.shownId && st.shownId !== h.id && headingToPreview()) {
      // Passing over this tile on the way to the open preview: wait.
      st.intent = window.setTimeout(fire, 80);
      return;
    }
    if (st.shownId !== h.id) api.previewShow(h.id, centerY(h.el));
  }, [headingToPreview]);

  const enter = useCallback(
    (id: string, el: HTMLElement) => {
      const st = r.current;
      st.hovered = { id, el };
      window.clearTimeout(st.grace);
      cancelIntent();
      if (st.shownId === id) return;
      st.intent = window.setTimeout(fire, st.delays.preview);
    },
    [cancelIntent, fire],
  );

  const leave = useCallback(
    (id: string) => {
      const st = r.current;
      if (st.hovered?.id === id) st.hovered = null;
      cancelIntent();
      scheduleHide();
    },
    [cancelIntent, scheduleHide],
  );

  const move = useCallback((e: React.PointerEvent) => {
    const samples = r.current.samples;
    samples.push({ x: e.clientX, y: e.clientY, t: performance.now() });
    if (samples.length > 12) samples.shift();
  }, []);

  const leaveDock = useCallback(() => {
    r.current.hovered = null;
    cancelIntent();
    scheduleHide();
  }, [cancelIntent, scheduleHide]);

  /** After a scroll the tile under the cursor changed: follow it at once. */
  const afterScroll = useCallback(() => {
    window.setTimeout(() => {
      const st = r.current;
      const last = st.samples[st.samples.length - 1];
      if (!last) return;
      const el = document.elementFromPoint(last.x, last.y)?.closest<HTMLElement>("[data-app]");
      if (!el) return;
      st.hovered = { id: el.dataset.app!, el };
      if (st.shownId) api.previewShow(el.dataset.app!, centerY(el));
    }, 240);
  }, []);

  useEffect(
    () =>
      on<boolean>("preview-hover", (over) => {
        const st = r.current;
        st.overPreview = over;
        if (over) {
          window.clearTimeout(st.grace);
          cancelIntent();
        } else scheduleHide();
      }),
    [cancelIntent, scheduleHide],
  );

  useEffect(
    () => () => {
      window.clearTimeout(r.current.intent);
      window.clearTimeout(r.current.grace);
    },
    [],
  );

  return { shownId, enter, leave, move, leaveDock, afterScroll, cancelIntent };
}
