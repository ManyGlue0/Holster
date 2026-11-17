import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect } from "react";
import { flushSync } from "react-dom";
import { api, subscribe, type AppView, type NoteSlot, type PreviewState, type Settings, type ShellState } from "./lib/api";
import { loadIcons } from "./lib/icons";
import { useDark, useStore } from "./lib/store";
import { MainWindow } from "./components/MainWindow";
import { GearWindow } from "./components/GearWindow";
import { PreviewWindow } from "./components/PreviewWindow";
import { NoteWindow } from "./components/NoteWindow";

const label = getCurrentWindow().label;

function retry(fn: () => Promise<unknown>, attempt = 0) {
  fn().catch(() => {
    if (attempt < 50) setTimeout(() => retry(fn, attempt + 1), 60 + attempt * 20);
  });
}
document.documentElement.dataset.window = label;

/** Keeps one piece of backend state in the store: listen first, then read.
 *  Reading first loses any change made between the read and the listener
 *  being registered (at startup: the API port, so on-disk previews had no URL
 *  until some other change came along). A read an event overtook is dropped,
 *  so an older snapshot never replaces newer state. */
function sync<T>(event: string, read: () => Promise<T>, apply: (value: T) => void): () => void {
  let events = 0;
  const sub = subscribe<T>(event, (value) => {
    events++;
    apply(value);
  });
  sub.ready
    .catch(() => api.diag(`couldn't listen for "${event}"`))
    .then(() =>
      // The window can load before the backend has finished setting up:
      // retry the first read until it answers.
      retry(() => {
        const seen = events;
        return read().then((value) => {
          if (events === seen) apply(value);
        });
      }),
    );
  return sub.off;
}

/** Previews from different threads can cross on the way: keep the newest. */
function setPreview(preview: PreviewState) {
  if (preview.seq >= useStore.getState().preview.seq) useStore.setState({ preview });
}

/** One bundle, every window: each renders its own surface over shared state. */
export default function App() {
  const dark = useDark();

  useEffect(() => {
    const offs = [
      sync<ShellState>("shell", api.shellState, (st) => useStore.getState().setShell(st)),
      sync<PreviewState>("preview", api.previewState, setPreview),
      sync<AppView[]>("apps", api.listApps, (apps) => useStore.setState({ apps, appsLoaded: true })),
      sync<Settings>("settings", api.getSettings, (settings) => useStore.setState({ settings })),
      sync<(NoteSlot | null)[]>("notes", api.noteSlots, (notes) => useStore.setState({ notes })),
    ];
    // Warm the icon set after first paint; the tab doesn't need it.
    setTimeout(loadIcons, 200);

    // Anything that breaks in a window goes to the diagnostic log.
    const onError = (e: ErrorEvent) => api.diag(`error: ${e.message} at ${e.filename}:${e.lineno}`);
    const onRejection = (e: PromiseRejectionEvent) => api.diag(`unhandled rejection: ${String(e.reason)}`);
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);

    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onMedia = () => useStore.setState({ systemDark: media.matches });
    media.addEventListener("change", onMedia);

    return () => {
      offs.forEach((off) => off());
      media.removeEventListener("change", onMedia);
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, []);

  // Theme: crossfade with a view transition when the value changes in the open panel.
  useLayoutEffect(() => {
    const root = document.documentElement;
    const next = dark ? "dark" : "light";
    if (root.dataset.theme === next) return;
    const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
    if (root.dataset.theme && doc.startViewTransition && useStore.getState().shell.mode === "panel") {
      doc.startViewTransition(() => flushSync(() => (root.dataset.theme = next)));
    } else {
      root.dataset.theme = next;
    }
  }, [dark]);

  if (label === "gear") return <GearWindow />;
  if (label === "preview") return <PreviewWindow />;
  if (label.startsWith("notify-")) return <NoteWindow slot={Number(label.slice(7))} />;
  return <MainWindow />;
}
