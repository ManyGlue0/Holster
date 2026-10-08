import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, type AppView } from "../lib/api";
import { AppIcon } from "../lib/icons";
import { tileVars } from "../lib/palette";
import { useDark, useStore } from "../lib/store";

/** How long a live preview may take to load before the fallback shows. */
const LOAD_TIMEOUT = 2500;
/** Frames stay alive this long after the dock closes, so reopening is instant. */
const KEEP_ALIVE = 20000;

const fade = { duration: 0.16, ease: "easeOut" } as const;

function withTheme(url: string, theme: string) {
  const u = new URL(url);
  u.searchParams.set("theme", theme);
  return u.toString();
}

/** The preview card: the hovered app's own page in an iframe, laid over
 *  Holster's material. Pages for every previewable app stay loaded while the
 *  dock is open, so moving between tiles is a crossfade, never a reload. */
export function PreviewWindow() {
  const preview = useStore((s) => s.preview);
  const apps = useStore((s) => s.apps);
  const mode = useStore((s) => s.shell.mode);
  const dark = useDark();
  const theme = dark ? "dark" : "light";
  const dockOpen = mode === "opening" || mode === "dock";
  const [alive, setAlive] = useState(false);
  const [loaded, setLoaded] = useState<Set<string>>(new Set());
  const frames = useRef(new Map<string, HTMLIFrameElement>());

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

  const live = useMemo(
    () => apps.filter((a) => a.previewUrl && !a.error && !a.hidden && a.status !== "stopped"),
    [apps],
  );
  const active = apps.find((a) => a.id === preview.appId) ?? null;
  const activeLive = !!active && !preview.fallback && !preview.error;

  // Follow Holster's theme live, without reloading the pages.
  useEffect(() => {
    for (const [id, frame] of frames.current) {
      const url = apps.find((a) => a.id === id)?.previewUrl;
      if (url) frame.contentWindow?.postMessage({ holster: "theme", value: theme }, new URL(url).origin);
    }
  }, [theme, apps]);

  // Size requests from the pages: { holster: "resize", width, height }.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const d = e.data as { holster?: string; width?: unknown; height?: unknown } | null;
      if (!d || d.holster !== "resize") return;
      for (const [id, frame] of frames.current) {
        if (frame.contentWindow !== e.source) continue;
        const url = apps.find((a) => a.id === id)?.previewUrl;
        if (!url || new URL(url).origin !== e.origin) return;
        const w = Number(d.width);
        const h = Number(d.height);
        if (Number.isFinite(w) && Number.isFinite(h)) api.previewResize(id, w, h);
        return;
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [apps]);

  // A page that doesn't load in time gets the fallback card.
  useEffect(() => {
    if (!active || !activeLive || loaded.has(active.id)) return;
    const id = active.id;
    const t = window.setTimeout(() => api.previewFailed(id, true), LOAD_TIMEOUT);
    return () => window.clearTimeout(t);
  }, [active, activeLive, loaded]);

  const onLoad = (id: string) => {
    setLoaded((s) => new Set(s).add(id));
    api.previewFailed(id, false);
  };

  const showFallback = preview.shown && !!active && !activeLive;

  return (
    <div className="card-root preview-root" onPointerEnter={() => api.previewHover(true)} onPointerLeave={() => api.previewHover(false)}>
      <div className="card-veil" />
      {alive &&
        live.map((app) => {
          const on = preview.shown && activeLive && active?.id === app.id && loaded.has(app.id);
          return (
            <motion.div
              key={app.id}
              className="preview-layer"
              style={{
                width: on ? preview.width : undefined,
                height: on ? preview.height : undefined,
                pointerEvents: on ? "auto" : "none",
              }}
              initial={{ opacity: 0, visibility: "hidden" }}
              animate={on ? { opacity: 1, visibility: "visible" } : { opacity: 0, transitionEnd: { visibility: "hidden" } }}
              transition={fade}
            >
              <Frame app={app} theme={theme} frames={frames.current} onLoad={onLoad} />
            </motion.div>
          );
        })}
      <AnimatePresence>
        {showFallback && active && (
          <motion.div
            key={`${active.id}:${preview.error ?? ""}`}
            className="fallback"
            style={{ width: preview.width, height: preview.height }}
            initial={{ opacity: 0, x: 8 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, transition: { duration: 0.1 } }}
            transition={{ ...fade, x: { type: "spring", stiffness: 520, damping: 38 } }}
          >
            <Fallback app={active} error={preview.error} />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** One app's page. Its src is fixed at mount; theme changes arrive by message. */
function Frame({
  app,
  theme,
  frames,
  onLoad,
}: {
  app: AppView;
  theme: string;
  frames: Map<string, HTMLIFrameElement>;
  onLoad: (id: string) => void;
}) {
  const src = useMemo(() => withTheme(app.previewUrl!, theme), [app.previewUrl]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <iframe
      ref={(el) => {
        if (el) frames.set(app.id, el);
        else frames.delete(app.id);
      }}
      title={`${app.name} preview`}
      src={src}
      sandbox="allow-scripts allow-same-origin allow-forms"
      referrerPolicy="no-referrer"
      onLoad={() => onLoad(app.id)}
    />
  );
}

function Fallback({ app, error }: { app: AppView; error: string | null }) {
  const sub = error
    ? error
    : app.status === "stopped"
      ? "Not running"
      : app.hasPreview
        ? "Preview unavailable"
        : app.status === "running"
          ? "Running"
          : "No preview";
  return (
    <>
      <span className="fallback-tile" style={tileVars(app.color)}>
        <AppIcon app={app} size={app.image ? 28 : 20} />
      </span>
      <span className="fallback-text">
        <span className="fallback-name">{app.name}</span>
        <span className={`fallback-sub ${error ? "error" : ""}`}>
          {app.status === "running" && !error && <span className="fallback-dot" />}
          {sub}
        </span>
      </span>
    </>
  );
}
