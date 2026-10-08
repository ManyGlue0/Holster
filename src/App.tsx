import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect } from "react";
import { flushSync } from "react-dom";
import { api, on, type AppView, type PreviewState, type Settings, type ShellState } from "./lib/api";
import { loadIcons } from "./lib/icons";
import { useDark, useStore } from "./lib/store";
import { MainWindow } from "./components/MainWindow";
import { GearWindow } from "./components/GearWindow";
import { PreviewWindow } from "./components/PreviewWindow";

const label = getCurrentWindow().label;
document.documentElement.dataset.window = label;

/** One bundle, three windows: each renders its own surface over shared state. */
export default function App() {
  const dark = useDark();

  useEffect(() => {
    const s = useStore.getState();
    api.shellState().then(s.setShell);
    api.previewState().then((preview) => useStore.setState({ preview }));
    s.loadSettings();
    s.refreshApps();
    // Warm the icon set after first paint; the tab doesn't need it.
    setTimeout(loadIcons, 200);

    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onMedia = () => useStore.setState({ systemDark: media.matches });
    media.addEventListener("change", onMedia);

    const offs = [
      on<ShellState>("shell", (st) => useStore.getState().setShell(st)),
      on<PreviewState>("preview", (preview) => useStore.setState({ preview })),
      on<AppView[]>("apps", (apps) => useStore.setState({ apps, appsLoaded: true })),
      on<Settings>("settings", (settings) => useStore.setState({ settings })),
    ];
    return () => {
      offs.forEach((off) => off());
      media.removeEventListener("change", onMedia);
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
  return <MainWindow />;
}
