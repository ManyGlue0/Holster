//! hello-holster: the smallest Holster app, and the dock's test fixture.
//!
//! - Serves `/holster/health` and `/holster/preview` (a live clock) on loopback.
//! - Writes its manifest to `%APPDATA%\Holster\apps\hello-holster.json` on start.
//! - Opens in `window` mode: a small native window. Press Space in it to send
//!   Holster an activity signal (the tab glows mint).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SPACE;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID: &str = "hello-holster";
const PREFERRED_PORT: u16 = 47390;
const PREVIEW: &str = include_str!("preview.html");
const CLASS: &str = "HelloHolsterWindow";

static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);
/// When the last ping was sent (ms since epoch) and whether Holster answered.
static PINGED_AT: AtomicU64 = AtomicU64::new(0);
static PING_OK: AtomicIsize = AtomicIsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn holster_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Holster"))
}

// ---- HTTP (loopback only) -------------------------------------------------------------

fn respond(mut s: TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn serve(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 4096];
            let mut n = 0;
            while n < buf.len() {
                match (&stream).read(&mut buf[n..]) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => n += k,
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&buf[..n]);
            let mut parts = req.lines().next().unwrap_or("").split_whitespace();
            let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let path = target.split('?').next().unwrap_or("");
            match (method, path) {
                ("GET", "/holster/health") => respond(stream, "200 OK", "text/plain", b"ok"),
                ("GET", "/holster/preview") => respond(stream, "200 OK", "text/html; charset=utf-8", PREVIEW.as_bytes()),
                _ => respond(stream, "404 Not Found", "text/plain", b"Not found"),
            }
        });
    }
}

fn start_server() -> u16 {
    // Loopback only: Holster routes must never be reachable from outside.
    let listener = TcpListener::bind(("127.0.0.1", PREFERRED_PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .expect("loopback listener");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || serve(listener));
    port
}

// ---- manifest ---------------------------------------------------------------------------

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Registers with Holster, if Holster is installed. Written on every start
/// because the port (and the exe path) can change.
fn write_manifest(port: u16) {
    let Some(dir) = holster_dir().filter(|d| d.is_dir()) else { return };
    let apps = dir.join("apps");
    let _ = std::fs::create_dir_all(&apps);
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let base = format!("http://127.0.0.1:{port}/holster");
    let manifest = format!(
        r##"{{
  "schema": 1,
  "id": "{ID}",
  "name": "Hello Holster",
  "icon": {{ "lucide": "clock" }},
  "color": "#7AD6AF",
  "health": "{base}/health",
  "preview": {{ "url": "{base}/preview", "width": 260, "height": 132 }},
  "open": {{ "mode": "window" }},
  "launch": {{ "exe": {exe}, "args": [] }}
}}
"##,
        exe = json_str(&exe)
    );
    // Write to a temp file, then rename: Holster never reads half a manifest.
    let file = apps.join(format!("{ID}.json"));
    let tmp = apps.join(format!(".{ID}.json.tmp"));
    if std::fs::read_to_string(&file).ok().as_deref() != Some(manifest.as_str())
        && std::fs::write(&tmp, &manifest).is_ok()
    {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// POSTs an activity signal to Holster (discovered via holster.json).
fn ping_holster() -> bool {
    let Some(text) = holster_dir().and_then(|d| std::fs::read_to_string(d.join("holster.json")).ok()) else { return false };
    let port = text
        .split("\"port\"")
        .nth(1)
        .and_then(|rest| rest.trim_start_matches([' ', ':']).split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|p| p.parse::<u16>().ok());
    let Some(port) = port else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)) else {
        return false;
    };
    let body = r#"{"kind":"activity"}"#;
    let req = format!(
        "POST /apps/{ID}/signal HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut resp = [0u8; 64];
    let n = s.read(&mut resp).unwrap_or(0);
    String::from_utf8_lossy(&resp[..n]).starts_with("HTTP/1.1 200")
}

// ---- window -------------------------------------------------------------------------------

fn dark_mode() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value: u32 = 1;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut value as *mut _ as *mut _, &mut size)
    } == 0;
    ok && value == 0
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

unsafe fn font(px: i32, weight: i32, face: &str) -> HFONT {
    let face = wide(face);
    CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr())
}

unsafe fn text(dc: HDC, s: &str, rect: &mut RECT, flags: u32) {
    let w: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(dc, w.as_ptr(), w.len() as i32, rect, flags);
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    // Double-buffered so the once-a-second redraw never flickers.
    let mem = CreateCompatibleDC(dc);
    let bmp = CreateCompatibleBitmap(dc, w, h);
    let old_bmp = SelectObject(mem, bmp as _);

    let dark = dark_mode();
    let (bg, fg, fg2, fg3, accent) = if dark {
        (rgb(28, 28, 33), rgb(242, 242, 245), rgb(180, 180, 190), rgb(133, 133, 143), rgb(122, 214, 175))
    } else {
        (rgb(249, 249, 251), rgb(26, 26, 31), rgb(85, 85, 95), rgb(139, 139, 149), rgb(70, 176, 134))
    };
    let brush = CreateSolidBrush(bg);
    FillRect(mem, &rc, brush);
    DeleteObject(brush as _);
    SetBkMode(mem, TRANSPARENT as i32);

    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let px = |v: f32| (v * s).round() as i32;
    let pad = px(28.0);

    let now = chrono_local();
    let small = font(px(13.0), 600, "Segoe UI Variable Text");
    let big = font(px(56.0), 600, "Segoe UI Variable Display");
    let body = font(px(14.0), 400, "Segoe UI Variable Text");

    let mut r = RECT { left: pad, top: px(24.0), right: w - pad, bottom: h };
    SelectObject(mem, small as _);
    SetTextColor(mem, fg3);
    text(mem, "HELLO HOLSTER", &mut r, DT_LEFT | DT_TOP | DT_SINGLELINE);

    // A small dot in the app's color, like the preview's live dot.
    let dot = CreateSolidBrush(accent);
    let old = SelectObject(mem, dot as _);
    let pen = SelectObject(mem, GetStockObject(NULL_PEN));
    let (dx, dy) = (w - pad - px(8.0), px(29.0));
    Ellipse(mem, dx, dy, dx + px(8.0), dy + px(8.0));
    SelectObject(mem, pen);
    SelectObject(mem, old);
    DeleteObject(dot as _);

    let mut r = RECT { left: pad, top: px(46.0), right: w - pad, bottom: h };
    SelectObject(mem, big as _);
    SetTextColor(mem, fg);
    text(mem, &format!("{:02}:{:02}:{:02}", now.0, now.1, now.2), &mut r, DT_LEFT | DT_TOP | DT_SINGLELINE);

    let mut r = RECT { left: pad, top: px(124.0), right: w - pad, bottom: h };
    SelectObject(mem, body as _);
    SetTextColor(mem, fg2);
    text(mem, "Opened from Holster. This is the window open mode.", &mut r, DT_LEFT | DT_TOP | DT_WORDBREAK);

    let since = now_ms().saturating_sub(PINGED_AT.load(Ordering::SeqCst));
    let hint = if since < 2500 {
        if PING_OK.load(Ordering::SeqCst) == 1 { "Pinged Holster: watch the tab glow." } else { "Holster isn't running." }
    } else {
        "Press Space to send Holster an activity signal."
    };
    let mut r = RECT { left: pad, top: h - px(46.0), right: w - pad, bottom: h };
    SetTextColor(mem, if since < 2500 { accent } else { fg3 });
    text(mem, hint, &mut r, DT_LEFT | DT_TOP | DT_SINGLELINE);

    BitBlt(dc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    DeleteObject(bmp as _);
    DeleteDC(mem);
    for f in [small, big, body] {
        DeleteObject(f as _);
    }
    EndPaint(hwnd, &ps);
}

/// Local wall-clock time (h, m, s) without pulling in a date crate.
fn chrono_local() -> (u32, u32, u32) {
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    (st.wHour as u32, st.wMinute as u32, st.wSecond as u32)
}

fn apply_title_bar(hwnd: HWND) {
    let dark: i32 = dark_mode() as i32;
    unsafe {
        // DWMWA_USE_IMMERSIVE_DARK_MODE
        DwmSetWindowAttribute(hwnd, 20, &dark as *const _ as *const _, 4);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_TIMER => {
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_KEYDOWN if wparam as u16 == VK_SPACE => {
            std::thread::spawn(|| {
                let ok = ping_holster();
                PING_OK.store(ok as isize, Ordering::SeqCst);
                PINGED_AT.store(now_ms(), Ordering::SeqCst);
                let h = HWND_MAIN.load(Ordering::SeqCst) as HWND;
                InvalidateRect(h, std::ptr::null(), 0);
            });
            0
        }
        WM_SETTINGCHANGE => {
            apply_title_bar(hwnd);
            InvalidateRect(hwnd, std::ptr::null(), 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DPICHANGED => {
            let r = &*(lparam as *const RECT);
            SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Single instance: a second launch brings the first window forward.
        let mutex = wide("Local\\HelloHolster");
        CreateMutexW(std::ptr::null(), 1, mutex.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class = wide(CLASS);
            let existing = FindWindowW(class.as_ptr(), std::ptr::null());
            if !existing.is_null() {
                ShowWindow(existing, SW_RESTORE);
                SetForegroundWindow(existing);
            }
            return;
        }
    }

    let port = start_server();
    write_manifest(port);

    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hIcon: LoadIconW(std::ptr::null_mut(), IDI_APPLICATION),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let title = wide("Hello Holster");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            300,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        HWND_MAIN.store(hwnd as isize, Ordering::SeqCst);
        // Size for the window's DPI, centered on its monitor.
        let s = GetDpiForWindow(hwnd) as f32 / 96.0;
        let (ww, wh) = ((480.0 * s) as i32, (300.0 * s) as i32);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        SetWindowPos(hwnd, std::ptr::null_mut(), (sw - ww) / 2, (sh - wh) / 2, ww, wh, SWP_NOZORDER);
        apply_title_bar(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, 1, 1000, None);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
