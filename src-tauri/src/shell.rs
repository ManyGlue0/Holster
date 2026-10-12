//! The window shell. Holster draws with three frameless, always-on-top
//! windows whose *bounds* are the visible shapes:
//!
//! - `main`: the tab on the screen edge; open, the app card (the dock); and
//!   for Settings, the full-height panel.
//! - `gear`: the small settings card detached below the dock.
//! - `preview`: the hovered app's preview card, to the left of the dock.
//! - `notify-0..2`: notification cards, popping out next to the tab (see
//!   `notify.rs`, which decides what they show and where).
//!
//! Every shape change animates the native window rect with springs, so the
//! real acrylic, DWM shadow and rounded corners follow every frame and there
//! are never transparent areas that could swallow clicks.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmFlush, DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::apps::{Registry, Status, FALLBACK_SIZE as ERROR_SIZE};
use crate::health::Health;
use crate::settings::Prefs;

// ---- layout (logical px) ---------------------------------------------------------
//
// The UI draws with the same numbers (src/lib/layout.ts); the window is sized
// from them so the visible shape is exactly the card.

pub const TAB_W: f64 = 14.0;
pub const TAB_H: f64 = 88.0;
const TAB_HOVER_OUT: f64 = 6.0;
const MARGIN: f64 = 8.0;

pub const TILE: f64 = 46.0;
pub const TILE_GAP: f64 = 8.0;
pub const DOCK_PAD_Y: f64 = 10.0;
pub const DOCK_PAD_L: f64 = 16.0;
pub const DOCK_PAD_R: f64 = 10.0;
pub const DOCK_W: f64 = DOCK_PAD_L + TILE + DOCK_PAD_R;
pub const VISIBLE_TILES: usize = 5;
pub const GEAR_H: f64 = 50.0;
const CARD_GAP: f64 = 8.0;
const PREVIEW_GAP: f64 = 8.0;

pub const PANEL_W: f64 = 420.0;
const PANEL_GAP: f64 = 10.0;
const PANEL_VM: f64 = 12.0;

pub fn dock_height(rows: usize) -> f64 {
    let n = rows.clamp(1, VISIBLE_TILES) as f64;
    DOCK_PAD_Y * 2.0 + n * TILE + (n - 1.0) * TILE_GAP
}

/// The screen edge Holster lives on. Only the right edge is implemented; all
/// geometry goes through here so another edge stays a local change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Right,
}

impl Edge {
    pub fn from_setting(_s: &str) -> Self {
        Edge::Right
    }
    pub fn name(self) -> &'static str {
        match self {
            Edge::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

impl Rect {
    fn arr(self) -> [f64; 4] {
        [self.l, self.t, self.r, self.b]
    }
    fn from_arr(a: [f64; 4]) -> Self {
        Self { l: a[0], t: a[1], r: a[2], b: a[3] }
    }
    pub fn h(&self) -> f64 {
        self.b - self.t
    }
    pub fn w(&self) -> f64 {
        self.r - self.l
    }
}

// ---- monitors -------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub name: String,
    pub primary: bool,
    pub bounds: Rect,
    pub work: Rect,
    pub scale: f64,
}

pub fn monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        let out = &mut *(data as *mut Vec<MonitorInfo>);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut _) != 0 {
            let (mut dx, mut dy) = (96u32, 96u32);
            GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let r = |r: RECT| Rect { l: r.left as f64, t: r.top as f64, r: r.right as f64, b: r.bottom as f64 };
            let name_len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            let m = info.monitorInfo;
            out.push(MonitorInfo {
                name: String::from_utf16_lossy(&info.szDevice[..name_len]),
                primary: m.dwFlags & 1 != 0,
                bounds: r(m.rcMonitor),
                work: r(m.rcWork),
                scale: dx as f64 / 96.0,
            });
        }
        1
    }
    let mut out: Vec<MonitorInfo> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(cb), &mut out as *mut _ as LPARAM);
    }
    out
}

// ---- spring animator --------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Spring {
    /// Angular frequency (rad/s) and damping ratio, per edge [l, t, r, b].
    pub omega: [f64; 4],
    pub zeta: [f64; 4],
    /// Per-edge start delay in seconds, for staggered morphs.
    pub delay: [f64; 4],
}

impl Spring {
    pub const fn uniform(omega: f64, zeta: f64) -> Self {
        Self { omega: [omega; 4], zeta: [zeta; 4], delay: [0.0; 4] }
    }
}

pub type Callback = Box<dyn FnOnce() + Send>;

struct AnimState {
    cur: [f64; 4],
    vel: [f64; 4],
    target: [f64; 4],
    spring: Spring,
    elapsed: f64,
    running: bool,
    near: Option<Callback>,
    done: Option<Callback>,
}

pub struct Animator {
    hwnd: isize,
    st: Mutex<AnimState>,
    wake: Condvar,
    applied: Mutex<[i32; 4]>,
}

impl Animator {
    fn new(hwnd: isize, initial: Rect, name: &str) -> Arc<Self> {
        let me = Arc::new(Self {
            hwnd,
            st: Mutex::new(AnimState {
                cur: initial.arr(),
                vel: [0.0; 4],
                target: initial.arr(),
                spring: Spring::uniform(20.0, 1.0),
                elapsed: 0.0,
                running: false,
                near: None,
                done: None,
            }),
            wake: Condvar::new(),
            applied: Mutex::new([i32::MIN; 4]),
        });
        let worker = me.clone();
        std::thread::Builder::new().name(format!("holster-anim-{name}")).spawn(move || worker.run()).unwrap();
        me
    }

    /// Animates towards `target`, keeping current velocity (smooth retargeting).
    pub fn animate(&self, target: Rect, spring: Spring, near: Option<Callback>, done: Option<Callback>) {
        let mut st = self.st.lock().unwrap();
        st.target = target.arr();
        st.spring = spring;
        st.elapsed = 0.0;
        st.near = near;
        st.done = done;
        st.running = true;
        self.wake.notify_one();
    }

    /// Jumps immediately (drags, display changes, placing before a reveal).
    pub fn jump(&self, target: Rect) {
        {
            let mut st = self.st.lock().unwrap();
            st.cur = target.arr();
            st.target = target.arr();
            st.vel = [0.0; 4];
            st.running = false;
            st.near = None;
            st.done = None;
        }
        self.apply(target);
    }

    pub fn target(&self) -> Rect {
        Rect::from_arr(self.st.lock().unwrap().target)
    }

    fn apply(&self, r: Rect) {
        let px = [r.l.round() as i32, r.t.round() as i32, r.r.round() as i32, r.b.round() as i32];
        {
            let mut last = self.applied.lock().unwrap();
            if *last == px {
                return;
            }
            *last = px;
        }
        unsafe {
            SetWindowPos(
                self.hwnd as HWND,
                std::ptr::null_mut(),
                px[0],
                px[1],
                (px[2] - px[0]).max(1),
                (px[3] - px[1]).max(1),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    fn run(&self) {
        let mut last = Instant::now();
        loop {
            {
                let mut st = self.st.lock().unwrap();
                while !st.running {
                    st = self.wake.wait(st).unwrap();
                    last = Instant::now();
                }
            }
            // Pace on the compositor's frame clock.
            if unsafe { DwmFlush() } != 0 {
                std::thread::sleep(Duration::from_millis(8));
            }
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f64().min(1.0 / 30.0);
            last = now;

            let (rect, near, done) = {
                let mut st = self.st.lock().unwrap();
                if !st.running {
                    continue;
                }
                st.elapsed += dt;
                let steps = (dt / 0.002).ceil().max(1.0) as usize;
                let h = dt / steps as f64;
                let mut settled = true;
                let mut max_dist: f64 = 0.0;
                for i in 0..4 {
                    if st.elapsed < st.spring.delay[i] {
                        settled &= (st.cur[i] - st.target[i]).abs() < 0.25;
                        max_dist = max_dist.max((st.cur[i] - st.target[i]).abs());
                        continue;
                    }
                    let w = st.spring.omega[i];
                    let k = w * w;
                    let c = 2.0 * st.spring.zeta[i] * w;
                    for _ in 0..steps {
                        let a = -k * (st.cur[i] - st.target[i]) - c * st.vel[i];
                        st.vel[i] += a * h;
                        st.cur[i] += st.vel[i] * h;
                    }
                    let d = (st.cur[i] - st.target[i]).abs();
                    max_dist = max_dist.max(d);
                    settled &= d < 0.25 && st.vel[i].abs() < 4.0;
                }
                let near = if max_dist < 6.0 { st.near.take() } else { None };
                let mut done = None;
                if settled {
                    st.cur = st.target;
                    st.vel = [0.0; 4];
                    st.running = false;
                    done = st.done.take();
                }
                (Rect::from_arr(st.cur), near, done)
            };
            self.apply(rect);
            if let Some(f) = near {
                f();
            }
            if let Some(f) = done {
                f();
            }
        }
    }
}

// Out from the edge first (slight overshoot), then height.
const DOCK_SPRING: Spring = Spring {
    omega: [22.0, 19.0, 22.0, 19.0],
    zeta: [0.8, 0.86, 0.9, 0.86],
    delay: [0.0, 0.025, 0.0, 0.025],
};
const UNDOCK_SPRING: Spring = Spring {
    omega: [25.0, 27.0, 25.0, 27.0],
    zeta: [1.0, 1.0, 1.0, 1.0],
    delay: [0.025, 0.0, 0.025, 0.0],
};
/// The settings card follows the app card out, a beat later.
const GEAR_SPRING: Spring = Spring {
    omega: [22.0, 22.0, 22.0, 22.0],
    zeta: [0.8, 0.9, 0.9, 0.9],
    delay: [0.07, 0.07, 0.07, 0.07],
};
const GEAR_HIDE_SPRING: Spring = Spring::uniform(28.0, 1.0);
const PANEL_SPRING: Spring = Spring {
    omega: [21.0, 17.0, 21.0, 17.0],
    zeta: [0.78, 0.9, 0.9, 0.9],
    delay: [0.0, 0.035, 0.0, 0.035],
};
const CLOSE_SPRING: Spring = Spring {
    omega: [24.0, 26.0, 24.0, 26.0],
    zeta: [1.0, 1.0, 1.0, 1.0],
    delay: [0.03, 0.0, 0.03, 0.0],
};
const HOVER_SPRING: Spring = Spring::uniform(26.0, 0.72);
/// The preview slides out from behind the dock: leftwards first, then height.
const PREVIEW_SPRING: Spring = Spring {
    omega: [23.0, 20.0, 23.0, 20.0],
    zeta: [0.82, 0.88, 0.9, 0.88],
    delay: [0.0, 0.02, 0.0, 0.02],
};
const PREVIEW_HIDE_SPRING: Spring = Spring::uniform(30.0, 1.0);
/// Moving between tiles, or an app resizing its preview.
const GLIDE_SPRING: Spring = Spring::uniform(24.0, 0.92);

// ---- shell ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Closed,
    /// Growing from the tab into the dock.
    Opening,
    /// The app card and settings card are out.
    Dock,
    /// Unfolding into the settings panel.
    Expanding,
    Panel,
    Closing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellState {
    pub mode: Mode,
    pub scale: f64,
    pub edge: Edge,
    /// Visible tile rows in the dock (1..=5).
    pub rows: usize,
    pub dock_height: f64,
    pub panel_width: f64,
    pub panel_height: f64,
    /// How far the edge shapes extend past the screen edge (CSS px), so
    /// content can keep clear of the hidden strip.
    pub overhang: f64,
    /// The tab is tucked into the edge: a fullscreen app is in front, or an
    /// app's panel is out on Holster's edge.
    pub tucked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewState {
    pub app_id: Option<String>,
    pub width: f64,
    pub height: f64,
    pub fallback: bool,
    /// Why opening the app failed, shown on the error card (`fallback`).
    pub error: Option<String>,
    /// Content visible (false while the card is still unfolding or folding away).
    pub shown: bool,
    /// Card bounds relative to the main window, in CSS px (for the hover corridor).
    pub rel: Option<Rect>,
}

struct Geometry {
    scale: f64,
    work: Rect,
    monitor: Rect,
    overhang: f64,
    rows: usize,
    tab: Rect,
    tab_hover: Rect,
    tab_tucked: Rect,
    dock: Rect,
    gear: Rect,
    gear_hidden: Rect,
    panel: Rect,
}

struct Shown {
    id: String,
    anchor: f64,
    w: f64,
    h: f64,
    fallback: bool,
    error: Option<String>,
}

/// An app panel Holster opened on its edge. The tab stays tucked away while
/// it's out, the way Holster's own settings panel takes the tab's place.
struct Cover {
    /// The app's processes (whoever serves its open URL, plus its launch exe).
    pids: Vec<u32>,
    since: Instant,
    /// The panel has been seen on the edge at least once.
    seen: bool,
    /// Consecutive checks without it, once seen.
    misses: u8,
}

/// How long an app gets to put its panel on the edge before the tab returns.
const COVER_APPEAR: Duration = Duration::from_millis(3000);

struct State {
    mode: Mode,
    hover: bool,
    drag: Option<f64>,
    drag_moved: bool,
    prev_foreground: isize,
    opened_at: Instant,
    blur_suppressed: bool,
    fullscreen: bool,
    covers: Vec<Cover>,
    /// A thread is following `covers`.
    cover_watch: bool,
    rows: usize,
    preview: Option<Shown>,
    preview_shown: bool,
    /// The preview card is on screen (otherwise parked).
    preview_out: bool,
    /// The settings card is on screen (otherwise parked).
    gear_out: bool,
    preview_failed: std::collections::HashSet<String>,
    /// Bumped on every preview change so delayed hides can tell they're stale.
    preview_gen: u64,
    /// The tab has grown into a notification card (the card's window stands
    /// in for it), so the tab itself is hidden.
    note_cover: bool,
}

impl State {
    fn tucked(&self) -> bool {
        self.fullscreen || !self.covers.is_empty()
    }
}

pub(crate) struct Win {
    pub hwnd: isize,
    pub anim: Arc<Animator>,
}

/// Notification card windows.
pub const NOTE_SLOTS: usize = 3;

/// What the notifier needs to place its cards (screen px).
pub(crate) struct NoteFrame {
    pub scale: f64,
    pub work: Rect,
    pub tab: Rect,
    /// The tab is busy or away: dock or settings open, being dragged, tucked
    /// for a fullscreen app or an app panel. Notifications wait.
    pub held: bool,
    /// The dock or settings is open (a new notification pulses its tile).
    pub open: bool,
}

pub struct Shell {
    app: AppHandle,
    prefs: Arc<Prefs>,
    reg: Arc<Registry>,
    health: Arc<Health>,
    main: Win,
    gear: Win,
    preview: Win,
    notes: [Win; NOTE_SLOTS],
    state: Mutex<State>,
    /// Called whenever the shell's state or the tab's position changes.
    on_change: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

static SHELL: OnceLock<Arc<Shell>> = OnceLock::new();
/// Per window (main, gear, preview, notify-0..2): refuse activation on click.
static NO_ACTIVATE: [AtomicBool; 3 + NOTE_SLOTS] = [const { AtomicBool::new(true) }; 3 + NOTE_SLOTS];
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

fn ex_style(hwnd: isize, set: u32, clear: u32) {
    unsafe {
        let cur = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, ((cur | set) & !clear) as isize);
    }
}

fn prepare_window(hwnd: isize, index: usize) {
    unsafe {
        let pref = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );
        SetWindowSubclass(hwnd as HWND, Some(subclass_proc), 1, index);
    }
    // Never take focus by default, never appear in Alt+Tab.
    ex_style(hwnd, WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, WS_EX_APPWINDOW);
}

fn show(hwnd: isize) {
    unsafe {
        if IsWindowVisible(hwnd as HWND) == 0 {
            ShowWindow(hwnd as HWND, SW_SHOWNOACTIVATE);
        }
        SetWindowPos(hwnd as HWND, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

fn hide(hwnd: isize) {
    unsafe { ShowWindow(hwnd as HWND, SW_HIDE) };
}

/// Moves a card window far off screen instead of hiding it. A hidden window
/// shows a hollow frame for its first frames when it's shown again; a parked
/// one stays composed, so it slides in clean.
pub(crate) fn park(win: &Win) {
    let r = win.anim.target();
    let (w, h) = (r.w().max(1.0), r.h().max(1.0));
    win.anim.jump(Rect { l: -32000.0, t: -32000.0, r: -32000.0 + w, b: -32000.0 + h });
}

/// Whether one of `pids` has a panel out on Holster's edge: a visible window
/// (not minimized or cloaked) wider than a tab, its right side at the edge
/// (panels keep a small gap), overlapping the work area. A tab of its own on
/// the edge is too narrow to count.
unsafe fn panel_on_edge(pids: &[u32], g: &Geometry) -> bool {
    struct Search<'a> {
        pids: &'a [u32],
        g: &'a Geometry,
        found: bool,
    }
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> i32 {
        let s = &mut *(data as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if !s.pids.contains(&pid) || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return 1;
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return 1;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return 1;
        }
        let (g, reach) = (s.g, 40.0 * s.g.scale);
        let (l, t, rr, b) = (r.left as f64, r.top as f64, r.right as f64, r.bottom as f64);
        let hit = rr - l >= 160.0 * g.scale
            && rr >= g.work.r - reach
            && rr <= g.monitor.r + reach
            && t < g.work.b
            && b > g.work.t;
        if hit {
            s.found = true;
            return 0;
        }
        1
    }
    let mut s = Search { pids, g, found: false };
    EnumWindows(Some(cb), &mut s as *mut _ as LPARAM);
    s.found
}

fn visible(hwnd: isize) -> bool {
    unsafe { IsWindowVisible(hwnd as HWND) != 0 }
}

impl Shell {
    pub fn install(app: &AppHandle, prefs: Arc<Prefs>, reg: Arc<Registry>, health: Arc<Health>) -> Arc<Shell> {
        let hwnd_of = |label: &str| app.get_webview_window(label).expect(label).hwnd().expect("hwnd").0 as isize;
        let (main, gear, preview) = (hwnd_of("main"), hwnd_of("gear"), hwnd_of("preview"));
        let notes: [isize; NOTE_SLOTS] = std::array::from_fn(|i| hwnd_of(&format!("notify-{i}")));
        MAIN_HWND.store(main, Ordering::SeqCst);
        for (i, h) in [main, gear, preview].into_iter().chain(notes).enumerate() {
            prepare_window(h, i);
        }
        let placeholder = Rect { l: 0.0, t: 0.0, r: 1.0, b: 1.0 };
        let shell = Arc::new(Shell {
            app: app.clone(),
            prefs,
            reg,
            health,
            main: Win { hwnd: main, anim: Animator::new(main, placeholder, "main") },
            gear: Win { hwnd: gear, anim: Animator::new(gear, placeholder, "gear") },
            preview: Win { hwnd: preview, anim: Animator::new(preview, placeholder, "preview") },
            notes: std::array::from_fn(|i| Win { hwnd: notes[i], anim: Animator::new(notes[i], placeholder, &format!("note{i}")) }),
            state: Mutex::new(State {
                mode: Mode::Closed,
                hover: false,
                drag: None,
                drag_moved: false,
                prev_foreground: 0,
                opened_at: Instant::now(),
                blur_suppressed: false,
                fullscreen: false,
                covers: Vec::new(),
                cover_watch: false,
                rows: 1,
                preview: None,
                preview_shown: false,
                preview_out: false,
                gear_out: false,
                preview_failed: Default::default(),
                preview_gen: 0,
                note_cover: false,
            }),
            on_change: OnceLock::new(),
        });
        let _ = SHELL.set(shell.clone());
        let g = shell.geometry();
        shell.state.lock().unwrap().rows = g.rows;
        shell.main.anim.jump(g.tab);
        // The cards live parked off screen: always shown, always composed.
        shell.gear.anim.jump(g.gear_hidden);
        park(&shell.gear);
        park(&shell.preview);
        for n in &shell.notes {
            park(n);
        }
        show(main);
        show(gear);
        show(preview);
        for n in &shell.notes {
            show(n.hwnd);
        }
        shell
    }

    pub fn hwnds(&self) -> [isize; 3 + NOTE_SLOTS] {
        let mut out = [self.main.hwnd, self.gear.hwnd, self.preview.hwnd, 0, 0, 0];
        for (i, n) in self.notes.iter().enumerate() {
            out[3 + i] = n.hwnd;
        }
        out
    }

    // ---- notification cards (placed by notify.rs) --------------------------------------

    pub(crate) fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_change.set(Box::new(f));
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.get() {
            f();
        }
    }

    pub(crate) fn note_win(&self, slot: usize) -> &Win {
        &self.notes[slot]
    }

    pub(crate) fn note_frame(&self) -> NoteFrame {
        let st = self.state.lock().unwrap();
        let (held, open) = (st.mode != Mode::Closed || st.tucked() || st.drag.is_some(), st.mode != Mode::Closed);
        drop(st);
        let g = self.geometry();
        NoteFrame { scale: g.scale, work: g.work, tab: g.tab, held, open }
    }

    /// Puts a card window above the others (and above the tab).
    pub(crate) fn raise(&self, win: &Win) {
        show(win.hwnd);
    }

    /// A notification card takes the tab's place: it starts as the tab and
    /// grows from it, so the tab hides underneath (a frame later, once the card
    /// is drawn there). When the last card has folded back into the tab's
    /// shape, the tab returns under it. Only while the tab is resting: an open
    /// dock or a tucked tab is left alone.
    pub(crate) fn note_cover(self: &Arc<Self>, on: bool) {
        let resting = {
            let mut st = self.state.lock().unwrap();
            if st.note_cover == on {
                return;
            }
            st.note_cover = on;
            let resting = st.mode == Mode::Closed && !st.tucked();
            if on && resting {
                st.hover = false;
            }
            resting
        };
        if !resting {
            return;
        }
        let g = self.geometry();
        if on {
            let me = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                let st = me.state.lock().unwrap();
                if st.note_cover && st.mode == Mode::Closed && !st.tucked() {
                    drop(st);
                    hide(me.main.hwnd);
                    me.main.anim.jump(g.tab);
                }
            });
        } else {
            self.main.anim.jump(g.tab);
            show(self.main.hwnd);
        }
    }

    fn geometry(&self) -> Geometry {
        let prefs = self.prefs.get();
        let _edge = Edge::from_setting(&prefs.edge);
        let mons = monitors();
        let mon = mons.iter().find(|m| m.primary).or(mons.first()).cloned().unwrap_or(MonitorInfo {
            name: String::new(),
            primary: true,
            bounds: Rect { l: 0.0, t: 0.0, r: 1920.0, b: 1080.0 },
            work: Rect { l: 0.0, t: 0.0, r: 1920.0, b: 1040.0 },
            scale: 1.0,
        });
        let s = mon.scale;
        let work = mon.work;
        let edge = work.r;
        // Push the outer side past the screen edge so only the inner corners
        // show, unless another monitor (or a right-side taskbar) is there.
        let neighbour_right = mons.iter().any(|m| {
            m.name != mon.name && m.bounds.l >= mon.bounds.r - 1.0 && m.bounds.t < mon.bounds.b && m.bounds.b > mon.bounds.t
        });
        let overhang = if neighbour_right || work.r < mon.bounds.r { 0.0 } else { (10.0 * s).round() };
        let px = |v: f64| (v * s).round();
        let margin = px(MARGIN);

        let (tw, th) = (px(TAB_W), px(TAB_H));
        let cy = (work.t + prefs.tab.y_frac * work.h()).clamp(work.t + th / 2.0 + margin, work.b - th / 2.0 - margin);
        let top = (cy - th / 2.0).round();
        let tab = Rect { l: edge - tw, t: top, r: edge + overhang, b: top + th };
        let tab_hover = Rect { l: tab.l - px(TAB_HOVER_OUT), ..tab };
        let tab_tucked = Rect { l: tab.r - 1.0, ..tab };

        // The dock: centered on the tab, clamped so both cards stay on screen.
        let rows = self.reg.dock_count(&prefs).clamp(1, VISIBLE_TILES);
        let dh = px(dock_height(rows));
        let stack = dh + px(CARD_GAP) + px(GEAR_H);
        let dt = (cy - dh / 2.0).clamp(work.t + margin, (work.b - margin - stack).max(work.t + margin)).round();
        let dock = Rect { l: edge - px(DOCK_W), t: dt, r: edge + overhang, b: dt + dh };
        let gt = dock.b + px(CARD_GAP);
        let gear = Rect { l: dock.l, t: gt, r: dock.r, b: gt + px(GEAR_H) };
        let gear_hidden = Rect { l: edge - 1.0, ..gear };

        let pw = (px(PANEL_W)).min((work.w() - px(48.0)).max(px(300.0)));
        let panel = Rect { l: edge - px(PANEL_GAP) - pw, t: work.t + px(PANEL_VM), r: edge - px(PANEL_GAP), b: work.b - px(PANEL_VM) };
        Geometry {
            scale: s,
            work,
            monitor: mon.bounds,
            overhang: overhang / s,
            rows,
            tab,
            tab_hover,
            tab_tucked,
            dock,
            gear,
            gear_hidden,
            panel,
        }
    }

    /// The preview card for a tile centered at `anchor` (screen px).
    fn preview_rect(g: &Geometry, anchor: f64, w: f64, h: f64) -> Rect {
        let s = g.scale;
        let (w, h) = ((w * s).round(), (h * s).round());
        let margin = (MARGIN * s).round();
        let r = g.dock.l - (PREVIEW_GAP * s).round();
        let w = w.min(r - g.work.l - margin);
        let h = h.min(g.work.h() - margin * 2.0);
        let t = (anchor - h / 2.0).clamp(g.work.t + margin, g.work.b - margin - h).round();
        Rect { l: r - w, t, r, b: t + h }
    }

    /// Where the preview unfolds from (and folds back to): a smaller card
    /// tucked against the dock, so it reads as a drawer, never a thin pill.
    fn preview_start(g: &Geometry, final_rect: Rect) -> Rect {
        let s = g.scale;
        let cy = (final_rect.t + final_rect.b) / 2.0;
        let w = (final_rect.w() * 0.3).max(56.0 * s).min(final_rect.w());
        let h = (final_rect.h() * 0.72).max(40.0 * s).min(final_rect.h());
        Rect { l: (final_rect.r - w).round(), t: (cy - h / 2.0).round(), r: final_rect.r, b: (cy + h / 2.0).round() }
    }

    fn emit_state(&self) {
        let _ = self.app.emit("shell", self.snapshot());
        self.changed();
    }

    pub fn snapshot(&self) -> ShellState {
        let st = self.state.lock().unwrap();
        let g = self.geometry();
        ShellState {
            mode: st.mode,
            scale: g.scale,
            edge: Edge::Right,
            rows: g.rows,
            dock_height: dock_height(g.rows),
            panel_width: g.panel.w() / g.scale,
            panel_height: g.panel.h() / g.scale,
            overhang: g.overhang,
            tucked: st.tucked(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.state.lock().unwrap().mode
    }

    fn set_noactivate(&self, index: usize, on: bool) {
        NO_ACTIVATE[index].store(on, Ordering::SeqCst);
        let hwnd = self.hwnds()[index];
        if on {
            ex_style(hwnd, WS_EX_NOACTIVATE, 0);
        } else {
            ex_style(hwnd, 0, WS_EX_NOACTIVATE);
        }
    }

    /// Side effects that follow the mode: hooks only while the dock is out,
    /// health polling only while something is open.
    fn sync_mode(&self, mode: Mode) {
        crate::hooks::set_active(matches!(mode, Mode::Opening | Mode::Dock));
        self.health.set_active(matches!(mode, Mode::Opening | Mode::Dock | Mode::Expanding | Mode::Panel));
    }

    // ---- dock ----------------------------------------------------------------------

    /// Grows the tab into the dock. Never takes focus.
    /// Also folds the settings panel back into the dock (its back button).
    pub fn open_dock(self: &Arc<Self>) {
        let (from_panel, prev) = {
            let mut st = self.state.lock().unwrap();
            let from_panel = matches!(st.mode, Mode::Panel | Mode::Expanding);
            if !(from_panel || matches!(st.mode, Mode::Closed | Mode::Closing)) || st.drag.is_some() {
                return;
            }
            st.mode = Mode::Opening;
            st.hover = false;
            st.opened_at = Instant::now();
            (from_panel, st.prev_foreground)
        };
        if from_panel {
            // The dock never holds focus: give it back to where it came from.
            unsafe { SendMessageW(self.main.hwnd as HWND, WM_RESTORE_FOCUS, prev as WPARAM, 0) };
        }
        let g = self.geometry();
        self.state.lock().unwrap().rows = g.rows;
        self.sync_mode(Mode::Opening);
        self.set_noactivate(0, true);
        self.emit_state();
        show(self.main.hwnd);
        let me = self.clone();
        self.main.anim.animate(
            g.dock,
            DOCK_SPRING,
            Some(Box::new(move || {
                let mut st = me.state.lock().unwrap();
                if st.mode == Mode::Opening {
                    st.mode = Mode::Dock;
                    drop(st);
                    me.emit_state();
                }
            })),
            None,
        );
        let was_out = std::mem::replace(&mut self.state.lock().unwrap().gear_out, true);
        if !was_out {
            self.gear.anim.jump(g.gear_hidden);
        }
        self.gear.anim.animate(g.gear, GEAR_SPRING, None, None);
    }

    pub fn close(self: &Arc<Self>) {
        let from = {
            let mut st = self.state.lock().unwrap();
            if matches!(st.mode, Mode::Closed | Mode::Closing) {
                return;
            }
            let from = st.mode;
            st.mode = Mode::Closing;
            st.drag = None;
            from
        };
        self.sync_mode(Mode::Closing);
        self.hide_preview();
        self.emit_state();
        let from_panel = matches!(from, Mode::Panel | Mode::Expanding);
        let me = self.clone();
        // Let the content fade out before the shape folds back into the tab.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(if from_panel { 90 } else { 70 }));
            if me.mode() != Mode::Closing {
                return;
            }
            if from_panel {
                let prev = me.state.lock().unwrap().prev_foreground;
                // Hand focus back to where it came from, on the window thread.
                unsafe { SendMessageW(me.main.hwnd as HWND, WM_RESTORE_FOCUS, prev as WPARAM, 0) };
                me.set_noactivate(0, true);
            }
            let g = me.geometry();
            me.retract_gear(&g);
            let tucked = me.is_tucked();
            let me2 = me.clone();
            me.main.anim.animate(
                if tucked { g.tab_tucked } else { g.tab },
                if from_panel { CLOSE_SPRING } else { UNDOCK_SPRING },
                None,
                Some(Box::new(move || {
                    let mut st = me2.state.lock().unwrap();
                    if st.mode == Mode::Closing {
                        st.mode = Mode::Closed;
                        let hide_it = st.tucked();
                        drop(st);
                        if hide_it {
                            hide(me2.main.hwnd);
                        }
                        me2.emit_state();
                    }
                })),
            );
        });
    }

    fn retract_gear(self: &Arc<Self>, g: &Geometry) {
        if !self.state.lock().unwrap().gear_out {
            return;
        }
        let me = self.clone();
        self.gear.anim.animate(
            g.gear_hidden,
            GEAR_HIDE_SPRING,
            Some(Box::new(move || {
                let mut st = me.state.lock().unwrap();
                if !matches!(st.mode, Mode::Opening | Mode::Dock) {
                    st.gear_out = false;
                    drop(st);
                    park(&me.gear);
                }
            })),
            None,
        );
    }

    /// Opens the settings panel, from the dock or from the tray.
    pub fn open_panel(self: &Arc<Self>) {
        {
            let mut st = self.state.lock().unwrap();
            if matches!(st.mode, Mode::Panel | Mode::Expanding) {
                return;
            }
            st.mode = Mode::Expanding;
            st.hover = false;
            st.drag = None;
            st.opened_at = Instant::now();
            let fg = unsafe { GetForegroundWindow() } as isize;
            if !self.hwnds().contains(&fg) {
                st.prev_foreground = fg;
            }
        }
        self.sync_mode(Mode::Expanding);
        self.hide_preview();
        self.emit_state();
        let g = self.geometry();
        self.retract_gear(&g);
        self.set_noactivate(0, false);
        show(self.main.hwnd);
        // Focus has to be taken on the window's own thread (see WM_TAKE_FOCUS).
        unsafe { PostMessageW(self.main.hwnd as HWND, WM_TAKE_FOCUS, 0, 0) };
        let me = self.clone();
        self.main.anim.animate(
            g.panel,
            PANEL_SPRING,
            Some(Box::new(move || {
                let mut st = me.state.lock().unwrap();
                if st.mode == Mode::Expanding {
                    st.mode = Mode::Panel;
                    drop(st);
                    me.emit_state();
                }
            })),
            None,
        );
    }

    /// The hotkey and the tray icon.
    pub fn toggle(self: &Arc<Self>) {
        match self.mode() {
            Mode::Closed | Mode::Closing => self.open_dock(),
            _ => self.close(),
        }
    }

    pub fn hover(&self, on: bool) {
        let mut st = self.state.lock().unwrap();
        if st.mode != Mode::Closed || st.hover == on || st.drag.is_some() || (on && st.tucked()) {
            return;
        }
        st.hover = on;
        drop(st);
        let g = self.geometry();
        self.main.anim.animate(if on { g.tab_hover } else { g.tab }, HOVER_SPRING, None, None);
    }

    fn cursor() -> (f64, f64) {
        let mut p = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut p) };
        (p.x as f64, p.y as f64)
    }

    /// Vertical drag of the tab, or of the dock by its grip; they share one
    /// position. Returns true on "end" if the gesture moved (not a click).
    pub fn drag(self: &Arc<Self>, phase: &str) -> bool {
        let (_, y) = Self::cursor();
        let mut st = self.state.lock().unwrap();
        match phase {
            "start" => {
                if !matches!(st.mode, Mode::Closed | Mode::Dock) {
                    return false;
                }
                let g = self.geometry();
                st.drag = Some(y - (g.tab.t + g.tab.b) / 2.0);
                st.drag_moved = false;
                false
            }
            "move" => {
                let Some(offset) = st.drag else { return false };
                let first = !st.drag_moved;
                st.drag_moved = true;
                let mode = st.mode;
                drop(st);
                if first && mode == Mode::Dock {
                    self.hide_preview_now();
                }
                let g = self.geometry();
                let frac = ((y - offset - g.work.t) / g.work.h()).clamp(0.0, 1.0);
                self.prefs.update_volatile(|s| s.tab.y_frac = frac);
                let g = self.geometry();
                match mode {
                    Mode::Dock => {
                        self.main.anim.jump(g.dock);
                        self.gear.anim.jump(g.gear);
                    }
                    _ => self.main.anim.jump(g.tab_hover),
                }
                self.changed();
                false
            }
            _ => {
                let moved = st.drag.is_some() && st.drag_moved;
                st.drag = None;
                st.drag_moved = false;
                drop(st);
                if moved {
                    let frac = self.prefs.get().tab.y_frac;
                    self.prefs.update(|s| s.tab.y_frac = frac);
                }
                // Notifications wait for the tab to be let go.
                self.changed();
                moved
            }
        }
    }

    /// Re-applies geometry after display, DPI, work-area or app-count changes.
    pub fn relayout(self: &Arc<Self>) {
        let st = self.state.lock().unwrap();
        let (mode, hover, tucked) = (st.mode, st.hover, st.tucked());
        drop(st);
        let g = self.geometry();
        self.state.lock().unwrap().rows = g.rows;
        match mode {
            Mode::Panel | Mode::Expanding => self.main.anim.jump(g.panel),
            Mode::Dock | Mode::Opening => {
                self.main.anim.jump(g.dock);
                self.gear.anim.jump(g.gear);
            }
            _ if tucked => self.main.anim.jump(g.tab_tucked),
            _ => self.main.anim.jump(if hover { g.tab_hover } else { g.tab }),
        }
        {
            let mut st = self.state.lock().unwrap();
            if st.mode == Mode::Expanding {
                st.mode = Mode::Panel;
            }
            if st.mode == Mode::Opening {
                st.mode = Mode::Dock;
            }
        }
        self.hide_preview_now();
        self.emit_state();
    }

    /// The number of dock tiles changed (apps added, removed, hidden).
    pub fn apps_changed(self: &Arc<Self>) {
        let g = self.geometry();
        let (mode, changed) = {
            let mut st = self.state.lock().unwrap();
            let changed = st.rows != g.rows;
            st.rows = g.rows;
            (st.mode, changed)
        };
        if !changed {
            return;
        }
        if matches!(mode, Mode::Dock) {
            self.main.anim.animate(g.dock, GLIDE_SPRING, None, None);
            self.gear.anim.animate(g.gear, GLIDE_SPRING, None, None);
        }
        self.emit_state();
    }

    // ---- preview -------------------------------------------------------------------

    fn emit_preview(&self) {
        let st = self.state.lock().unwrap();
        let g = self.geometry();
        let payload = match &st.preview {
            Some(p) => {
                let r = self.preview.anim.target();
                let d = self.main.anim.target();
                let s = g.scale;
                PreviewState {
                    app_id: Some(p.id.clone()),
                    width: p.w,
                    height: p.h,
                    fallback: p.fallback,
                    error: p.error.clone(),
                    shown: st.preview_shown,
                    rel: Some(Rect { l: (r.l - d.l) / s, t: (r.t - d.t) / s, r: (r.r - d.l) / s, b: (r.b - d.t) / s }),
                }
            }
            None => PreviewState { app_id: None, width: 0.0, height: 0.0, fallback: false, error: None, shown: false, rel: None },
        };
        drop(st);
        let _ = self.app.emit("preview", payload);
    }

    /// Whether `id` has something to preview right now: a page on disk
    /// always, a live page only while the app may be running.
    fn has_preview_now(&self, id: &str, st: &State) -> bool {
        if st.preview_failed.contains(id) {
            return false;
        }
        let Some(p) = self.reg.manifest(id).and_then(|m| m.preview) else { return false };
        p.path.is_some() || (p.url.is_some() && self.reg.status(id) != Status::Stopped)
    }

    /// The card size for `id` and whether it's the error card; None when
    /// there's nothing to show.
    fn preview_size(&self, id: &str, st: &State) -> Option<(f64, f64, bool)> {
        if st.preview.as_ref().is_some_and(|p| p.id == id && p.error.is_some()) {
            return Some((ERROR_SIZE.0, ERROR_SIZE.1, true));
        }
        if !self.has_preview_now(id, st) {
            return None;
        }
        self.reg.preview_size(id).map(|(w, h)| (w, h, false))
    }

    /// Shows `id`'s preview next to its tile; `anchor_y` is the tile center in
    /// the main window's CSS px. Apps without a preview show nothing.
    pub fn show_preview(self: &Arc<Self>, id: &str, anchor_y: f64) {
        self.show_preview_with(id, anchor_y, None)
    }

    /// Shows a small card saying why opening `id` failed.
    pub fn show_error(self: &Arc<Self>, id: &str, anchor_y: f64, error: String) {
        if self.mode() == Mode::Closed {
            return;
        }
        self.show_preview_with(id, anchor_y, Some(error))
    }

    fn show_preview_with(self: &Arc<Self>, id: &str, anchor_y: f64, error: Option<String>) {
        let g = self.geometry();
        let anchor = self.main.anim.target().t + anchor_y * g.scale;
        let (rect, first) = {
            let mut st = self.state.lock().unwrap();
            if !matches!(st.mode, Mode::Dock | Mode::Opening) || st.drag.is_some() {
                return;
            }
            if st.preview.as_ref().is_some_and(|p| p.id != id || error.is_some()) {
                st.preview.as_mut().unwrap().error = None;
            }
            let size = match &error {
                Some(_) => Some((ERROR_SIZE.0, ERROR_SIZE.1, true)),
                None => self.preview_size(id, &st),
            };
            let Some((w, h, fallback)) = size else {
                drop(st);
                self.hide_preview();
                return;
            };
            let first = st.preview.is_none() || !st.preview_out;
            if !first
                && error.is_none()
                && st.preview.as_ref().is_some_and(|p| {
                    p.id == id && p.w == w && p.h == h && p.fallback == fallback && (p.anchor - anchor).abs() < 1.0
                })
            {
                return;
            }
            st.preview = Some(Shown { id: id.to_string(), anchor, w, h, fallback, error });
            st.preview_gen += 1;
            // Content is revealed while the card unfolds around it.
            st.preview_shown = true;
            st.preview_out = true;
            (Self::preview_rect(&g, anchor, w, h), first)
        };
        if first {
            self.preview.anim.jump(Self::preview_start(&g, rect));
            self.preview.anim.animate(rect, PREVIEW_SPRING, None, None);
        } else {
            self.preview.anim.animate(rect, GLIDE_SPRING, None, None);
        }
        self.emit_preview();
    }

    /// Fades the preview content, then folds the card away.
    pub fn hide_preview(self: &Arc<Self>) {
        let gen = {
            let mut st = self.state.lock().unwrap();
            if st.preview.is_none() {
                return;
            }
            st.preview = None;
            st.preview_shown = false;
            st.preview_gen += 1;
            st.preview_gen
        };
        self.emit_preview();
        // Fold right away: the content rides the leading edge back behind the
        // dock and fades on the way, like a drawer closing.
        let g = self.geometry();
        let cur = self.preview.anim.target();
        let me = self.clone();
        // Park as soon as the fold is nearly done: waiting for the spring's
        // last pixel leaves a small stub of card behind for a few frames.
        self.preview.anim.animate(
            Self::preview_end(&g, cur),
            PREVIEW_HIDE_SPRING,
            Some(Box::new(move || {
                let mut st = me.state.lock().unwrap();
                if st.preview_gen == gen {
                    st.preview_out = false;
                    drop(st);
                    park(&me.preview);
                }
            })),
            None,
        );
    }

    /// Where a closing preview folds to: a thin strip behind the dock, so
    /// whatever is left when it's parked is a sliver, not a card.
    fn preview_end(g: &Geometry, cur: Rect) -> Rect {
        let s = g.scale;
        let cy = (cur.t + cur.b) / 2.0;
        let h = (cur.h() * 0.6).max(32.0 * s);
        Rect { l: cur.r - (10.0 * s).round(), t: (cy - h / 2.0).round(), r: cur.r, b: (cy + h / 2.0).round() }
    }

    /// Instant hide (dragging, display changes).
    fn hide_preview_now(&self) {
        {
            let mut st = self.state.lock().unwrap();
            st.preview = None;
            st.preview_shown = false;
            st.preview_out = false;
            st.preview_gen += 1;
        }
        park(&self.preview);
        self.emit_preview();
    }

    /// The preview page asked for a new size (postMessage resize).
    pub fn resize_preview(self: &Arc<Self>, id: &str, w: f64, h: f64) {
        self.reg.set_preview_size(id, w, h);
        self.refit_preview(id);
    }

    /// The preview page didn't load: show nothing for it.
    pub fn preview_failed(self: &Arc<Self>, id: &str, failed: bool) {
        {
            let mut st = self.state.lock().unwrap();
            let changed = if failed { st.preview_failed.insert(id.to_string()) } else { st.preview_failed.remove(id) };
            if !changed {
                return;
            }
        }
        self.refit_preview(id);
    }

    /// Re-sizes the visible preview if it belongs to `id`, or folds it away
    /// when there's nothing left to show.
    fn refit_preview(self: &Arc<Self>, id: &str) {
        let g = self.geometry();
        let rect = {
            let mut st = self.state.lock().unwrap();
            if st.preview.as_ref().is_none_or(|p| p.id != id) {
                return;
            }
            let Some((w, h, fallback)) = self.preview_size(id, &st) else {
                drop(st);
                self.hide_preview();
                return;
            };
            let p = st.preview.as_mut().unwrap();
            if p.w == w && p.h == h && p.fallback == fallback {
                return;
            }
            p.w = w;
            p.h = h;
            p.fallback = fallback;
            Self::preview_rect(&g, p.anchor, w, h)
        };
        self.preview.anim.animate(rect, GLIDE_SPRING, None, None);
        self.emit_preview();
    }

    /// Statuses changed: a live preview appears or goes away with its app.
    pub fn statuses_changed(self: &Arc<Self>) {
        let id = self.state.lock().unwrap().preview.as_ref().map(|p| p.id.clone());
        if let Some(id) = id {
            if self.reg.status(&id) == Status::Running {
                self.state.lock().unwrap().preview_failed.remove(&id);
            }
            self.refit_preview(&id);
        }
    }

    pub fn preview_state(&self) -> PreviewState {
        let st = self.state.lock().unwrap();
        match &st.preview {
            Some(p) => PreviewState {
                app_id: Some(p.id.clone()),
                width: p.w,
                height: p.h,
                fallback: p.fallback,
                error: p.error.clone(),
                shown: st.preview_shown,
                rel: None,
            },
            None => PreviewState { app_id: None, width: 0.0, height: 0.0, fallback: false, error: None, shown: false, rel: None },
        }
    }

    /// Screen y (px) of a tile center given in main-window CSS px.
    pub fn screen_y(&self, client_y: f64) -> f64 {
        let g = self.geometry();
        (self.main.anim.target().t + client_y * g.scale).round()
    }

    /// The tab's vertical center in screen px: where it comes back when an
    /// app's panel closes.
    pub fn tab_y(&self) -> f64 {
        let t = self.geometry().tab;
        ((t.t + t.b) / 2.0).round()
    }

    // ---- tucking (game mode, app panels) ---------------------------------------------------

    pub fn monitor_bounds(&self) -> Rect {
        self.geometry().monitor
    }

    pub fn is_tucked(&self) -> bool {
        self.state.lock().unwrap().tucked()
    }

    /// Game mode: a fullscreen app is in front.
    pub fn set_fullscreen(self: &Arc<Self>, on: bool) {
        self.update_tuck(|st| st.fullscreen = on);
    }

    /// The tab steps aside while something else owns the edge: it fades and
    /// retracts into the edge, then hides so it can't catch clicks there. Once
    /// nothing does, it slides back out. An open dock or panel is left alone
    /// and tucks away when it closes.
    fn update_tuck(self: &Arc<Self>, change: impl FnOnce(&mut State)) {
        let (mode, before, after) = {
            let mut st = self.state.lock().unwrap();
            let before = st.tucked();
            change(&mut st);
            let after = st.tucked();
            if after {
                st.hover = false;
            }
            (st.mode, before, after)
        };
        if before == after {
            return;
        }
        self.emit_state();
        if mode != Mode::Closed {
            return;
        }
        let g = self.geometry();
        if after {
            let me = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(140));
                if !me.is_tucked() || me.mode() != Mode::Closed {
                    return;
                }
                let me2 = me.clone();
                me.main.anim.animate(
                    g.tab_tucked,
                    UNDOCK_SPRING,
                    None,
                    Some(Box::new(move || {
                        if me2.is_tucked() && me2.mode() == Mode::Closed {
                            hide(me2.main.hwnd);
                        }
                    })),
                );
            });
        } else {
            if !visible(self.main.hwnd) {
                self.main.anim.jump(g.tab_tucked);
            }
            show(self.main.hwnd);
            self.main.anim.animate(g.tab, HOVER_SPRING, None, None);
        }
    }

    /// An app is opening its panel on Holster's edge. Call before `close()`,
    /// so the dock folds straight into the edge instead of back into the tab.
    /// If the panel doesn't show up on the edge within a few seconds (the app
    /// put it somewhere else), the tab comes back.
    pub fn cover_with(self: &Arc<Self>, pids: Vec<u32>) {
        if pids.is_empty() {
            return;
        }
        let mut spawn = false;
        self.update_tuck(|st| {
            st.covers.retain(|c| !c.pids.iter().any(|p| pids.contains(p)));
            st.covers.push(Cover { pids, since: Instant::now(), seen: false, misses: 0 });
            spawn = !std::mem::replace(&mut st.cover_watch, true);
        });
        if spawn {
            let me = self.clone();
            std::thread::Builder::new()
                .name("holster-cover".into())
                .spawn(move || me.watch_covers())
                .expect("cover watcher");
        }
    }

    /// Follows the panels covering the edge ten times a second, only while
    /// there are any. Each check is one EnumWindows pass.
    fn watch_covers(self: Arc<Self>) {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let pids: Vec<Vec<u32>> = self.state.lock().unwrap().covers.iter().map(|c| c.pids.clone()).collect();
            let g = self.geometry();
            let out: Vec<(Vec<u32>, bool)> = pids.into_iter().map(|p| {
                let on = unsafe { panel_on_edge(&p, &g) };
                (p, on)
            }).collect();
            let mut done = false;
            self.update_tuck(|st| {
                for c in st.covers.iter_mut() {
                    let Some((_, on)) = out.iter().find(|(p, _)| *p == c.pids) else { continue };
                    if *on {
                        c.seen = true;
                        c.misses = 0;
                    } else if c.seen {
                        c.misses = c.misses.saturating_add(1);
                    }
                }
                // Two misses in a row: the panel is gone (closing animations
                // already left the edge by then).
                st.covers.retain(|c| if c.seen { c.misses < 2 } else { c.since.elapsed() < COVER_APPEAR });
                if st.covers.is_empty() {
                    st.cover_watch = false;
                    done = true;
                }
            });
            if done {
                return;
            }
        }
    }

    // ---- focus (settings panel) ----------------------------------------------------------

    pub fn suppress_blur(&self, on: bool) {
        self.state.lock().unwrap().blur_suppressed = on;
    }

    fn may_close_on_blur(&self) -> bool {
        let st = self.state.lock().unwrap();
        matches!(st.mode, Mode::Panel | Mode::Expanding)
            && !st.blur_suppressed
            && st.opened_at.elapsed() > Duration::from_millis(350)
    }

    /// Click outside the panel: after a short debounce, ask the UI to close.
    /// The UI has the final say (pin, open menus, IME composition).
    pub fn on_blur(self: &Arc<Self>) {
        if !self.may_close_on_blur() {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            let fg = unsafe { GetForegroundWindow() } as isize;
            if !me.hwnds().contains(&fg) && me.may_close_on_blur() {
                let _ = me.app.emit_to("main", "shell-blur", ());
            }
        });
    }

    /// Click-away from the dock (low-level hook).
    pub fn on_outside_click(self: &Arc<Self>) {
        if matches!(self.mode(), Mode::Dock | Mode::Opening) {
            self.close();
        }
    }
}

/// Brings the window to the foreground and focuses the webview. Called on the
/// window's thread: attaching to the current foreground thread's input lets
/// Windows accept the switch even when another app is in front.
unsafe fn take_focus(hwnd: HWND) {
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    let fg = GetForegroundWindow();
    if fg != hwnd {
        let me = GetCurrentThreadId();
        let other = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
        let attached = other != 0 && other != me && AttachThreadInput(me, other, 1) != 0;
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(me, other, 0);
        }
    }
    if let Some(w) = SHELL.get().and_then(|s| s.app.get_webview_window("main")) {
        let _ = w.set_focus();
    }
}

const WM_RELAYOUT: u32 = WM_APP + 7;
const WM_TAKE_FOCUS: u32 = WM_APP + 8;
const WM_RESTORE_FOCUS: u32 = WM_APP + 9;

/// Gives the foreground back to `prev` if Holster still has it.
unsafe fn restore_focus(hwnd: HWND, prev: HWND) {
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    if prev.is_null() || IsWindow(prev) == 0 || GetForegroundWindow() != hwnd {
        return;
    }
    let me = GetCurrentThreadId();
    let other = GetWindowThreadProcessId(prev, std::ptr::null_mut());
    let attached = other != 0 && other != me && AttachThreadInput(me, other, 1) != 0;
    SetForegroundWindow(prev);
    if attached {
        AttachThreadInput(me, other, 0);
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    index: usize,
) -> LRESULT {
    match msg {
        // Keep the acrylic in its "active" look even when unfocused, so the
        // cards are always frosted rather than a flat fallback color.
        WM_NCACTIVATE => return DefSubclassProc(hwnd, msg, 1, lparam),
        WM_MOUSEACTIVATE if NO_ACTIVATE.get(index).is_some_and(|f| f.load(Ordering::SeqCst)) => {
            return MA_NOACTIVATE as LRESULT
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE if index == 0 => {
            if msg != WM_SETTINGCHANGE || wparam as u32 == SPI_SETWORKAREA {
                PostMessageW(hwnd, WM_RELAYOUT, 0, 0);
            }
        }
        WM_RELAYOUT => {
            if let Some(shell) = SHELL.get() {
                let shell = shell.clone();
                // Monitor info settles a moment after the change notification.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(120));
                    shell.relayout();
                });
            }
            return 0;
        }
        WM_RESTORE_FOCUS => {
            restore_focus(hwnd, wparam as HWND);
            return 0;
        }
        WM_TAKE_FOCUS => {
            if SHELL.get().is_some_and(|s| matches!(s.mode(), Mode::Panel | Mode::Expanding)) {
                take_focus(hwnd);
            }
            return 0;
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Frosted acrylic via the window accent policy. Unlike the DWM system
/// backdrop (which falls back to a flat color whenever the window is not
/// active), this stays frosted while the cards sit unfocused on the edge.
///
/// `card`: the settings and preview cards carry the veil in the material
/// itself (tint = acrylic tint with the CSS veil composited over it). While a
/// card grows, the webview fills the new edge a frame late; with the veil in
/// the material that edge already looks finished, so nothing flickers.
pub fn apply_material(hwnd: isize, dark: bool, card: bool) -> bool {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct WinCompAttrData {
        attrib: u32,
        data: *mut std::ffi::c_void,
        size: usize,
    }
    type SetWca = unsafe extern "system" fn(HWND, *mut WinCompAttrData) -> i32;
    unsafe {
        // Make sure the DWM system backdrop is off so the two don't fight.
        let none: u32 = 1; // DWMSBT_NONE
        DwmSetWindowAttribute(hwnd as HWND, 38, &none as *const _ as *const _, 4);
        let user32: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        let module = GetModuleHandleW(user32.as_ptr());
        let Some(proc) = GetProcAddress(module, c"SetWindowCompositionAttribute".as_ptr() as *const u8) else {
            return false;
        };
        let set: SetWca = std::mem::transmute(proc);
        // Gradient is ABGR: a soft tint that keeps the blur legible.
        let gradient = match (dark, card) {
            (true, false) => 0x5C201C1Cu32,
            (false, false) => 0x66F4F2F2u32,
            // rgba(24,24,28,.52) and rgba(249,249,251,.6) over the tints above.
            (true, true) => 0xB01D1919u32,
            (false, true) => 0xC2F9F7F7u32,
        };
        let mut policy = AccentPolicy { state: 4 /* ACRYLICBLURBEHIND */, flags: 2, gradient, animation: 0 };
        let mut data = WinCompAttrData {
            attrib: 19, // WCA_ACCENT_POLICY
            data: &mut policy as *mut _ as *mut _,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        set(hwnd as HWND, &mut data) != 0
    }
}
