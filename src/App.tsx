import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect } from "react";
import { flushSync } from "react-dom";
import { api, on, type AppView, type NoteSlot, type PreviewState, type Settings, type ShellState } from "./lib/api";
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

/** Previews from different threads can cross on the way: keep the newest. */
function setPreview(preview: PreviewState) {
  if (preview.seq >= useStore.getState().preview.seq) useStore.setState({ preview });
}

/** One bundle, every window: each renders its own surface over shared state. */
export default function App() {
  const dark = useDark();

  useEffect(() => {
    const s = useStore.getState();
    // The main window can load before the backend has finished setting up:
    // retry the first reads until it answers.
    retry(() => api.shellState().then(s.setShell));
    retry(() => api.previewState().then(setPreview));
    retry(() => api.noteSlots().then((notes) => useStore.setState({ notes })));
    retry(s.loadSettings);
    retry(s.refreshApps);
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

    const offs = [
      on<ShellState>("shell", (st) => useStore.getState().setShell(st)),
      on<PreviewState>("preview", setPreview),
      on<AppView[]>("apps", (apps) => useStore.setState({ apps, appsLoaded: true })),
      on<Settings>("settings", (settings) => useStore.setState({ settings })),
      on<(NoteSlot | null)[]>("notes", (notes) => useStore.setState({ notes })),
    ];
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
