//! Launching apps and bringing their windows forward.

use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn normalize(p: &Path) -> String {
    let s = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    s.to_string_lossy().trim_start_matches(r"\\?\").to_ascii_lowercase()
}

fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// Process ids running `exe`. Only processes whose file name matches are
/// opened, so this stays cheap.
pub fn pids_for(exe: &Path) -> Vec<u32> {
    let want = normalize(exe);
    let Some(file) = exe.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase()) else { return Vec::new() };
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            if String::from_utf16_lossy(&e.szExeFile[..n]).to_ascii_lowercase() == file
                && image_path(e.th32ProcessID).is_some_and(|p| normalize(&p) == want)
            {
                out.push(e.th32ProcessID);
            }
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// The main visible window of any of `pids`: unowned, not a tool window.
pub fn main_window(pids: &[u32]) -> Option<isize> {
    struct Search<'a> {
        pids: &'a [u32],
        found: isize,
    }
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> i32 {
        let s = &mut *(data as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if !s.pids.contains(&pid) || IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
            return 1;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW != 0 && ex & WS_EX_APPWINDOW == 0 {
            return 1;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return 1;
        }
        s.found = hwnd as isize;
        0
    }
    if pids.is_empty() {
        return None;
    }
    let mut s = Search { pids, found: 0 };
    unsafe { EnumWindows(Some(cb), &mut s as *mut _ as LPARAM) };
    (s.found != 0).then_some(s.found)
}

/// The process listening on a local TCP port (IPv4), e.g. the app behind an
/// `open.url`.
pub fn listener_pid(port: u16) -> Option<u32> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER};
    const AF_INET: u32 = 2;
    unsafe {
        let mut size = 0u32;
        GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
        // Rows are six u32s (state, local addr, local port, remote addr,
        // remote port, pid) after a u32 count. Room for a few new listeners.
        let mut buf = vec![0u32; size as usize / 4 + 64];
        size = (buf.len() * 4) as u32;
        if GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0) != 0 {
            return None;
        }
        let n = (buf[0] as usize).min((buf.len() - 1) / 6);
        buf[1..1 + n * 6]
            .chunks_exact(6)
            .find(|row| u16::from_be(row[2] as u16) == port)
            .map(|row| row[5])
    }
}

/// Restores and brings a window to the foreground. Attaching to the current
/// foreground thread's input lets Windows accept the switch.
pub fn focus(hwnd: isize) {
    let hwnd = hwnd as HWND;
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        let me = GetCurrentThreadId();
        let other = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
        let attached = other != 0 && other != me && AttachThreadInput(me, other, 1) != 0;
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(me, other, 0);
        }
    }
}

/// Lets whichever process Holster wakes next take the foreground (a panel
/// app opening its panel). Holster may grant this because the click that
/// triggered it was Holster's input.
pub fn allow_foreground() {
    unsafe { AllowSetForegroundWindow(u32::MAX /* ASFW_ANY */) };
}

pub fn launch(exe: &Path, args: &[String]) -> Result<(), String> {
    if !exe.is_file() {
        return Err(format!("{} wasn't found", exe.display()));
    }
    allow_foreground();
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    {
        use std::os::windows::process::CommandExt;
        // Its own process group, so it outlives Holster and ignores our console.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn().map(|_| ()).map_err(|e| format!("Couldn't start {}: {e}", exe.display()))
}
