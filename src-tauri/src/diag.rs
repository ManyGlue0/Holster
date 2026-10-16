//! A small diagnostic log, `%APPDATA%\Holster\holster.log`, for problems that
//! only show up after hours of use: what a window reported, and the
//! decisions Holster made around it. One line per event, local time first.
//! It never grows past `MAX_BYTES`: the older half moves to `holster.log.old`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 256 * 1024;

static FILE: OnceLock<Mutex<PathBuf>> = OnceLock::new();

pub fn init(dir: &std::path::Path) {
    let _ = FILE.set(Mutex::new(dir.join("holster.log")));
}

fn now() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// Appends one line. Never fails loudly: a log that can't be written is skipped.
pub fn log(line: impl AsRef<str>) {
    let Some(path) = FILE.get() else { return };
    let path = path.lock().unwrap();
    if std::fs::metadata(&*path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&*path, path.with_extension("log.old"));
    }
    let line: String = line.as_ref().chars().filter(|c| *c != '\r' && *c != '\n').take(500).collect();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&*path) {
        let _ = writeln!(f, "{} {}", now(), line);
    }
}
