mod api;
mod apps;
mod commands;
mod diag;
mod fullscreen;
mod health;
mod hooks;
mod notify;
mod process;
mod settings;
mod shell;
mod update;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use serde_json::json;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Theme, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::apps::Registry;
use crate::health::Health;
use crate::notify::Notifier;
use crate::settings::{Prefs, Settings, SettingsStore};
use crate::shell::Shell;

pub const WINDOWS: [&str; 6] = ["main", "gear", "preview", "notify-0", "notify-1", "notify-2"];

pub struct Core {
    pub app: AppHandle,
    pub prefs: Arc<Prefs>,
    pub reg: Arc<Registry>,
    pub health: Arc<Health>,
    pub data_dir: PathBuf,
    pub port: RwLock<u16>,
    shell: OnceLock<Arc<Shell>>,
    notifier: OnceLock<Arc<Notifier>>,
    pub update: update::Pending,
}

impl Core {
    pub fn shell(&self) -> &Arc<Shell> {
        self.shell.get().expect("shell installed")
    }

    pub fn notifier(&self) -> &Arc<Notifier> {
        self.notifier.get().expect("notifier installed")
    }

    /// Apps were added, removed, reordered, hidden or changed status.
    pub fn apps_changed(&self, app: &AppHandle) {
        let _ = app.emit("apps", self.reg.views(&self.prefs.get()));
        if let Some(shell) = self.shell.get() {
            shell.apps_changed();
            shell.statuses_changed();
        }
    }

    /// Side effects of a settings change.
    pub fn apply_settings_change(&self, app: &AppHandle, before: &Settings, after: &Settings) {
        if before.theme != after.theme {
            apply_theme(app, &after.theme);
        }
        if before.launch_at_startup != after.launch_at_startup {
            sync_autostart(app, after.launch_at_startup);
        }
        if before.hide_in_fullscreen != after.hide_in_fullscreen {
            fullscreen::recheck();
        }
        if before.order != after.order || before.hidden != after.hidden || before.muted != after.muted {
            self.apps_changed(app);
        }
        if before.do_not_disturb != after.do_not_disturb || before.muted != after.muted {
            if let Some(n) = self.notifier.get() {
                n.settings_changed();
            }
        }
        if let Some(item) = DND_ITEM.get() {
            let _ = item.set_checked(after.do_not_disturb);
        }
        let _ = app.emit("settings", after);
    }
}

/// The holster folder: `%APPDATA%\Holster`. Apps find the manifests folder
/// and `holster.json` here, so it is part of the contract.
/// `%APPDATA%\Holster`: named after the product, so a differently named
/// build (a test of the updater) keeps its data apart.
fn holster_dir(product: &str) -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(product)
}

fn apply_theme(app: &AppHandle, theme: &str) {
    for label in WINDOWS {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.set_theme(match theme {
                "light" => Some(Theme::Light),
                "dark" => Some(Theme::Dark),
                _ => None,
            });
            apply_material(&w);
        }
    }
}

/// Tints the frosted material for the effective (light/dark) theme.
fn apply_material(w: &tauri::WebviewWindow) {
    let dark = w.theme().map(|t| t == Theme::Dark).unwrap_or(false);
    if let Ok(hwnd) = w.hwnd() {
        // Notification cards grow out of the tab, so they wear its material.
        let card = w.label() != "main" && !w.label().starts_with("notify-");
        if !shell::apply_material(hwnd.0 as isize, dark, card) {
            let tint = if dark { (28, 26, 26, 160) } else { (244, 244, 246, 160) };
            let _ = window_vibrancy::apply_blur(w, Some(tint));
        }
    }
}

fn sync_autostart(app: &AppHandle, enabled: bool) {
    // Dev builds would register target/debug; only touch autostart in release.
    if cfg!(debug_assertions) {
        return;
    }
    let al = app.autolaunch();
    let _ = if enabled { al.enable() } else { al.disable() };
}

static HOTKEY_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn hotkey_error() -> Option<String> {
    HOTKEY_ERROR.lock().unwrap().clone()
}

pub fn register_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let result = register_hotkey_inner(app, hotkey);
    *HOTKEY_ERROR.lock().unwrap() = result.as_ref().err().map(|_| {
        format!("{hotkey} is already used by another app. Record a different shortcut.")
    });
    result
}

fn register_hotkey_inner(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if hotkey.trim().is_empty() {
        return Ok(());
    }
    gs.on_shortcut(hotkey, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            if let Some(core) = app.try_state::<Arc<Core>>() {
                core.shell().toggle();
            }
        }
    })
    .map_err(|e| format!("Couldn't register {hotkey}: {e}"))
}

fn with_shell(app: &AppHandle, f: impl FnOnce(&Arc<Shell>)) {
    if let Some(core) = app.try_state::<Arc<Core>>() {
        f(core.shell());
    }
}

/// The tray's Do not disturb item, kept in sync with Settings.
static DND_ITEM: OnceLock<CheckMenuItem<tauri::Wry>> = OnceLock::new();

fn set_do_not_disturb(app: &AppHandle, on: bool) {
    if let Some(core) = app.try_state::<Arc<Core>>() {
        let before = core.prefs.get();
        let after = core.prefs.update(|s| s.do_not_disturb = on);
        core.apply_settings_change(app, &before, &after);
    }
}

fn build_tray(app: &AppHandle, dnd: bool) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open dock", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quiet = CheckMenuItem::with_id(app, "dnd", "Do not disturb", true, dnd, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Holster", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(app, &[&open, &settings, &sep()?, &quiet, &sep()?, &quit])?;
    let _ = DND_ITEM.set(quiet);
    TrayIconBuilder::with_id("holster")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("Holster")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => with_shell(app, |s| s.open_dock()),
            "settings" => with_shell(app, |s| s.open_panel()),
            "dnd" => {
                let on = app.try_state::<Arc<Core>>().is_some_and(|c| !c.prefs.get().do_not_disturb);
                set_do_not_disturb(app, on);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                with_shell(tray.app_handle(), |s| s.toggle());
            }
        })
        .build(app)?;
    Ok(())
}

/// The settings, preview and notification cards: the same frameless,
/// transparent, always-on-top shell as the main window, created hidden.
fn build_card_window(app: &AppHandle, label: &str) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("Holster")
        .inner_size(10.0, 10.0)
        .min_inner_size(1.0, 1.0)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .visible(false)
        .focused(false)
        .shadow(false)
        .disable_drag_drop_handler()
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            with_shell(app, |s| s.open_dock());
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = holster_dir(&app.package_info().name);
            std::fs::create_dir_all(&data_dir)?;
            diag::init(&data_dir);
            diag::log(format!("Holster {} started", app.package_info().version));
            let prefs = Arc::new(Prefs::new(SettingsStore::new(&data_dir)));
            let settings = prefs.get();

            for label in &WINDOWS[1..] {
                build_card_window(&handle, label)?;
            }

            let reg_handle = handle.clone();
            let reg = Registry::new(data_dir.join("apps"), move || {
                if let Some(core) = reg_handle.try_state::<Arc<Core>>() {
                    core.apps_changed(&reg_handle);
                }
            });
            let health = Health::new(reg.clone());

            let core = Arc::new(Core {
                app: handle.clone(),
                prefs: prefs.clone(),
                reg: reg.clone(),
                health: health.clone(),
                data_dir: data_dir.clone(),
                port: RwLock::new(0),
                shell: OnceLock::new(),
                notifier: OnceLock::new(),
                update: Default::default(),
            });
            app.manage(core.clone());

            // Windows: real acrylic behind transparent webviews.
            apply_theme(&handle, &settings.theme);
            let shell = Shell::install(&handle, prefs.clone(), reg.clone(), health.clone());
            let _ = core.shell.set(shell.clone());
            let notifier = Notifier::new(handle.clone(), shell.clone(), reg.clone(), prefs.clone(), health.clone());
            let _ = core.notifier.set(notifier.clone());
            {
                let (a, b) = (shell.clone(), shell.clone());
                hooks::start(shell.hwnds(), move || a.on_outside_click(), move || {
                    if matches!(b.mode(), shell::Mode::Dock | shell::Mode::Opening) {
                        b.close();
                    }
                });
            }
            fullscreen::start(shell.clone(), prefs.clone());
            health.start();
            apps::watch(reg.clone());

            for label in WINDOWS {
                let w = app.get_webview_window(label).expect("window");
                let (shell, win) = (shell.clone(), w.clone());
                w.on_window_event(move |event| match event {
                    WindowEvent::ThemeChanged(_) => apply_material(&win),
                    WindowEvent::Focused(false) if win.label() == "main" => shell.on_blur(),
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        shell.close();
                    }
                    _ => {}
                });
            }

            if let Err(e) = register_hotkey(&handle, &settings.hotkey) {
                eprintln!("{e}");
            }
            sync_autostart(&handle, settings.launch_at_startup);
            build_tray(&handle, settings.do_not_disturb)?;

            // The local API for apps, and the discovery file that points at it.
            let api_core = core.clone();
            let signal_handle = handle.clone();
            let signal_reg = reg.clone();
            tauri::async_runtime::spawn(async move {
                let version = signal_handle.package_info().version.to_string();
                let emit = signal_handle.clone();
                let result = api::start(api_core.data_dir.clone(), version, signal_reg.clone(), notifier, move |id| {
                    if let Some(m) = signal_reg.manifest(id) {
                        let _ = emit.emit_to("main", "signal", json!({ "id": id, "color": m.color }));
                    }
                })
                .await;
                match result {
                    Ok(port) => {
                        *api_core.port.write().unwrap() = port;
                        api_core.reg.set_api_port(port);
                    }
                    Err(e) => eprintln!("Holster API failed to start: {e}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::shell_state,
            commands::shell_open_dock,
            commands::shell_open_panel,
            commands::shell_close,
            commands::shell_hover,
            commands::shell_drag,
            commands::preview_state,
            commands::preview_show,
            commands::preview_hide,
            commands::preview_resize,
            commands::preview_failed,
            commands::diag_log,
            commands::update_check,
            commands::update_install,
            commands::list_apps,
            commands::open_app,
            commands::reorder_apps,
            commands::set_app_hidden,
            commands::remove_app,
            commands::add_app,
            commands::open_apps_folder,
            commands::get_settings,
            commands::update_settings,
            commands::reset_tab_position,
            commands::app_info,
            commands::note_slots,
            commands::note_measured,
            commands::note_resize,
            commands::note_hover,
            commands::note_drag,
            commands::note_action,
            commands::note_dismiss,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Holster");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(core) = app.try_state::<Arc<Core>>() {
                api::remove_discovery(&core.data_dir);
            }
        }
    });
}
