//! Updates from GitHub Releases. Each release carries `latest.json` (the
//! newest version, its installer and the installer's signature); Holster
//! checks it when asked, verifies the download against the public key in
//! `tauri.conf.json`, and runs the installer, which replaces Holster and
//! starts it again. Nothing happens without the user pressing Update.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    pub notes: Option<String>,
}

/// The update found by the last check, ready to install.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

/// Asks GitHub for the newest release. None: this is the newest.
pub async fn check(app: &AppHandle, pending: &Pending, data_dir: PathBuf) -> Result<Option<Available>, String> {
    // The installer can only replace Holster once it has exited, so the
    // updater ends the process itself: tidy up first, as a normal exit would.
    let handle = app.clone();
    let found = app
        .updater_builder()
        .on_before_exit(move || {
            crate::diag::log("exiting for the installer");
            crate::api::remove_discovery(&data_dir);
            handle.remove_tray_by_id("holster");
        })
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| {
            crate::diag::log(format!("update check failed: {e}"));
            format!("Couldn't check for updates ({e})")
        })?;
    let available = found.as_ref().map(|u| Available { version: u.version.clone(), notes: u.body.clone() });
    if let Some(a) = &available {
        crate::diag::log(format!("update check: {} is available", a.version));
    }
    *pending.0.lock().unwrap() = found;
    Ok(available)
}

/// Downloads the update found by `check`, verifies it and runs its
/// installer. Progress goes to the UI as `update-progress`. On Windows the
/// installer takes over from here: Holster exits and the new version starts.
pub async fn install(app: &AppHandle, pending: &Pending) -> Result<(), String> {
    let Some(update) = pending.0.lock().unwrap().take() else {
        return Err("Check for updates first".into());
    };
    crate::diag::log(format!("updating to {}", update.version));
    let mut downloaded: u64 = 0;
    let progress = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress.emit("update-progress", json!({ "downloaded": downloaded, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| {
            crate::diag::log(format!("update failed: {e}"));
            format!("The update didn't install ({e})")
        })
}
