import { DndContext, PointerSensor, closestCenter, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, arrayMove, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { AnimatePresence, motion } from "framer-motion";
import { Bell, BellOff, ChevronLeft, ChevronRight, Download, FolderOpen, GripVertical, PackageOpen, Plus, RefreshCw, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, on, type AppInfo, type AppView } from "../lib/api";
import { AppIcon } from "../lib/icons";
import { spring } from "../lib/motion";
import { tileVars } from "../lib/palette";
import { useStore } from "../lib/store";
import { Dot, Row, Section, type DotState } from "./settingsUi";
import { Button, IconButton, PinButton, Segmented, Stepper, Toggle } from "./ui";

/** Fades the panel out, then folds it back into the dock. */
function backToDock() {
  useStore.setState({ panelLeaving: true });
  window.setTimeout(() => api.openDock(), 90);
}

export function SettingsPage() {
  const settings = useStore((s) => s.settings);
  const update = useStore((s) => s.updateSettings);
  const mode = useStore((s) => s.shell.mode);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [advanced, setAdvanced] = useState(false);

  useEffect(() => {
    if (mode === "panel") api.appInfo().then(setInfo);
  }, [settings?.hotkey, mode]);

  if (!settings) return null;

  return (
    <div className="settings">
      <header className="panel-bar">
        <IconButton label="Back to the dock" onClick={backToDock}>
          <ChevronLeft size={19} />
        </IconButton>
        <h1 className="bar-title">Settings</h1>
        <PinButton />
      </header>
      <div className="settings-scroll">
        <AppsSection />

        <Section title="Appearance">
          <Row label="Theme">
            <Segmented
              id="theme"
              value={settings.theme}
              onChange={(theme) => update({ theme })}
              options={[
                { value: "system", label: "System" },
                { value: "light", label: "Light" },
                { value: "dark", label: "Dark" },
              ]}
            />
          </Row>
          <Row label="Tab position" hint="Drag the tab or the dock's grip to move it">
            <Button variant="ghost" onClick={() => api.resetTabPosition()}>
              <RotateCcw size={14} /> Reset
            </Button>
          </Row>
        </Section>

        <Section title="Behavior">
          <Row
            label="Shortcut"
            hint={info?.hotkeyError ?? "Opens and closes the dock from anywhere"}
            warn={!!info?.hotkeyError}
          >
            <HotkeyRecorder value={settings.hotkey} onChange={(hotkey) => update({ hotkey })} />
          </Row>
          <Row label="Launch at startup">
            <Toggle label="Launch at startup" on={settings.launchAtStartup} onChange={(v) => update({ launchAtStartup: v })} />
          </Row>
          <Row label="Hide during fullscreen apps" hint="Games, videos and presentations. The shortcut still opens the dock">
            <Toggle label="Hide during fullscreen apps" on={settings.hideInFullscreen} onChange={(v) => update({ hideInFullscreen: v })} />
          </Row>
          <Row label="Do not disturb" hint="Notifications wait until you turn this off. Also in the tray menu">
            <Toggle label="Do not disturb" on={settings.doNotDisturb} onChange={(v) => update({ doNotDisturb: v })} />
          </Row>
          <Row label="Notification sound" hint="A soft sound when a notification comes in">
            <Toggle label="Notification sound" on={settings.notifySound} onChange={(v) => update({ notifySound: v })} />
          </Row>
          <div className="set-row disclosure-row">
            <button className="disclosure" aria-expanded={advanced} onClick={() => setAdvanced((a) => !a)}>
              <ChevronRight size={13} className={advanced ? "open" : ""} /> Hover timing
            </button>
          </div>
          <AnimatePresence initial={false}>
            {advanced && (
              <motion.div
                className="advanced"
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: "auto", opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ type: "spring", stiffness: 420, damping: 40, opacity: { duration: 0.15 } }}
              >
                <Row label="Open the dock after" hint="How long to rest on the tab">
                  <Stepper
                    label="dock delay"
                    value={settings.delays.dock}
                    min={0}
                    max={800}
                    step={20}
                    unit="ms"
                    onChange={(dock) => update({ delays: { dock } })}
                  />
                </Row>
                <Row label="Show a preview after" hint="How long to rest on a tile">
                  <Stepper
                    label="preview delay"
                    value={settings.delays.preview}
                    min={0}
                    max={800}
                    step={10}
                    unit="ms"
                    onChange={(preview) => update({ delays: { preview } })}
                  />
                </Row>
                <Row label="Keep a preview for" hint="After the cursor leaves it">
                  <Stepper
                    label="preview grace"
                    value={settings.delays.previewGrace}
                    min={50}
                    max={1500}
                    step={50}
                    unit="ms"
                    onChange={(previewGrace) => update({ delays: { previewGrace } })}
                  />
                </Row>
              </motion.div>
            )}
          </AnimatePresence>
        </Section>

        <UpdatesSection version={info?.version} />

        <div className="about">
          Holster {info?.version}
          {info?.port ? (
            <>
              {" "}
              · apps reach it at <code>127.0.0.1:{info.port}</code>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

// ---- updates -----------------------------------------------------------------------

type UpdateState =
  | { step: "idle" | "checking" | "latest" | "installing" }
  | { step: "available"; version: string }
  | { step: "downloading"; version: string; pct: number | null }
  | { step: "error"; message: string };

/** Checks GitHub for a newer Holster when asked, and installs it on Update. */
function UpdatesSection({ version }: { version?: string }) {
  const [state, setState] = useState<UpdateState>({ step: "idle" });

  useEffect(
    () =>
      on<{ downloaded: number; total: number | null }>("update-progress", ({ downloaded, total }) =>
        setState((s) =>
          s.step === "downloading" ? { ...s, pct: total ? Math.min(100, Math.round((downloaded / total) * 100)) : null } : s,
        ),
      ),
    [],
  );

  const check = async () => {
    setState({ step: "checking" });
    try {
      const found = await api.updateCheck();
      setState(found ? { step: "available", version: found.version } : { step: "latest" });
    } catch (e) {
      setState({ step: "error", message: String(e) });
    }
  };
  const install = async (version: string) => {
    setState({ step: "downloading", version, pct: 0 });
    try {
      await api.updateInstall();
      setState({ step: "installing" });
    } catch (e) {
      setState({ step: "error", message: String(e) });
    }
  };

  const hint =
    state.step === "checking"
      ? "Checking…"
      : state.step === "latest"
        ? "You have the latest version"
        : state.step === "available"
          ? `Version ${state.version} is ready. Holster restarts by itself`
          : state.step === "downloading"
            ? `Downloading ${state.version}${state.pct === null ? "…" : `, ${state.pct}%`}`
            : state.step === "installing"
              ? "Installing. Holster restarts by itself"
              : state.step === "error"
                ? state.message
                : "Looks for a newer version on GitHub";

  return (
    <Section title="Updates">
      <Row label={version ? `Holster ${version}` : "Holster"} hint={hint} warn={state.step === "error"}>
        {state.step === "available" ? (
          <Button variant="primary" onClick={() => install(state.version)}>
            <Download size={14} /> Update
          </Button>
        ) : (
          <Button
            variant="ghost"
            disabled={state.step === "checking" || state.step === "downloading" || state.step === "installing"}
            onClick={check}
          >
            <RefreshCw size={14} className={state.step === "checking" ? "spin" : ""} /> Check
          </Button>
        )}
      </Row>
    </Section>
  );
}

// ---- apps --------------------------------------------------------------------------

function AppsSection() {
  const apps = useStore((s) => s.apps);
  const loaded = useStore((s) => s.appsLoaded);
  const toast = useStore((s) => s.toast);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }));

  const onDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id) return;
    const from = apps.findIndex((a) => a.id === active.id);
    const to = apps.findIndex((a) => a.id === over.id);
    const next = arrayMove(apps, from, to);
    useStore.setState({ apps: next });
    api.reorderApps(next.map((a) => a.id));
  };

  const add = async () => {
    try {
      const name = await api.addApp();
      if (name) toast(`Added ${name}`);
    } catch (e) {
      toast(String(e));
    }
  };

  return (
    <Section
      title="Apps"
      action={
        <IconButton label="Open the apps folder" className="small" onClick={() => api.openAppsFolder()}>
          <FolderOpen size={14} />
        </IconButton>
      }
    >
      {loaded && apps.length === 0 ? (
        <div className="apps-empty">
          <span className="apps-empty-icon">
            <PackageOpen size={20} />
          </span>
          <span className="apps-empty-title">No apps yet</span>
          <span className="apps-empty-text">Apps add themselves when they start. You can also add a manifest file.</span>
        </div>
      ) : (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          <SortableContext items={apps.map((a) => a.id)} strategy={verticalListSortingStrategy}>
            <div className="app-list">
              <AnimatePresence initial={false}>
                {apps.map((app) => (
                  <AppRow key={app.id} app={app} />
                ))}
              </AnimatePresence>
            </div>
          </SortableContext>
        </DndContext>
      )}
      <div className="apps-actions">
        <Button variant="ghost" onClick={add}>
          <Plus size={14} /> Add app
        </Button>
      </div>
    </Section>
  );
}

function statusOf(app: AppView): { dot: DotState; label: string } {
  if (app.error) return { dot: "error", label: "Manifest error" };
  if (app.status === "running") return { dot: "online", label: "Running" };
  if (app.status === "stopped") return { dot: "off", label: "Not running" };
  if (!app.trackable) return { dot: "off", label: "No status" };
  return { dot: "idle", label: "Checking…" };
}

function AppRow({ app }: { app: AppView }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: app.id });
  const [confirming, setConfirming] = useState(false);
  const toast = useStore((s) => s.toast);
  const updateSettings = useStore((s) => s.updateSettings);
  const status = statusOf(app);

  const setMuted = (muted: boolean) => {
    const current = useStore.getState().settings?.muted ?? [];
    updateSettings({ muted: muted ? [...current.filter((id) => id !== app.id), app.id] : current.filter((id) => id !== app.id) });
  };

  const remove = async () => {
    try {
      await api.removeApp(app.id);
      toast(`Removed ${app.name}`);
    } catch (e) {
      toast(String(e));
      setConfirming(false);
    }
  };

  return (
    <motion.div
      ref={setNodeRef}
      className={`app-row ${isDragging ? "dragging" : ""} ${app.hidden ? "hidden-app" : ""}`}
      style={{ transform: CSS.Translate.toString(transform), transition }}
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      exit={{ opacity: 0, height: 0 }}
      transition={{ type: "spring", stiffness: 420, damping: 40, opacity: { duration: 0.15 } }}
    >
      <div className="app-row-inner">
        <span className="app-grip" {...attributes} {...listeners} aria-label={`Reorder ${app.name}`}>
          <GripVertical size={14} />
        </span>
        <span
          className={`app-tile ${app.error ? "broken" : ""}`}
          style={app.error ? undefined : tileVars(app.color)}
        >
          <AppIcon app={app} size={15} tile={32} />
        </span>
        <span className="app-text">
          <span className="app-name">{app.name}</span>
          <span className="app-sub">
            <Dot state={status.dot} />
            {status.label}
            {app.hidden && !app.error && <span className="app-tag">Hidden</span>}
          </span>
        </span>
        <AnimatePresence mode="popLayout" initial={false}>
          {confirming ? (
            <motion.span
              key="confirm"
              className="inline-confirm"
              initial={{ opacity: 0, x: 8 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: 8 }}
              transition={spring}
            >
              <Button variant="ghost" onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button variant="danger" onClick={remove}>
                Remove
              </Button>
            </motion.span>
          ) : (
            <motion.span
              key="controls"
              className="app-controls"
              initial={{ opacity: 0, x: -8 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: -8 }}
              transition={spring}
            >
              {!app.error && (
                <IconButton
                  label={app.muted ? `Turn on ${app.name} notifications` : `Mute ${app.name} notifications`}
                  className={`small ${app.muted ? "muted" : ""}`}
                  onClick={() => setMuted(!app.muted)}
                >
                  {app.muted ? <BellOff size={14} /> : <Bell size={14} />}
                </IconButton>
              )}
              {!app.error && (
                <Toggle label={`Show ${app.name} in the dock`} on={!app.hidden} onChange={(v) => api.setAppHidden(app.id, !v)} />
              )}
              <IconButton label={`Remove ${app.name}`} className="small danger" onClick={() => setConfirming(true)}>
                <Trash2 size={14} />
              </IconButton>
            </motion.span>
          )}
        </AnimatePresence>
      </div>
      {(app.error || app.warnings.length > 0) && (
        <div className="app-notes">
          {app.error && <p className="app-error">{app.error}</p>}
          {app.warnings.map((w) => (
            <p key={w} className="app-warning">
              {w}
            </p>
          ))}
          {app.error && (
            <p className="app-file" title={app.file}>
              {app.file}
            </p>
          )}
        </div>
      )}
    </motion.div>
  );
}

// ---- hotkey recorder -----------------------------------------------------------------

const KEY_NAMES: Record<string, string> = {
  " ": "Space",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Escape: "Escape",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
};

function keyName(e: KeyboardEvent): string | null {
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return null;
  if (KEY_NAMES[e.key]) return KEY_NAMES[e.key];
  if (/^F\d{1,2}$/.test(e.key)) return e.key;
  if (e.code.startsWith("Key")) return e.code.slice(3);
  if (e.code.startsWith("Digit")) return e.code.slice(5);
  return e.key.length === 1 ? e.key.toUpperCase() : null;
}

function HotkeyRecorder({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [recording, setRecording] = useState(false);
  const [preview, setPreview] = useState("");

  useEffect(() => {
    if (!recording) return;
    const down = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey) {
        setRecording(false);
        return;
      }
      const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
      const k = keyName(e);
      setPreview([...mods, k ?? ""].filter(Boolean).join("+"));
      if (k && mods.length > 0) {
        onChange([...mods, k].join("+"));
        setRecording(false);
      }
    };
    window.addEventListener("keydown", down, true);
    return () => window.removeEventListener("keydown", down, true);
  }, [recording, onChange]);

  return (
    <button className={`hotkey ${recording ? "recording" : ""}`} onClick={() => (setPreview(""), setRecording((r) => !r))}>
      {recording ? preview || "Press keys…" : value.split("+").map((k) => <kbd key={k}>{k}</kbd>)}
    </button>
  );
}
