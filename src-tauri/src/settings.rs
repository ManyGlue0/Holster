//! User settings, persisted as JSON in `%APPDATA%\Holster\settings.json`.
//!
//! Dock order and hidden apps live here, not in the manifests: apps rewrite
//! their own manifests whenever they start, and must never undo the user's
//! arrangement.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_HOTKEY: &str = "Ctrl+Shift+H";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    pub hotkey: String,
    /// Game mode: tuck the tab away while a fullscreen app is in front.
    pub hide_in_fullscreen: bool,
    pub launch_at_startup: bool,
    /// Which screen edge Holster lives on. Only "right" is implemented; every
    /// piece of geometry reads it through `shell::Edge` so adding "left" is local.
    pub edge: String,
    pub tab: TabSettings,
    /// Dock order by app id. Apps not listed yet go after these, by name.
    pub order: Vec<String>,
    /// App ids hidden from the dock (still listed in Settings).
    pub hidden: Vec<String>,
    /// App ids whose notifications are dropped.
    pub muted: Vec<String>,
    /// Notifications wait (like during a fullscreen app) until this is off.
    pub do_not_disturb: bool,
    /// A soft system sound when a notification comes in.
    pub notify_sound: bool,
    pub delays: Delays,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TabSettings {
    /// Vertical position of the tab's center as a fraction of the work area height.
    pub y_frac: f64,
}

/// Hover timings in milliseconds (Settings, Behavior, Advanced).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Delays {
    /// Hover intent before the tab opens the dock.
    pub dock: u32,
    /// Hover intent before a tile shows its preview.
    pub preview: u32,
    /// Grace period before a preview hides once the cursor leaves it.
    pub preview_grace: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            hotkey: DEFAULT_HOTKEY.into(),
            hide_in_fullscreen: true,
            launch_at_startup: true,
            edge: "right".into(),
            tab: TabSettings::default(),
            order: Vec::new(),
            hidden: Vec::new(),
            muted: Vec::new(),
            do_not_disturb: false,
            notify_sound: false,
            delays: Delays::default(),
        }
    }
}

impl Default for TabSettings {
    fn default() -> Self {
        Self { y_frac: 0.42 }
    }
}

impl Default for Delays {
    fn default() -> Self {
        Self { dock: 300, preview: 150, preview_grace: 250 }
    }
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("settings.json") }
    }

    pub fn load(&self) -> Settings {
        let mut s: Settings =
            std::fs::read_to_string(&self.path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        s.tab.y_frac = s.tab.y_frac.clamp(0.0, 1.0);
        if s.edge != "right" {
            s.edge = "right".into();
        }
        s.delays.dock = s.delays.dock.clamp(0, 1500);
        s.delays.preview = s.delays.preview.clamp(0, 1500);
        s.delays.preview_grace = s.delays.preview_grace.clamp(50, 2000);
        if !self.path.exists() {
            let _ = self.save(&s);
        }
        s
    }

    /// Atomic write: temp file then rename, so a crash never leaves half a file.
    pub fn save(&self, s: &Settings) -> anyhow::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Live settings shared across the app, saved on every change.
pub struct Prefs {
    current: std::sync::RwLock<Settings>,
    store: SettingsStore,
}

impl Prefs {
    pub fn new(store: SettingsStore) -> Self {
        Self { current: std::sync::RwLock::new(store.load()), store }
    }

    pub fn get(&self) -> Settings {
        self.current.read().unwrap().clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = self.current.write().unwrap();
        f(&mut s);
        let _ = self.store.save(&s);
        s.clone()
    }

    /// Updates in memory only (e.g. during a drag); call `update` to persist.
    pub fn update_volatile(&self, f: impl FnOnce(&mut Settings)) {
        f(&mut self.current.write().unwrap());
    }
}
