//! Tauri commands called by the UI. All are async so they never run on (and
//! never block) the window thread.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::apps::{self, AppView, OpenMode, Status};
use crate::notify::{Content, Request, SlotView};
use crate::process;
use crate::settings::Settings;
use crate::shell::{PreviewState, ShellState};
use crate::Core;

type Res<T> = Result<T, String>;
type C<'a> = State<'a, Arc<Core>>;

// ---- shell -------------------------------------------------------------------

#[tauri::command]
pub async fn shell_state(core: C<'_>) -> Res<ShellState> {
    Ok(core.shell().snapshot())
}

#[tauri::command]
pub async fn shell_open_dock(core: C<'_>) -> Res<()> {
    core.shell().open_dock();
    Ok(())
}

#[tauri::command]
pub async fn shell_open_panel(core: C<'_>) -> Res<()> {
    core.shell().open_panel();
    Ok(())
}

#[tauri::command]
pub async fn shell_close(core: C<'_>) -> Res<()> {
    core.shell().close();
    Ok(())
}

#[tauri::command]
pub async fn shell_hover(core: C<'_>, on: bool) -> Res<()> {
    core.shell().hover(on);
    Ok(())
}

/// Returns true at "end" when the gesture moved (so it isn't a click).
#[tauri::command]
pub async fn shell_drag(core: C<'_>, phase: String) -> Res<bool> {
    Ok(core.shell().drag(&phase))
}

#[tauri::command]
pub async fn preview_state(core: C<'_>) -> Res<PreviewState> {
    Ok(core.shell().preview_state())
}

#[tauri::command]
pub async fn preview_show(core: C<'_>, id: String, anchor_y: f64) -> Res<()> {
    core.shell().show_preview(&id, anchor_y);
    Ok(())
}

#[tauri::command]
pub async fn preview_hide(core: C<'_>) -> Res<()> {
    core.shell().hide_preview();
    Ok(())
}

#[tauri::command]
pub async fn preview_resize(core: C<'_>, id: String, width: f64, height: f64) -> Res<()> {
    if width.is_finite() && height.is_finite() {
        core.shell().resize_preview(&id, width, height);
    }
    Ok(())
}

#[tauri::command]
pub async fn preview_failed(core: C<'_>, id: String, failed: bool) -> Res<()> {
    core.shell().preview_failed(&id, failed);
    Ok(())
}

// ---- apps ----------------------------------------------------------------------

#[tauri::command]
pub async fn list_apps(core: C<'_>) -> Res<Vec<AppView>> {
    Ok(core.reg.views(&core.prefs.get()))
}

/// Opens an app the way its manifest says. `anchor_y` is the tile's center in
/// the main window's CSS px. The dock closes once the app is open; on failure
/// the reason shows on the tile's preview card instead.
#[tauri::command]
pub async fn open_app(core: C<'_>, id: String, anchor_y: f64) -> Res<()> {
    let shell = core.shell().clone();
    let result = open_app_inner(&core, &id, shell.screen_y(anchor_y), shell.tab_y()).await;
    match &result {
        Ok(Some(pids)) => {
            shell.cover_with(pids.clone());
            shell.close();
        }
        Ok(None) => shell.close(),
        Err(e) => shell.show_error(&id, anchor_y, e.clone()),
    }
    core.health.poke();
    result.map(|_| ())
}

/// Opens the app. For a panel app, returns its processes so the tab can step
/// aside while the panel is out. A panel opens from `anchor_screen_y` and
/// should close toward `tab_y`, where the tab comes back.
async fn open_app_inner(core: &Core, id: &str, anchor_screen_y: f64, tab_y: f64) -> Res<Option<Vec<u32>>> {
    let m = core.reg.manifest(id).ok_or("This app's manifest has an error. See Settings, Apps.")?;
    match m.mode {
        OpenMode::Window => {
            let launch = m.launch.clone().ok_or("No launch section")?;
            let exe = launch.exe.clone();
            let window = tokio::task::spawn_blocking(move || process::main_window(&process::pids_for(&exe)))
                .await
                .map_err(|e| e.to_string())?;
            match window {
                Some(hwnd) => process::focus(hwnd),
                None => process::launch(&launch.exe, &launch.args)?,
            }
            Ok(None)
        }
        OpenMode::Panel => {
            if core.health.check(&m).await != Status::Running {
                let launch = m.launch.clone().ok_or(format!("{} isn't running and its manifest has no launch section", m.name))?;
                core.reg.set_status(&m.id, Status::Stopped);
                let _ = core.app.emit_to("main", "launching", json!({ "id": m.id, "on": true }));
                let started = process::launch(&launch.exe, &launch.args);
                let ready = match started {
                    Ok(()) => core.health.wait_running(&m, Duration::from_secs(20)).await,
                    Err(e) => {
                        let _ = core.app.emit_to("main", "launching", json!({ "id": m.id, "on": false }));
                        return Err(e);
                    }
                };
                let _ = core.app.emit_to("main", "launching", json!({ "id": m.id, "on": false }));
                if !ready {
                    return Err(format!("{} started but didn't answer in time", m.name));
                }
            }
            let url = m.open_url.clone().ok_or("No open.url")?;
            process::allow_foreground();
            let edge = crate::shell::Edge::from_setting(&core.prefs.get().edge).name();
            let resp = core
                .health
                .client()
                .post(&url)
                .json(&json!({ "edge": edge, "anchorY": anchor_screen_y, "tabY": tab_y }))
                .timeout(Duration::from_secs(4))
                .send()
                .await
                .map_err(|_| format!("{} didn't respond to open", m.name))?;
            if !resp.status().is_success() {
                return Err(format!("{} couldn't open its panel ({})", m.name, resp.status().as_u16()));
            }
            let port = reqwest::Url::parse(&url).ok().and_then(|u| u.port_or_known_default());
            let exe = m.launch.as_ref().map(|l| l.exe.clone());
            let pids = tokio::task::spawn_blocking(move || {
                let mut pids: Vec<u32> = port.and_then(process::listener_pid).into_iter().collect();
                if let Some(exe) = exe {
                    pids.extend(process::pids_for(&exe).into_iter().filter(|p| !pids.contains(p)).collect::<Vec<_>>());
                }
                pids
            })
            .await
            .unwrap_or_default();
            Ok(Some(pids))
        }
    }
}

#[tauri::command]
pub async fn reorder_apps(app: AppHandle, core: C<'_>, ids: Vec<String>) -> Res<()> {
    core.prefs.update(|s| s.order = ids);
    core.apps_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_app_hidden(app: AppHandle, core: C<'_>, id: String, hidden: bool) -> Res<()> {
    core.prefs.update(|s| {
        s.hidden.retain(|h| h != &id);
        if hidden {
            s.hidden.push(id);
        }
    });
    core.apps_changed(&app);
    Ok(())
}

/// Deletes an app's manifest. The watcher picks the change up.
#[tauri::command]
pub async fn remove_app(core: C<'_>, id: String) -> Res<()> {
    let entry = core.reg.entries().into_iter().find(|e| e.id == id).ok_or("Unknown app")?;
    std::fs::remove_file(&entry.file).map_err(|e| format!("Couldn't remove {}: {e}", entry.file.display()))?;
    core.reg.reload();
    Ok(())
}

/// Picks a manifest file, validates it and copies it into the apps folder.
/// Returns the app's name, or None when the dialog was cancelled.
#[tauri::command]
pub async fn add_app(app: AppHandle, core: C<'_>) -> Res<Option<String>> {
    core.shell().suppress_blur(true);
    let picked = app
        .dialog()
        .file()
        .set_title("Add an app to Holster")
        .add_filter("Holster manifest", &["json"])
        .blocking_pick_file();
    core.shell().suppress_blur(false);
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let text = text.trim_start_matches('\u{feff}');
    let id = serde_json::from_str::<Value>(text)
        .map_err(|e| format!("Not valid JSON: {e}"))?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or("The manifest has no id")?;
    let (manifest, _) = apps::parse(text, &id);
    let manifest = manifest?;
    let dest = core.reg.dir.join(format!("{id}.json"));
    std::fs::write(&dest, text).map_err(|e| format!("Couldn't save the manifest: {e}"))?;
    core.reg.reload();
    Ok(Some(manifest.name))
}

#[tauri::command]
pub async fn open_apps_folder(app: AppHandle, core: C<'_>) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path(core.reg.dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

// ---- settings --------------------------------------------------------------------

#[tauri::command]
pub async fn get_settings(core: C<'_>) -> Res<Settings> {
    Ok(core.prefs.get())
}

/// Applies a partial settings object (same shape as `Settings`, camelCase).
#[tauri::command]
pub async fn update_settings(app: AppHandle, core: C<'_>, patch: Value) -> Res<Settings> {
    let before = core.prefs.get();
    let mut merged = serde_json::to_value(&before).map_err(|e| e.to_string())?;
    merge(&mut merged, patch);
    let next: Settings = serde_json::from_value(merged).map_err(|e| e.to_string())?;
    if next.hotkey != before.hotkey {
        // A taken shortcut is reported in the UI rather than rejected, so the
        // user sees which combination failed.
        let _ = crate::register_hotkey(&app, &next.hotkey);
    }
    let after = core.prefs.update(|s| *s = next);
    core.apply_settings_change(&app, &before, &after);
    Ok(after)
}

fn merge(a: &mut Value, b: Value) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

#[tauri::command]
pub async fn reset_tab_position(app: AppHandle, core: C<'_>) -> Res<()> {
    let before = core.prefs.get();
    let after = core.prefs.update(|s| s.tab = Default::default());
    core.apply_settings_change(&app, &before, &after);
    core.shell().relayout();
    Ok(())
}

#[tauri::command]
pub async fn app_info(app: AppHandle, core: C<'_>) -> Res<Value> {
    Ok(json!({
        "version": app.package_info().version.to_string(),
        "dataDir": core.data_dir,
        "appsDir": core.reg.dir,
        "port": *core.port.read().unwrap(),
        "hotkeyError": crate::hotkey_error(),
    }))
}

// ---- notifications ---------------------------------------------------------------

#[tauri::command]
pub async fn note_slots(core: C<'_>) -> Res<Vec<Option<SlotView>>> {
    Ok(core.notifier().slots())
}

/// A text card's drawn height (CSS px).
#[tauri::command]
pub async fn note_measured(core: C<'_>, key: u64, height: f64) -> Res<()> {
    core.notifier().measured(key, height);
    Ok(())
}

/// A page card asked for a new size.
#[tauri::command]
pub async fn note_resize(core: C<'_>, key: u64, width: f64, height: f64) -> Res<()> {
    core.notifier().resize(key, width, height);
    Ok(())
}

#[tauri::command]
pub async fn note_hover(core: C<'_>, key: u64, on: bool) -> Res<()> {
    core.notifier().hover(key, on);
    Ok(())
}

/// Flicking a card into the edge: "move" with the distance, then "end".
#[tauri::command]
pub async fn note_drag(core: C<'_>, key: u64, phase: String, dx: f64) -> Res<()> {
    core.notifier().drag(key, &phase, dx);
    Ok(())
}

#[tauri::command]
pub async fn note_action(core: C<'_>, key: u64, button: String) -> Res<()> {
    core.notifier().action(key, &button);
    Ok(())
}

#[tauri::command]
pub async fn note_dismiss(core: C<'_>, key: u64) -> Res<()> {
    core.notifier().dismiss(key);
    Ok(())
}

/// The card was clicked: it closes and its app opens the way the manifest
/// says, its panel unfolding from the card's height. If that fails, the
/// reason comes back as a notification.
#[tauri::command]
pub async fn note_open(core: C<'_>, key: u64) -> Res<()> {
    let Some((id, cy)) = core.notifier().clicked(key) else { return Ok(()) };
    let shell = core.shell().clone();
    match open_app_inner(&core, &id, cy, shell.tab_y()).await {
        Ok(Some(pids)) => shell.cover_with(pids),
        Ok(None) => {}
        Err(e) => {
            let name = core.reg.manifest(&id).map(|m| m.name).unwrap_or(id.clone());
            let content = Content::Text { title: format!("{name} didn't open"), body: Some(e), buttons: Vec::new() };
            core.notifier().post(&id, Request { id: None, content, timeout: Some(Duration::from_secs(6)) });
        }
    }
    core.health.poke();
    Ok(())
}
