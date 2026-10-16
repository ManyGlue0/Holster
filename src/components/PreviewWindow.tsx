import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, type AppView } from "../lib/api";
import { AppIcon } from "../lib/icons";
import { tileVars } from "../lib/palette";
import { useDark, useStore } from "../lib/store";

/** How long a preview page may take to load before it's skipped. */
const LOAD_TIMEOUT = 2500;
/** Pages stay alive this long after the dock closes, so reopening is instant. */
const KEEP_ALIVE = 20000;

/** Content fades in while the card unfolds around it, and out before it folds. */
const appear = { duration: 0.2, delay: 0.04, ease: "easeOut" } as const;
const vanish = { duration: 0.12, ease: "easeIn" } as const;

/** The page to show for an app: its live page while it runs, else its page on
 *  disk (served by Holster, works when the app is off). */
function previewSrc(app: AppView): string | null {
  if (app.error || app.hidden) return null;
  if (app.previewUrl && app.status === "running") return app.previewUrl;
  if (app.previewFile) return app.previewFile;
  if (app.previewUrl && app.status !== "stopped") return app.previewUrl;
  return null;
}

function withParams(url: string, theme: string, running: boolean) {
  const u = new URL(url);
  u.searchParams.set("theme", theme);
  u.searchParams.set("running", running ? "1" : "0");
  return u.toString();
}

/** The preview card: the hovered app's own page in an iframe, laid over
 *  Holster's material. Every app's page stays loaded while the dock is open,
 *  so moving between tiles is a crossfade, never a reload. */
export function PreviewWindow() {
  const preview = useStore((s) => s.preview);
  const apps = useStore((s) => s.apps);
  const mode = useStore((s) => s.shell.mode);
  const dark = useDark();
  const theme = dark ? "dark" : "light";
  // A card on screen needs its page even if a dock state arrived late.
  const dockOpen = mode === "opening" || mode === "dock" || preview.appId !== null;
  const [alive, setAlive] = useState(false);
  const [loaded, setLoaded] = useState<Set<string>>(new Set());
  /** Bumped when an app's page misses its load window: its frame starts over. */
  const [attempts, setAttempts] = useState<Record<string, number>>({});
  const frames = useRef(new Map<string, { el: HTMLIFrameElement; origin: string }>());
  /** Last size each page was shown at, so hidden pages don't reflow. */
  const sizes = useRef(new Map<string, { w: number; h: number }>());

  useEffect(() => {
    if (dockOpen) {
      setAlive(true);
      return;
    }
    const t = window.setTimeout(() => {
      setAlive(false);
      setLoaded(new Set());
    }, KEEP_ALIVE);
    return () => window.clearTimeout(t);
  }, [dockOpen]);

  const pages = useMemo(
    () => apps.flatMap((app) => (previewSrc(app) ? [{ app, src: previewSrc(app)! }] : [])),
    [apps],
  );
  const keyOf = (app: AppView) => `${app.id}|${previewSrc(app)}|${attempts[app.id] ?? 0}`;
  const active = apps.find((a) => a.id === preview.appId) ?? null;
  const showError = preview.shown && !!active && !!preview.error;
  const activeKey = active && !preview.error ? keyOf(active) : null;
  if (active && activeKey && !preview.error) sizes.current.set(active.id, { w: preview.width, h: preview.height });

  const post = (id: string, msg: object) => {
    const f = frames.current.get(id);
    f?.el.contentWindow?.postMessage(msg, f.origin);
  };

  // Follow Holster's theme and each app's running state live.
  useEffect(() => {
    for (const { app } of pages) post(app.id, { holster: "theme", value: theme });
  }, [theme, pages]);
  useEffect(() => {
    for (const { app } of pages) post(app.id, { holster: "status", running: app.status === "running" });
  }, [pages]);
  // Tell a page when it comes into view (time to refresh its data).
  useEffect(() => {
    if (preview.shown && active) post(active.id, { holster: "show" });
  }, [preview.shown, active?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Size requests from the pages: { holster: "resize", width, height }.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const d = e.data as { holster?: string; width?: unknown; height?: unknown } | null;
      if (!d || d.holster !== "resize") return;
      for (const [id, f] of frames.current) {
        if (f.el.contentWindow !== e.source) continue;
        if (f.origin !== e.origin) return;
        const w = Number(d.width);
        const h = Number(d.height);
        if (Number.isFinite(w) && Number.isFinite(h)) api.previewResize(id, w, h);
        return;
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  // A page that doesn't load in time is skipped (the card folds away) and
  // loaded again from scratch; once it loads it's shown again.
  useEffect(() => {
    if (!active || !activeKey || loaded.has(activeKey)) return;
    const id = active.id;
    const t = window.setTimeout(() => {
      const frame = pages.some(({ app }) => keyOf(app) === activeKey);
      api.diag(
        `preview of ${id} didn't load in ${LOAD_TIMEOUT} ms (frame ${frame ? "mounted" : "missing"}, ` +
          `alive ${alive}, mode ${useStore.getState().shell.mode}, ${loaded.size}/${pages.length} pages loaded, ` +
          `page ${document.visibilityState})`,
      );
      api.previewFailed(id, true);
      setAttempts((a) => ({ ...a, [id]: (a[id] ?? 0) + 1 }));
    }, LOAD_TIMEOUT);
    return () => window.clearTimeout(t);
  }, [active, activeKey, loaded]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="card-root preview-root" onPointerEnter={() => api.previewHover(true)} onPointerLeave={() => api.previewHover(false)}>
      {alive &&
        pages.map(({ app, src }) => {
          const key = keyOf(app);
          const on = preview.shown && activeKey === key && loaded.has(key);
          const size = sizes.current.get(app.id);
          return (
            <motion.div
              key={key}
              className="preview-layer"
              style={{ width: size?.w, height: size?.h, pointerEvents: on ? "auto" : "none" }}
              initial={{ opacity: 0, visibility: "hidden" }}
              animate={on ? { opacity: 1, visibility: "visible", transition: appear } : { opacity: 0, transitionEnd: { visibility: "hidden" }, transition: vanish }}
            >
              <Frame
                app={app}
                src={src}
                theme={theme}
                frames={frames.current}
                onLoad={() => {
                  setLoaded((s) => new Set(s).add(key));
                  api.previewFailed(app.id, false);
                }}
              />
            </motion.div>
          );
        })}
      <AnimatePresence>
        {showError && active && (
          <motion.div
            key={`${active.id}:${preview.error}`}
            className="open-error"
            style={{ width: preview.width, height: preview.height }}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1, transition: appear }}
            exit={{ opacity: 0, transition: vanish }}
          >
            <span className="open-error-tile" style={tileVars(active.color)}>
              <AppIcon app={active} size={20} tile={42} />
            </span>
            <span className="open-error-text">
              <span className="open-error-name">{active.name}</span>
              <span className="open-error-sub">{preview.error}</span>
            </span>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** One app's page. Its URL is fixed at mount; theme and status changes
 *  arrive as messages, so the page never reloads under the user. */
function Frame({
  app,
  src,
  theme,
  frames,
  onLoad,
}: {
  app: AppView;
  src: string;
  theme: string;
  frames: Map<string, { el: HTMLIFrameElement; origin: string }>;
  onLoad: () => void;
}) {
  const url = useMemo(() => withParams(src, theme, app.status === "running"), [src]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <iframe
      ref={(el) => {
        if (!el) return;
        frames.set(app.id, { el, origin: new URL(src).origin });
        return () => {
          if (frames.get(app.id)?.el === el) frames.delete(app.id);
        };
      }}
      title={`${app.name} preview`}
      src={url}
      sandbox="allow-scripts allow-same-origin allow-forms"
      referrerPolicy="no-referrer"
      onLoad={onLoad}
    />
  );
}
