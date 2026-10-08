//! The app registry: one manifest per app in `%APPDATA%\Holster\apps\<id>.json`.
//!
//! Manifests are read, validated and watched live. A broken manifest never
//! takes anything down: it becomes an entry with an error, shown in Settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;

use crate::settings::Settings;

pub const SCHEMA: u64 = 1;

/// Preview size limits in CSS px. Apps choose within these.
pub const PREVIEW_MIN: (f64, f64) = (140.0, 64.0);
pub const PREVIEW_MAX: (f64, f64) = (720.0, 720.0);
/// The card that explains why opening an app failed.
pub const FALLBACK_SIZE: (f64, f64) = (248.0, 76.0);

const PALETTE: [&str; 10] = ["mint", "peach", "lavender", "sky", "butter", "rose", "sage", "lilac", "apricot", "mist"];

#[derive(Debug, Clone, PartialEq)]
pub enum Icon {
    Lucide(String),
    /// Image file rendered as a data URL (the webview can't read local
    /// paths), drawn on the tile at `size` percent of the tile.
    Image(String, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenMode {
    Panel,
    Window,
}

/// A preview is a static page on disk (shown whether or not the app runs,
/// served by Holster) and/or a live page served by the running app.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub path: Option<PathBuf>,
    pub url: Option<String>,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub exe: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub icon: Icon,
    pub color: String,
    pub health: Option<String>,
    pub preview: Option<Preview>,
    pub mode: OpenMode,
    pub open_url: Option<String>,
    pub launch: Option<Launch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Unknown,
    Running,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub file: PathBuf,
    pub manifest: Result<Manifest, String>,
    pub warnings: Vec<String>,
}

/// What the UI gets for each app.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub id: String,
    pub file: String,
    pub name: String,
    /// Lucide icon name, when the icon is one.
    pub lucide: Option<String>,
    /// Data URL, when the icon is an image file.
    pub image: Option<String>,
    /// How much of the tile the image covers, in percent.
    pub image_size: f64,
    pub color: String,
    pub mode: Option<OpenMode>,
    pub has_preview: bool,
    /// The live page, served by the app while it runs.
    pub preview_url: Option<String>,
    /// The page on disk, served by Holster (works when the app isn't running).
    pub preview_file: Option<String>,
    pub can_launch: bool,
    /// Holster has a way to tell whether it's running (health, launch or preview).
    pub trackable: bool,
    pub status: Status,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub hidden: bool,
}

// ---- validation ---------------------------------------------------------------------

fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_' | b'.'))
}

/// Holster only ever talks to `http://127.0.0.1[:port]/...`.
pub fn loopback_url(u: &str) -> bool {
    let Some(rest) = u.strip_prefix("http://127.0.0.1") else { return false };
    let rest = match rest.strip_prefix(':') {
        Some(r) => {
            let digits = r.bytes().take_while(u8::is_ascii_digit).count();
            match r[..digits].parse::<u32>() {
                Ok(p) if (1..=65535).contains(&p) => &r[digits..],
                _ => return false,
            }
        }
        None => rest,
    };
    rest.is_empty() || rest.starts_with('/') || rest.starts_with('?')
}

fn valid_color(c: &str) -> bool {
    if PALETTE.contains(&c) {
        return true;
    }
    let Some(hex) = c.strip_prefix('#') else { return false };
    matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

fn lucide_names() -> &'static std::collections::HashSet<&'static str> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| include_str!("lucide-names.txt").lines().filter(|l| !l.is_empty()).collect())
}

fn image_data_url(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        _ => return Err(format!("icon.path must be a .png, .svg, .jpg, .webp or .ico file (got \"{}\")", path.display())),
    };
    let bytes = std::fs::read(path).map_err(|e| format!("icon.path can't be read ({}): {e}", path.display()))?;
    if bytes.len() > 1024 * 1024 {
        return Err("icon.path is larger than 1 MB".into());
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// `icon.size`: how much of the tile the image covers, as a percentage
/// ("80%", or a bare 80). Defaults to 80%.
pub const ICON_SIZE_DEFAULT: f64 = 80.0;

fn icon_size(v: Option<&Value>) -> Result<f64, String> {
    let pct = match v {
        None | Some(Value::Null) => return Ok(ICON_SIZE_DEFAULT),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().strip_suffix('%').and_then(|n| n.trim().parse::<f64>().ok()),
        Some(_) => None,
    };
    match pct {
        Some(p) if (10.0..=100.0).contains(&p) => Ok(p),
        _ => Err("icon.size must be a percentage of the tile between 10% and 100%, like \"80%\"".into()),
    }
}

fn str_field<'a>(v: &'a Value, key: &str, ctx: &str) -> Result<Option<&'a str>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("{ctx}{key} must be a string")),
    }
}

/// Parses and validates one manifest. `stem` is the file name without `.json`.
pub fn parse(text: &str, stem: &str) -> (Result<Manifest, String>, Vec<String>) {
    let mut warnings = Vec::new();
    let r = (|| -> Result<Manifest, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("Not valid JSON: {e}"))?;
        if !v.is_object() {
            return Err("The manifest must be a JSON object".into());
        }
        match v.get("schema").and_then(Value::as_u64) {
            Some(SCHEMA) => {}
            Some(n) => return Err(format!("schema {n} isn't supported (this Holster understands schema {SCHEMA})")),
            None => return Err("schema is missing (use \"schema\": 1)".into()),
        }
        let id = str_field(&v, "id", "")?.ok_or("id is missing")?.to_string();
        if !valid_id(&id) {
            return Err(format!("id \"{id}\" must be lowercase letters, digits, '-', '_' or '.' (max 64)"));
        }
        if id != stem {
            return Err(format!("id \"{id}\" doesn't match the file name ({stem}.json)"));
        }
        let name = str_field(&v, "name", "")?.map(str::trim).unwrap_or("").to_string();
        if name.is_empty() {
            return Err("name is missing".into());
        }

        let icon_v = v.get("icon").ok_or("icon is missing")?;
        let icon = if let Some(n) = str_field(icon_v, "lucide", "icon.")? {
            if !lucide_names().contains(n) {
                return Err(format!("icon.lucide \"{n}\" isn't a Lucide icon name (see lucide.dev/icons)"));
            }
            Icon::Lucide(n.to_string())
        } else if let Some(p) = str_field(icon_v, "path", "icon.")? {
            let path = Path::new(p);
            if !path.is_absolute() {
                return Err("icon.path must be an absolute path".into());
            }
            let size = icon_size(icon_v.get("size"))?;
            match image_data_url(path) {
                Ok(d) => Icon::Image(d, size),
                Err(e) => {
                    warnings.push(format!("{e}; showing a default icon"));
                    Icon::Lucide("app-window".into())
                }
            }
        } else {
            return Err("icon needs either \"lucide\" or \"path\"".into());
        };

        let color = str_field(&v, "color", "")?.ok_or("color is missing")?.trim().to_ascii_lowercase();
        if !valid_color(&color) {
            return Err(format!("color \"{color}\" must be a hex color like #B8A9F0"));
        }

        let health = str_field(&v, "health", "")?.map(str::to_string);
        if let Some(h) = &health {
            if !loopback_url(h) {
                return Err("health must be an http://127.0.0.1 URL".into());
            }
        }

        let preview = match v.get("preview") {
            None | Some(Value::Null) => None,
            Some(p) => {
                let url = str_field(p, "url", "preview.")?.map(str::to_string);
                if url.as_deref().is_some_and(|u| !loopback_url(u)) {
                    return Err("preview.url must be an http://127.0.0.1 URL".into());
                }
                let mut path = str_field(p, "path", "preview.")?.map(PathBuf::from);
                if path.is_none() && url.is_none() {
                    return Err("preview needs a \"path\" (a page on disk) or a \"url\" (a live page)".into());
                }
                if let Some(f) = &path {
                    if !f.is_absolute() {
                        return Err("preview.path must be an absolute path".into());
                    }
                    if !f.is_file() {
                        warnings.push(format!("preview.path wasn't found at {}", f.display()));
                        path = None;
                    }
                }
                let dim = |k: &str, min: f64, max: f64, def: f64, warnings: &mut Vec<String>| -> Result<f64, String> {
                    match p.get(k) {
                        None | Some(Value::Null) => Ok(def),
                        Some(Value::Number(n)) => {
                            let x = n.as_f64().unwrap_or(def);
                            if x < min || x > max {
                                warnings.push(format!("preview.{k} {x} is outside {min}–{max}; using {}", x.clamp(min, max)));
                            }
                            Ok(x.clamp(min, max).round())
                        }
                        Some(_) => Err(format!("preview.{k} must be a number")),
                    }
                };
                let width = dim("width", PREVIEW_MIN.0, PREVIEW_MAX.0, 320.0, &mut warnings)?;
                let height = dim("height", PREVIEW_MIN.1, PREVIEW_MAX.1, 240.0, &mut warnings)?;
                (path.is_some() || url.is_some()).then_some(Preview { path, url, width, height })
            }
        };

        let launch = match v.get("launch") {
            None | Some(Value::Null) => None,
            Some(l) => {
                let exe = str_field(l, "exe", "launch.")?.ok_or("launch.exe is missing")?;
                let exe = PathBuf::from(exe);
                if !exe.is_absolute() {
                    return Err("launch.exe must be an absolute path".into());
                }
                if !exe.is_file() {
                    warnings.push(format!("launch.exe wasn't found at {}", exe.display()));
                }
                let args = match l.get("args") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(a)) => a
                        .iter()
                        .map(|x| x.as_str().map(str::to_string).ok_or("launch.args must be a list of strings"))
                        .collect::<Result<_, _>>()?,
                    Some(_) => return Err("launch.args must be a list of strings".into()),
                };
                Some(Launch { exe, args })
            }
        };

        let open = v.get("open").ok_or("open is missing")?;
        let (mode, open_url) = match str_field(open, "mode", "open.")? {
            Some("panel") => {
                let url = str_field(open, "url", "open.")?.ok_or("open.url is required for \"panel\" mode")?;
                if !loopback_url(url) {
                    return Err("open.url must be an http://127.0.0.1 URL".into());
                }
                if launch.is_none() {
                    warnings.push("No launch section: Holster can't start this app when it isn't running".into());
                }
                (OpenMode::Panel, Some(url.to_string()))
            }
            Some("window") => {
                if launch.is_none() {
                    return Err("open.mode \"window\" needs a launch section".into());
                }
                (OpenMode::Window, None)
            }
            Some(m) => return Err(format!("open.mode \"{m}\" must be \"panel\" or \"window\"")),
            None => return Err("open.mode is missing".into()),
        };

        Ok(Manifest { id, name, icon, color, health, preview, mode, open_url, launch })
    })();
    (r, warnings)
}

fn read_dir(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("json")) != Some(true) {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        // A writer may be mid-way through replacing the file: retry briefly.
        let mut text = None;
        for _ in 0..3 {
            match std::fs::read_to_string(&path) {
                Ok(t) if !t.trim().is_empty() => {
                    text = Some(Ok(t));
                    break;
                }
                Ok(_) => text = Some(Err("The file is empty".to_string())),
                Err(err) => text = Some(Err(format!("Can't read the file: {err}"))),
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        let (manifest, warnings) = match text.unwrap() {
            Ok(t) => parse(t.trim_start_matches('\u{feff}'), &stem),
            Err(e) => (Err(e), Vec::new()),
        };
        out.push(Entry { id: stem, file: path, manifest, warnings });
    }
    out
}

// ---- registry --------------------------------------------------------------------------

pub struct Registry {
    pub dir: PathBuf,
    entries: RwLock<Vec<Entry>>,
    status: RwLock<HashMap<String, Status>>,
    /// Preview sizes requested at runtime by the preview page (postMessage).
    preview_size: RwLock<HashMap<String, (f64, f64)>>,
    /// Holster's API port, for the URLs of on-disk previews.
    api_port: std::sync::atomic::AtomicU16,
    on_change: Box<dyn Fn() + Send + Sync>,
}

impl Registry {
    pub fn new(dir: PathBuf, on_change: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&dir);
        let me = Arc::new(Self {
            dir,
            entries: RwLock::new(Vec::new()),
            status: RwLock::new(HashMap::new()),
            preview_size: RwLock::new(HashMap::new()),
            api_port: std::sync::atomic::AtomicU16::new(0),
            on_change: Box::new(on_change),
        });
        me.reload();
        me
    }

    /// Re-reads every manifest. Returns true when something changed.
    pub fn reload(&self) -> bool {
        let next = read_dir(&self.dir);
        let changed = {
            let mut cur = self.entries.write().unwrap();
            let same = cur.len() == next.len()
                && cur.iter().zip(&next).all(|(a, b)| a.file == b.file && a.manifest == b.manifest && a.warnings == b.warnings);
            if !same {
                // A changed manifest may point at a new preview: forget runtime sizes.
                let mut sizes = self.preview_size.write().unwrap();
                for n in &next {
                    let old = cur.iter().find(|c| c.id == n.id).and_then(|c| c.manifest.as_ref().ok()).map(|m| &m.preview);
                    if old != n.manifest.as_ref().ok().map(|m| &m.preview) {
                        sizes.remove(&n.id);
                    }
                }
                *cur = next;
            }
            !same
        };
        if changed {
            (self.on_change)();
        }
        changed
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.entries.read().unwrap().clone()
    }

    pub fn manifest(&self, id: &str) -> Option<Manifest> {
        self.entries.read().unwrap().iter().find(|e| e.id == id).and_then(|e| e.manifest.clone().ok())
    }

    pub fn status(&self, id: &str) -> Status {
        self.status.read().unwrap().get(id).copied().unwrap_or(Status::Unknown)
    }

    /// Records health results; notifies when any status changed.
    pub fn set_statuses(&self, next: HashMap<String, Status>) {
        let changed = {
            let mut cur = self.status.write().unwrap();
            let changed = *cur != next;
            *cur = next;
            changed
        };
        if changed {
            (self.on_change)();
        }
    }

    pub fn set_status(&self, id: &str, s: Status) {
        let changed = self.status.write().unwrap().insert(id.to_string(), s) != Some(s);
        if changed {
            (self.on_change)();
        }
    }

    pub fn set_api_port(&self, port: u16) {
        self.api_port.store(port, std::sync::atomic::Ordering::SeqCst);
        (self.on_change)();
    }

    /// The folder an on-disk preview is served from.
    pub fn preview_dir(&self, id: &str) -> Option<PathBuf> {
        self.manifest(id)?.preview?.path?.parent().map(Path::to_path_buf)
    }

    /// The preview size to show: runtime request, else the manifest's.
    pub fn preview_size(&self, id: &str) -> Option<(f64, f64)> {
        if let Some(s) = self.preview_size.read().unwrap().get(id) {
            return Some(*s);
        }
        self.manifest(id).and_then(|m| m.preview).map(|p| (p.width, p.height))
    }

    pub fn set_preview_size(&self, id: &str, w: f64, h: f64) -> (f64, f64) {
        let s = (w.clamp(PREVIEW_MIN.0, PREVIEW_MAX.0).round(), h.clamp(PREVIEW_MIN.1, PREVIEW_MAX.1).round());
        self.preview_size.write().unwrap().insert(id.to_string(), s);
        s
    }

    /// All apps in dock order (user order first, then new apps by name).
    pub fn views(&self, settings: &Settings) -> Vec<AppView> {
        let status = self.status.read().unwrap().clone();
        let mut views: Vec<AppView> = self
            .entries
            .read()
            .unwrap()
            .iter()
            .map(|e| {
                let hidden = settings.hidden.contains(&e.id);
                match &e.manifest {
                    Ok(m) => AppView {
                        id: e.id.clone(),
                        file: e.file.display().to_string(),
                        name: m.name.clone(),
                        lucide: match &m.icon {
                            Icon::Lucide(n) => Some(n.clone()),
                            Icon::Image(..) => None,
                        },
                        image: match &m.icon {
                            Icon::Image(d, _) => Some(d.clone()),
                            Icon::Lucide(_) => None,
                        },
                        image_size: match &m.icon {
                            Icon::Image(_, size) => *size,
                            Icon::Lucide(_) => ICON_SIZE_DEFAULT,
                        },
                        color: m.color.clone(),
                        mode: Some(m.mode),
                        has_preview: m.preview.is_some(),
                        preview_url: m.preview.as_ref().and_then(|p| p.url.clone()),
                        preview_file: m.preview.as_ref().and_then(|p| p.path.as_ref()).and_then(|f| {
                            let port = self.api_port.load(std::sync::atomic::Ordering::SeqCst);
                            let name = f.file_name()?.to_string_lossy().to_string();
                            (port != 0).then(|| format!("http://127.0.0.1:{port}/apps/{}/preview/{}", e.id, encode_segment(&name)))
                        }),
                        can_launch: m.launch.is_some(),
                        trackable: m.health.is_some() || m.launch.is_some() || m.preview.as_ref().is_some_and(|p| p.url.is_some()),
                        status: status.get(&e.id).copied().unwrap_or(Status::Unknown),
                        error: None,
                        warnings: e.warnings.clone(),
                        hidden,
                    },
                    Err(err) => AppView {
                        id: e.id.clone(),
                        file: e.file.display().to_string(),
                        name: e.id.clone(),
                        lucide: Some("circle-alert".into()),
                        image: None,
                        image_size: ICON_SIZE_DEFAULT,
                        color: "mist".into(),
                        mode: None,
                        has_preview: false,
                        preview_url: None,
                        preview_file: None,
                        can_launch: false,
                        trackable: false,
                        status: Status::Unknown,
                        error: Some(err.clone()),
                        warnings: Vec::new(),
                        hidden,
                    },
                }
            })
            .collect();
        let rank = |id: &str| settings.order.iter().position(|o| o == id).unwrap_or(usize::MAX);
        // User order first; new apps by name; broken manifests last.
        views.sort_by(|a, b| {
            rank(&a.id)
                .cmp(&rank(&b.id))
                .then_with(|| a.error.is_some().cmp(&b.error.is_some()))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        views
    }

    /// Apps that appear as dock tiles: valid and not hidden.
    pub fn dock_count(&self, settings: &Settings) -> usize {
        self.views(settings).iter().filter(|v| v.error.is_none() && !v.hidden).count()
    }
}

/// Percent-encodes one URL path segment.
fn encode_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Watches the apps folder and reloads on any change, debounced.
pub fn watch(reg: Arc<Registry>) {
    use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::Storage::FileSystem::{
        FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME,
        FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    std::thread::Builder::new()
        .name("holster-watch".into())
        .spawn(move || loop {
            let _ = std::fs::create_dir_all(&reg.dir);
            let wide: Vec<u16> = reg.dir.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
            let h = unsafe {
                FindFirstChangeNotificationW(
                    wide.as_ptr(),
                    0,
                    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE,
                )
            };
            if h == INVALID_HANDLE_VALUE || h.is_null() {
                // Folder unavailable: fall back to polling.
                std::thread::sleep(Duration::from_secs(2));
                reg.reload();
                continue;
            }
            loop {
                // Wake periodically too, in case the folder itself was replaced.
                let r = unsafe { WaitForSingleObject(h, 5000) };
                if r == WAIT_OBJECT_0 {
                    // Writers often touch a file several times in a row.
                    std::thread::sleep(Duration::from_millis(120));
                    if unsafe { FindNextChangeNotification(h) } == 0 {
                        break;
                    }
                    reg.reload();
                } else if !reg.dir.is_dir() {
                    break;
                } else {
                    reg.reload();
                }
            }
            unsafe { windows_sys::Win32::Storage::FileSystem::FindCloseChangeNotification(h) };
        })
        .expect("manifest watcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r##"{
      "schema": 1, "id": "anotes", "name": "Anotes",
      "icon": { "lucide": "notebook-pen" }, "color": "#B8A9F0",
      "health": "http://127.0.0.1:47321/holster/health",
      "preview": { "url": "http://127.0.0.1:47321/holster/preview", "width": 320, "height": 260 },
      "open": { "mode": "panel", "url": "http://127.0.0.1:47321/holster/open" },
      "launch": { "exe": "C:\\Windows\\notepad.exe", "args": [] }
    }"##;

    #[test]
    fn parses_the_spec_example() {
        let (m, w) = parse(GOOD, "anotes");
        let m = m.unwrap();
        assert_eq!(m.mode, OpenMode::Panel);
        assert_eq!(m.preview.unwrap().width, 320.0);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn icon_size_is_a_percentage() {
        assert_eq!(icon_size(None).unwrap(), 80.0);
        assert_eq!(icon_size(Some(&serde_json::json!("65%"))).unwrap(), 65.0);
        assert_eq!(icon_size(Some(&serde_json::json!(90))).unwrap(), 90.0);
        assert!(icon_size(Some(&serde_json::json!("40px"))).is_err());
        assert!(icon_size(Some(&serde_json::json!("150%"))).is_err());
    }

    #[test]
    fn previews_need_a_page() {
        let none = GOOD.replace(r#""url": "http://127.0.0.1:47321/holster/preview", "#, "");
        assert!(parse(&none, "anotes").0.unwrap_err().contains("preview needs"));
        let missing = GOOD.replace(
            r#""url": "http://127.0.0.1:47321/holster/preview""#,
            r#""path": "C:\\nope\\preview.html""#,
        );
        let (m, w) = parse(&missing, "anotes");
        assert!(m.unwrap().preview.is_none());
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn rejects_non_loopback_and_mismatched_ids() {
        assert!(parse(GOOD, "other").0.unwrap_err().contains("file name"));
        let bad = GOOD.replace("http://127.0.0.1:47321/holster/open", "http://example.com/open");
        assert!(parse(&bad, "anotes").0.unwrap_err().contains("open.url"));
        let bad = GOOD.replace("notebook-pen", "not-an-icon");
        assert!(parse(&bad, "anotes").0.is_err());
    }

    #[test]
    fn loopback_rules() {
        assert!(loopback_url("http://127.0.0.1:5000/x"));
        assert!(loopback_url("http://127.0.0.1/x"));
        assert!(!loopback_url("http://127.0.0.1.evil.com/x"));
        assert!(!loopback_url("http://127.0.0.1:99999/x"));
        assert!(!loopback_url("https://127.0.0.1:5000/x"));
        assert!(!loopback_url("http://localhost:5000/x"));
    }

    #[test]
    fn clamps_preview_size_with_a_warning() {
        let big = GOOD.replace("\"width\": 320", "\"width\": 5000");
        let (m, w) = parse(&big, "anotes");
        assert_eq!(m.unwrap().preview.unwrap().width, PREVIEW_MAX.0);
        assert_eq!(w.len(), 1);
    }
}
