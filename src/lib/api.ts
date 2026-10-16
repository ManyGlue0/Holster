import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Theme = "system" | "light" | "dark";

export interface Settings {
  theme: Theme;
  hotkey: string;
  hideInFullscreen: boolean;
  launchAtStartup: boolean;
  edge: "right";
  tab: { yFrac: number };
  order: string[];
  hidden: string[];
  /** Apps whose notifications are dropped. */
  muted: string[];
  doNotDisturb: boolean;
  notifySound: boolean;
  delays: { dock: number; preview: number; previewGrace: number };
}

export type ShellMode = "closed" | "opening" | "dock" | "expanding" | "panel" | "closing";

export interface ShellState {
  mode: ShellMode;
  scale: number;
  edge: "right";
  rows: number;
  dockHeight: number;
  panelWidth: number;
  panelHeight: number;
  overhang: number;
  tucked: boolean;
  /** Newer states have a higher number; an older one arriving late is dropped. */
  seq: number;
}

export interface Rect {
  l: number;
  t: number;
  r: number;
  b: number;
}

export interface PreviewState {
  appId: string | null;
  width: number;
  height: number;
  fallback: boolean;
  error: string | null;
  shown: boolean;
  /** Card bounds relative to the main window (CSS px). */
  rel: Rect | null;
  seq: number;
}

export type AppStatus = "unknown" | "running" | "stopped";

export interface AppView {
  id: string;
  file: string;
  name: string;
  lucide: string | null;
  image: string | null;
  /** How much of the tile the image covers, in percent. */
  imageSize: number;
  color: string;
  mode: "panel" | "window" | null;
  hasPreview: boolean;
  /** Live page, served by the app while it runs. */
  previewUrl: string | null;
  /** Page on disk, served by Holster; shows whether or not the app runs. */
  previewFile: string | null;
  canLaunch: boolean;
  trackable: boolean;
  status: AppStatus;
  error: string | null;
  warnings: string[];
  hidden: boolean;
  muted: boolean;
}

export interface NoteButton {
  id: string;
  label: string;
  primary: boolean;
  /** Pressing it also opens the app. */
  open: boolean;
}

export type NoteContent =
  | { kind: "text"; title: string; body: string | null; buttons: NoteButton[] }
  | { kind: "page"; url: string; width: number; height: number; dismiss: boolean };

/** What one notification window shows. */
export interface NoteSlot {
  key: number;
  appId: string;
  content: NoteContent;
  timed: boolean;
  /** Content visible: false while it's measured, or folding away. */
  shown: boolean;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  appsDir: string;
  port: number;
  hotkeyError: string | null;
}

export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

export const api = {
  shellState: () => invoke<ShellState>("shell_state"),
  openDock: () => invoke("shell_open_dock"),
  openPanel: () => invoke("shell_open_panel"),
  close: () => invoke("shell_close"),
  hover: (on: boolean) => invoke("shell_hover", { on }),
  drag: (phase: "start" | "move" | "end") => invoke<boolean>("shell_drag", { phase }),

  previewState: () => invoke<PreviewState>("preview_state"),
  previewShow: (id: string, anchorY: number) => invoke("preview_show", { id, anchorY }),
  previewHide: () => invoke("preview_hide"),
  previewResize: (id: string, width: number, height: number) => invoke("preview_resize", { id, width, height }),
  previewFailed: (id: string, failed: boolean) => invoke("preview_failed", { id, failed }),
  /** A line for the diagnostic log (holster.log, next to settings.json). */
  diag: (message: string) => invoke("diag_log", { message }).catch(() => {}),

  listApps: () => invoke<AppView[]>("list_apps"),
  openApp: (id: string, anchorY: number) => invoke("open_app", { id, anchorY }),
  reorderApps: (ids: string[]) => invoke("reorder_apps", { ids }),
  setAppHidden: (id: string, hidden: boolean) => invoke("set_app_hidden", { id, hidden }),
  removeApp: (id: string) => invoke("remove_app", { id }),
  addApp: () => invoke<string | null>("add_app"),
  openAppsFolder: () => invoke("open_apps_folder"),

  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: DeepPartial<Settings>) => invoke<Settings>("update_settings", { patch }),
  resetTabPosition: () => invoke("reset_tab_position"),
  appInfo: () => invoke<AppInfo>("app_info"),

  noteSlots: () => invoke<(NoteSlot | null)[]>("note_slots"),
  noteMeasured: (key: number, height: number) => invoke("note_measured", { key, height }),
  noteResize: (key: number, width: number, height: number) => invoke("note_resize", { key, width, height }),
  noteHover: (key: number, on: boolean) => invoke("note_hover", { key, on }),
  noteDrag: (key: number, phase: "move" | "end", dx: number) => invoke("note_drag", { key, phase, dx }),
  /** `open` only counts for pages; a text card's button carries its own. */
  noteAction: (key: number, button: string, open = false) => invoke("note_action", { key, button, open }),
  noteDismiss: (key: number) => invoke("note_dismiss", { key }),

  /** The preview window tells the dock whether the cursor is over it. */
  previewHover: (on: boolean) => emitTo("main", "preview-hover", on),
};

export function on<T>(event: string, handler: (payload: T) => void): () => void {
  let un: UnlistenFn | null = null;
  let disposed = false;
  listen<T>(event, (e) => handler(e.payload)).then((u) => {
    if (disposed) u();
    else un = u;
  });
  return () => {
    disposed = true;
    un?.();
  };
}
