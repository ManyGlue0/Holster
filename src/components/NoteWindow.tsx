import { AnimatePresence, motion } from "framer-motion";
import { X } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { api, type AppView, type NoteContent } from "../lib/api";

type TextContent = Extract<NoteContent, { kind: "text" }>;
type PageContent = Extract<NoteContent, { kind: "page" }>;
import { AppIcon } from "../lib/icons";
import { tileVars } from "../lib/palette";
import { useDark, useStore } from "../lib/store";

/** Content fades in while the card unfolds around it, and out before it folds. */
const appear = { duration: 0.2, delay: 0.05, ease: "easeOut" } as const;
const vanish = { duration: 0.09, ease: "easeIn" } as const;
/** Pointer travel before a press on the card becomes a flick. */
const DRAG_THRESHOLD = 4;
/** Must match notify.rs CARD_W. */
const CARD_W = 340;

/** One notification window. Holster places the window; this draws what's in
 *  it: a card Holster lays out from the app's title, text and buttons, or the
 *  app's own page. The card grows out of the tab, so it starts and ends looking
 *  like it (the tab's material, sheen and grip) and takes on the veil while
 *  it's out, the way the dock does. */
export function NoteWindow({ slot }: { slot: number }) {
  const note = useStore((s) => s.notes[slot] ?? null);
  const app = useStore((s) => s.apps.find((a) => a.id === note?.appId)) ?? null;
  const hovered = useRef<number | null>(null);

  const hover = (on: boolean) => {
    if (!note) return;
    if (on === (hovered.current === note.key)) return;
    hovered.current = on ? note.key : null;
    api.noteHover(note.key, on);
  };

  return (
    <div
      className={`card-root note-root ${note?.shown ? "out" : ""}`}
      onPointerEnter={() => hover(true)}
      onPointerLeave={() => hover(false)}
    >
      <div className="tab-sheen" />
      <div className="tab-veil" />
      <div className="tab-grip" />
      <AnimatePresence>
        {note && app && (
          <motion.div
            key={`${note.key}:${note.content.kind}`}
            className="note-layer"
            initial={{ opacity: 0 }}
            animate={note.shown ? { opacity: 1, transition: appear } : { opacity: 0, transition: vanish }}
            exit={{ opacity: 0, transition: vanish }}
          >
            {note.content.kind === "text" ? (
              <TextNote noteKey={note.key} content={note.content} app={app} />
            ) : (
              <PageNote noteKey={note.key} content={note.content} />
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** A card Holster draws: the app's tile and name, the title, a few lines of
 *  text and up to two buttons. The card itself does nothing on a click (only
 *  a button the app marks `open` opens it); flick it into the edge to dismiss
 *  it. */
function TextNote({ noteKey, content, app }: { noteKey: number; content: TextContent; app: AppView }) {
  const ref = useRef<HTMLDivElement>(null);
  const press = useRef<{ x: number; dragging: boolean; frame: number; dx: number } | null>(null);
  const { title, body, buttons } = content;

  // Tell Holster the card's height, now and whenever the content changes.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const report = () => api.noteMeasured(noteKey, Math.ceil(el.getBoundingClientRect().height));
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => ro.disconnect();
  }, [noteKey]);

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as Element).closest("button")) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    press.current = { x: e.screenX, dragging: false, frame: 0, dx: 0 };
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const p = press.current;
    if (!p) return;
    p.dx = e.screenX - p.x;
    if (!p.dragging && Math.abs(p.dx) > DRAG_THRESHOLD) p.dragging = true;
    if (p.dragging && !p.frame) {
      p.frame = requestAnimationFrame(() => {
        if (!press.current) return;
        press.current.frame = 0;
        api.noteDrag(noteKey, "move", Math.max(0, press.current.dx));
      });
    }
  };
  const onPointerUp = () => {
    const p = press.current;
    press.current = null;
    if (!p) return;
    if (p.frame) cancelAnimationFrame(p.frame);
    if (p.dragging) api.noteDrag(noteKey, "end", Math.max(0, p.dx));
  };

  return (
    <div
      ref={ref}
      className="note"
      style={{ width: CARD_W }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <div className="note-head">
        <span className="note-tile" style={tileVars(app.color)}>
          <AppIcon app={app} size={13} tile={22} />
        </span>
        <span className="note-app">{app.name}</span>
        <button className="icon-btn small note-close" aria-label="Dismiss" title="Dismiss" onClick={() => api.noteDismiss(noteKey)}>
          <X size={14} />
        </button>
      </div>
      <div className="note-title">{title}</div>
      {body && <div className="note-body">{body}</div>}
      {buttons.length > 0 && (
        <div className="note-actions">
          {buttons.map((b) => (
            <button key={b.id} className={`btn ${b.primary ? "btn-primary" : "btn-default"}`} onClick={() => api.noteAction(noteKey, b.id)}>
              {b.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** The app's own page, in Holster's card. It speaks the preview protocol
 *  (theme, resize) plus `action` (optionally opening the app) and `close`. With `dismiss: false`
 *  the page draws its own way out and Holster leaves out its `×`. */
function PageNote({ noteKey, content }: { noteKey: number; content: PageContent }) {
  const dark = useDark();
  const theme = dark ? "dark" : "light";
  const frame = useRef<HTMLIFrameElement>(null);
  const { url, width, height, dismiss } = content;
  // The URL is fixed for the card's life; theme changes arrive as messages.
  const src = useMemo(() => {
    const u = new URL(url);
    u.searchParams.set("theme", theme);
    return u.toString();
  }, [url]); // eslint-disable-line react-hooks/exhaustive-deps
  const origin = new URL(url).origin;

  useEffect(() => {
    frame.current?.contentWindow?.postMessage({ holster: "theme", value: theme }, origin);
  }, [theme, origin]);

  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.source !== frame.current?.contentWindow || e.origin !== origin) return;
      const d = e.data as { holster?: string; width?: unknown; height?: unknown; button?: unknown; open?: unknown } | null;
      if (!d) return;
      if (d.holster === "resize") {
        const w = Number(d.width);
        const h = Number(d.height);
        if (Number.isFinite(w) && Number.isFinite(h)) api.noteResize(noteKey, w, h);
      } else if (d.holster === "action" && typeof d.button === "string") api.noteAction(noteKey, d.button, d.open === true);
      else if (d.holster === "close") api.noteDismiss(noteKey);
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [noteKey, origin]);

  return (
    <div className="note note-page" style={{ width, height }}>
      <iframe
        ref={frame}
        title="Notification"
        src={src}
        sandbox="allow-scripts allow-same-origin allow-forms"
        referrerPolicy="no-referrer"
      />
      {dismiss && (
        <button className="icon-btn small note-close note-page-close" aria-label="Dismiss" title="Dismiss" onClick={() => api.noteDismiss(noteKey)}>
          <X size={14} />
        </button>
      )}
    </div>
  );
}
