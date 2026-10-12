//! Click-away and Escape for the dock, without stealing focus.
//!
//! The dock opens on hover, so it never takes keyboard focus from whatever
//! the user is typing in. While it is open, low-level mouse and keyboard hooks
//! (installed only for that time) notice a click outside Holster's windows
//! and the Escape key. Escape is swallowed: while the dock is open, it belongs
//! to the dock, exactly as if the dock had focus.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

type Callback = Box<dyn Fn() + Send + Sync>;

struct Ctx {
    thread_id: u32,
    outside_click: Callback,
    escape: Callback,
}

static CTX: OnceLock<Ctx> = OnceLock::new();
const OWN_COUNT: usize = 3 + crate::shell::NOTE_SLOTS;
static OWN: [AtomicIsize; OWN_COUNT] = [const { AtomicIsize::new(0) }; OWN_COUNT];
static MOUSE: AtomicIsize = AtomicIsize::new(0);
static KEYS: AtomicIsize = AtomicIsize::new(0);
static SWALLOW_ESC_UP: AtomicBool = AtomicBool::new(false);
static READY: AtomicU32 = AtomicU32::new(0);

const WM_INSTALL: u32 = WM_APP + 1;
const WM_REMOVE: u32 = WM_APP + 2;

/// Starts the hook thread. `own` are Holster's top-level windows.
pub fn start(own: [isize; OWN_COUNT], outside_click: impl Fn() + Send + Sync + 'static, escape: impl Fn() + Send + Sync + 'static) {
    for (slot, h) in OWN.iter().zip(own) {
        slot.store(h, Ordering::SeqCst);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("holster-hooks".into())
        .spawn(move || unsafe {
            let _ = tx.send(GetCurrentThreadId());
            let mut msg: MSG = std::mem::zeroed();
            // Make sure the thread has a message queue before anyone posts to it.
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            READY.store(1, Ordering::SeqCst);
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                match msg.message {
                    WM_INSTALL => install(),
                    WM_REMOVE => remove(),
                    _ => {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        })
        .expect("hook thread");
    let thread_id = rx.recv().unwrap_or(0);
    while READY.load(Ordering::SeqCst) == 0 {
        std::thread::yield_now();
    }
    let _ = CTX.set(Ctx { thread_id, outside_click: Box::new(outside_click), escape: Box::new(escape) });
}

/// Hooks are live only while the dock is open.
pub fn set_active(on: bool) {
    if let Some(c) = CTX.get() {
        unsafe { PostThreadMessageW(c.thread_id, if on { WM_INSTALL } else { WM_REMOVE }, 0, 0) };
    }
}

unsafe fn install() {
    let module = GetModuleHandleW(std::ptr::null());
    if MOUSE.load(Ordering::SeqCst) == 0 {
        let h = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        MOUSE.store(h as isize, Ordering::SeqCst);
    }
    if KEYS.load(Ordering::SeqCst) == 0 {
        let h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), module, 0);
        KEYS.store(h as isize, Ordering::SeqCst);
    }
}

unsafe fn remove() {
    let m = MOUSE.swap(0, Ordering::SeqCst);
    if m != 0 {
        UnhookWindowsHookEx(m as HHOOK);
    }
    let k = KEYS.swap(0, Ordering::SeqCst);
    if k != 0 {
        UnhookWindowsHookEx(k as HHOOK);
    }
}

fn is_own(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) } as isize;
    OWN.iter().any(|o| o.load(Ordering::SeqCst) == root)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(wparam as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN)
    {
        let info = &*(lparam as *const MSLLHOOKSTRUCT);
        if !is_own(WindowFromPoint(info.pt)) {
            // Never do real work inside a low-level hook.
            std::thread::spawn(|| {
                if let Some(c) = CTX.get() {
                    (c.outside_click)();
                }
            });
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam as *const KBDLLHOOKSTRUCT);
        if info.vkCode == VK_ESCAPE as u32 {
            match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    SWALLOW_ESC_UP.store(true, Ordering::SeqCst);
                    std::thread::spawn(|| {
                        if let Some(c) = CTX.get() {
                            (c.escape)();
                        }
                    });
                    return 1;
                }
                WM_KEYUP | WM_SYSKEYUP if SWALLOW_ESC_UP.swap(false, Ordering::SeqCst) => return 1,
                _ => {}
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}
