//! Game mode: notices when a fullscreen app (exclusive D3D, presentation mode,
//! or a borderless window covering the monitor) is in front on Holster's
//! monitor, and tells the shell to tuck the tab away. (Same logic as Anotes.)
//!
//! A dedicated thread reacts to foreground changes through a WinEvent hook and
//! re-checks once a second, so a window going fullscreen in place (F11, a game
//! switching modes) is caught too. Each check is a handful of cheap Win32 calls.

use std::sync::{Arc, OnceLock};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows_sys::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::settings::Prefs;
use crate::shell::Shell;

struct Ctx {
    shell: Arc<Shell>,
    prefs: Arc<Prefs>,
    thread_id: u32,
}

static CTX: OnceLock<Ctx> = OnceLock::new();
const WM_RECHECK: u32 = WM_APP + 1;

pub fn start(shell: Arc<Shell>, prefs: Arc<Prefs>) {
    std::thread::Builder::new()
        .name("holster-fullscreen".into())
        .spawn(move || unsafe {
            let thread_id = windows_sys::Win32::System::Threading::GetCurrentThreadId();
            let _ = CTX.set(Ctx { shell, prefs, thread_id });
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            SetTimer(std::ptr::null_mut(), 0, 1000, None);
            check();
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_TIMER || msg.message == WM_RECHECK {
                    check();
                }
                DispatchMessageW(&msg);
            }
        })
        .expect("fullscreen watcher");
}

/// Re-evaluates right away (settings changed).
pub fn recheck() {
    if let Some(c) = CTX.get() {
        unsafe { PostThreadMessageW(c.thread_id, WM_RECHECK, 0, 0) };
    }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    check();
}

fn check() {
    let Some(c) = CTX.get() else { return };
    let on = if c.prefs.get().hide_in_fullscreen {
        match unsafe { detect(&c.shell) } {
            Some(v) => v,
            None => return, // our own window is in front: keep the current state
        }
    } else {
        false
    };
    c.shell.set_fullscreen(on);
}

/// `None` when Holster itself is in front (settings opened over a game).
unsafe fn detect(shell: &Shell) -> Option<bool> {
    let fg = GetForegroundWindow();
    if fg.is_null() {
        return Some(false);
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(fg, &mut pid);
    if shell.hwnds().contains(&(fg as isize)) || pid == GetCurrentProcessId() {
        return None;
    }
    if IsWindowVisible(fg) == 0 || IsIconic(fg) != 0 || is_shell_window(fg) {
        return Some(false);
    }

    // Only the monitor Holster lives on matters.
    let hmon = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
    if hmon.is_null() {
        return Some(false);
    }
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(hmon, &mut mi) == 0 {
        return Some(false);
    }
    let m = mi.rcMonitor;
    let tab_mon = shell.monitor_bounds();
    let same_monitor = m.left as f64 == tab_mon.l
        && m.top as f64 == tab_mon.t
        && m.right as f64 == tab_mon.r
        && m.bottom as f64 == tab_mon.b;
    if !same_monitor {
        return Some(false);
    }

    let mut state = 0;
    if SHQueryUserNotificationState(&mut state) >= 0
        && matches!(state, QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_BUSY | QUNS_PRESENTATION_MODE)
    {
        return Some(true);
    }

    // Borderless "windowed fullscreen": a captionless window covering the
    // whole monitor. A normal maximized window (which can also reach the edges
    // when the taskbar auto-hides) keeps its caption and is ignored.
    let style = GetWindowLongW(fg, GWL_STYLE) as u32;
    if IsZoomed(fg) != 0 && style & WS_CAPTION == WS_CAPTION {
        return Some(false);
    }
    let mut r: RECT = std::mem::zeroed();
    if GetWindowRect(fg, &mut r) == 0 {
        return Some(false);
    }
    Some(r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom)
}

/// Desktop, taskbars and other shell surfaces that can be "foreground" and
/// span the monitor without being an app.
unsafe fn is_shell_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    matches!(
        class.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow" | "XamlExplorerHostIslandWindow"
    ) || hwnd == GetShellWindow()
        || hwnd == GetDesktopWindow()
}
