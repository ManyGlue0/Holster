import { useEffect } from "react";
import { create } from "zustand";
import { api, type AppView, type DeepPartial, type PreviewState, type Settings, type ShellState } from "./api";

export interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

interface Store {
  shell: ShellState;
  /** Which surface the main window shows; "closing" keeps the last one. */
  surface: "edge" | "panel";
  preview: PreviewState;
  settings: Settings | null;
  systemDark: boolean;
  apps: AppView[];
  appsLoaded: boolean;
  toasts: Toast[];
  /** Keeps the settings panel open on click outside. Session only. */
  pinned: boolean;
  /** The panel is fading out on its way back to the dock. */
  panelLeaving: boolean;

  setShell: (s: ShellState) => void;
  refreshApps: () => Promise<void>;
  loadSettings: () => Promise<void>;
  updateSettings: (patch: DeepPartial<Settings>) => Promise<void>;
  toast: (text: string, action?: Toast["action"]) => void;
  dismissToast: (id: number) => void;
}

let toastId = 0;

const surfaceOf = (mode: ShellState["mode"], prev: Store["surface"]): Store["surface"] =>
  mode === "expanding" || mode === "panel" ? "panel" : mode === "closing" ? prev : "edge";

export const useStore = create<Store>((set, get) => ({
  shell: {
    mode: "closed",
    scale: 1,
    edge: "right",
    rows: 1,
    dockHeight: 66,
    panelWidth: 420,
    panelHeight: 800,
    overhang: 0,
    tucked: false,
  },
  surface: "edge",
  preview: { appId: null, width: 0, height: 0, fallback: false, error: null, shown: false, rel: null },
  settings: null,
  systemDark: window.matchMedia("(prefers-color-scheme: dark)").matches,
  apps: [],
  appsLoaded: false,
  toasts: [],
  pinned: false,
  panelLeaving: false,

  setShell: (shell) =>
    set((s) => ({
      shell,
      surface: surfaceOf(shell.mode, s.surface),
      pinned: shell.mode === "panel" || shell.mode === "expanding" ? s.pinned : false,
      panelLeaving: shell.mode === "panel" ? s.panelLeaving : false,
    })),
  refreshApps: async () => set({ apps: await api.listApps(), appsLoaded: true }),
  loadSettings: async () => set({ settings: await api.getSettings() }),
  updateSettings: async (patch) => {
    try {
      set({ settings: await api.updateSettings(patch) });
    } catch (e) {
      get().toast(String(e));
      await get().loadSettings();
    }
  },
  toast: (text, action) => {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts.slice(-2), { id, text, action }] }));
    setTimeout(() => get().dismissToast(id), action ? 6000 : 3200);
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export function useDark(): boolean {
  const theme = useStore((s) => s.settings?.theme ?? "system");
  const systemDark = useStore((s) => s.systemDark);
  return theme === "dark" || (theme === "system" && systemDark);
}

/** Apps that appear as dock tiles, in dock order. */
export function useDockApps(): AppView[] {
  return useStore((s) => s.apps).filter((a) => !a.error && !a.hidden);
}

// ---- Escape handling: innermost overlay first, then close. ----

const escStack: (() => void)[] = [];

export function useEscape(active: boolean, handler: () => void) {
  useEffect(() => {
    if (!active) return;
    escStack.push(handler);
    return () => {
      const i = escStack.lastIndexOf(handler);
      if (i >= 0) escStack.splice(i, 1);
    };
  }, [active, handler]);
}

export function handleEscape(): boolean {
  const top = escStack[escStack.length - 1];
  if (top) {
    top();
    return true;
  }
  return false;
}

// ---- Click outside the settings panel: the shell asks after a blur. ----

let composing = false;
document.addEventListener("compositionstart", () => (composing = true), true);
document.addEventListener("compositionend", () => (composing = false), true);

export function shouldCloseOnBlur(): boolean {
  const s = useStore.getState();
  if (s.shell.mode !== "panel" || s.pinned || composing) return false;
  if (escStack.length > 0 || document.querySelector(".popover")) return false;
  return true;
}
