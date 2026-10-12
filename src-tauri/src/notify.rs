//! Notifications: small cards an app asks Holster to show. The tab grows into
//! the card, like it grows into the dock: the newest card takes the tab's
//! place on the edge and older ones are pushed away from it. A card folds
//! into the edge when it's dismissed, acted on, or its time is up; the last
//! one folds back into the tab.
//!
//! Holster owns the queue, the timers, the stack and the windows (three card
//! windows, `notify-0..2`; more notifications wait their turn). The UI draws
//! the content (`NoteWindow`). Apps hear back on their manifest's
//! `notify.url`.
//!
//! Notifications wait while the tab is busy or away (dock or settings open, a
//! fullscreen app, an app panel on the edge) and during Do not disturb. A
//! timed one only counts down while it's on screen and not hovered.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::apps::{loopback_url, Registry};
use crate::health::Health;
use crate::settings::Prefs;
use crate::shell::{park, NoteFrame, Rect, Shell, Spring, NOTE_SLOTS};

/// Seconds a timed notification stays when the app doesn't say.
pub const DEFAULT_TIMEOUT: f64 = 6.0;
const MAX_TIMEOUT: f64 = 3600.0;
/// Card width for notifications Holster draws (CSS px).
pub const CARD_W: f64 = 340.0;
const TEXT_H: (f64, f64) = (56.0, 360.0);
const PAGE_MIN: (f64, f64) = (200.0, 48.0);
const PAGE_MAX: (f64, f64) = (480.0, 480.0);
const PAGE_DEFAULT: (f64, f64) = (CARD_W, 120.0);
const MAX_BUTTONS: usize = 2;
/// Between cards, and between a card and the tab (CSS px).
const GAP: f64 = 8.0;
const MARGIN: f64 = 8.0;
/// A card whose content hasn't reported its size by then pops at this height.
const MEASURE_WAIT: Duration = Duration::from_millis(400);
const MEASURE_FALLBACK_H: f64 = 96.0;
/// Leaving a card with less than this left gives it this long again.
const AFTER_HOVER: Duration = Duration::from_millis(1500);
/// A flick farther than this share of the card (or these px) dismisses it.
const FLICK_SHARE: f64 = 0.35;
const FLICK_PX: f64 = 90.0;

/// The tab grows into the card like it grows into the dock: out from the edge
/// first (slight overshoot), then height.
const POP_SPRING: Spring = Spring {
    omega: [22.0, 19.0, 22.0, 19.0],
    zeta: [0.8, 0.86, 0.9, 0.86],
    delay: [0.0, 0.025, 0.0, 0.025],
};
/// The last card shrinks back into the tab like the dock does, once its
/// content has faded.
const INTO_TAB_SPRING: Spring = Spring {
    omega: [25.0, 27.0, 25.0, 27.0],
    zeta: [1.0, 1.0, 1.0, 1.0],
    delay: [0.095, 0.07, 0.095, 0.07],
};
/// The tab under a card that has just folded into it gets this long to draw
/// before the card goes.
const TAB_HANDOVER: Duration = Duration::from_millis(60);
const FOLD_SPRING: Spring = Spring::uniform(30.0, 1.0);
/// The stack rearranging, a card resizing, a flick springing back.
const GLIDE_SPRING: Spring = Spring::uniform(24.0, 0.92);

// ---- the request apps send ----------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Button {
    pub id: String,
    pub label: String,
    pub primary: bool,
    /// Pressing it also opens the app the way its manifest says.
    pub open: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Content {
    /// Drawn by Holster.
    #[serde(rename = "text")]
    Text { title: String, body: Option<String>, buttons: Vec<Button> },
    /// The app's own page, in Holster's card. `dismiss: false` leaves out
    /// Holster's `×`: the page draws its own way to dismiss (`close`).
    #[serde(rename = "page")]
    Page { url: String, width: f64, height: f64, dismiss: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: Option<String>,
    pub content: Content,
    /// None: stays until dismissed.
    pub timeout: Option<Duration>,
}

fn short_text(v: &Value, key: &str, max: usize) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.chars().count() > max {
                return Err(format!("{key} is longer than {max} characters"));
            }
            Ok((!s.is_empty()).then(|| s.to_string()))
        }
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn valid_token(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= 64 && !s.chars().any(char::is_control)
}

/// Validates a `POST /apps/{id}/notify` body.
pub fn parse_request(v: &Value) -> Result<Request, String> {
    if !v.is_object() {
        return Err("The body must be a JSON object".into());
    }
    let id = match v.get("id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if valid_token(s) => Some(s.clone()),
        Some(_) => return Err("id must be a string of 1 to 64 characters".into()),
    };
    let timeout = match v.get("timeout") {
        None | Some(Value::Null) => Some(Duration::from_secs_f64(DEFAULT_TIMEOUT)),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(0.0) => None,
            Some(x) if x > 0.0 && x <= MAX_TIMEOUT => Some(Duration::from_secs_f64(x)),
            _ => return Err(format!("timeout must be 0 (stays until dismissed) or 0 to {MAX_TIMEOUT} seconds")),
        },
        Some(_) => return Err("timeout must be a number of seconds".into()),
    };
    let content = match v.get("page") {
        Some(p) if !p.is_null() => {
            let url = p.get("url").and_then(Value::as_str).ok_or("page.url is missing")?;
            if !loopback_url(url) {
                return Err("page.url must be an http://127.0.0.1 URL".into());
            }
            let dim = |k: &str, min: f64, max: f64, def: f64| -> Result<f64, String> {
                match p.get(k) {
                    None | Some(Value::Null) => Ok(def),
                    Some(Value::Number(n)) => Ok(n.as_f64().unwrap_or(def).clamp(min, max).round()),
                    Some(_) => Err(format!("page.{k} must be a number")),
                }
            };
            let dismiss = match p.get("dismiss") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err("page.dismiss must be true or false".into()),
            };
            Content::Page {
                url: url.to_string(),
                width: dim("width", PAGE_MIN.0, PAGE_MAX.0, PAGE_DEFAULT.0)?,
                height: dim("height", PAGE_MIN.1, PAGE_MAX.1, PAGE_DEFAULT.1)?,
                dismiss,
            }
        }
        _ => {
            let title = short_text(v, "title", 120)?.ok_or("title is missing (or send a page)")?;
            let body = short_text(v, "body", 500)?;
            let buttons = match v.get("buttons") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(list)) => {
                    if list.len() > MAX_BUTTONS {
                        return Err(format!("at most {MAX_BUTTONS} buttons"));
                    }
                    let mut out: Vec<Button> = Vec::new();
                    for b in list {
                        let id = b.get("id").and_then(Value::as_str).filter(|s| valid_token(s)).ok_or("each button needs an id")?;
                        if out.iter().any(|o| o.id == id) {
                            return Err(format!("two buttons share the id \"{id}\""));
                        }
                        let label = short_text(b, "label", 24)?.ok_or("each button needs a label")?;
                        let flag = |k: &str| match b.get(k) {
                            None | Some(Value::Null) => Ok(false),
                            Some(Value::Bool(v)) => Ok(*v),
                            Some(_) => Err(format!("button.{k} must be true or false")),
                        };
                        let (primary, open) = (flag("primary")?, flag("open")?);
                        out.push(Button { id: id.to_string(), label, primary, open });
                    }
                    out
                }
                Some(_) => return Err("buttons must be a list".into()),
            };
            Content::Text { title, body, buttons }
        }
    };
    Ok(Request { id, content, timeout })
}

// ---- the stack ------------------------------------------------------------------------

#[derive(Clone)]
struct Note {
    /// Holster's own key (the app's id is only unique per app).
    key: u64,
    app: String,
    nid: String,
    content: Content,
    timeout: Option<Duration>,
    remaining: Option<Duration>,
    /// Content size in CSS px; text cards report theirs once drawn.
    size: Option<(f64, f64)>,
    slot: Option<usize>,
    assigned_at: Instant,
    /// Admission order: higher is newer, closer to the tab.
    order: u64,
    /// On screen (popped), at `rect`.
    out: bool,
    rect: Rect,
    hovered: bool,
    /// Horizontal flick in progress (CSS px, towards the edge).
    drag: Option<f64>,
    announced: bool,
}

/// What a card window shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotView {
    pub key: u64,
    pub app_id: String,
    pub content: Content,
    pub timed: bool,
    /// Content visible (false while it's being measured, or folding away).
    pub shown: bool,
}

#[derive(Default)]
struct State {
    notes: Vec<Note>,
    seq: u64,
    order: u64,
    /// Cards folding away: their windows keep the content until parked.
    ghosts: [Option<SlotView>; NOTE_SLOTS],
    /// Bumped when a slot's window gets a new job, so stale fold callbacks
    /// don't park it.
    gen: [u64; NOTE_SLOTS],
    /// The card folding back into the tab, if any: the tab stays hidden
    /// until it's there.
    tab_ghost: Option<usize>,
    ticking: bool,
}

pub struct Notifier {
    app: AppHandle,
    shell: Arc<Shell>,
    reg: Arc<Registry>,
    prefs: Arc<Prefs>,
    health: Arc<Health>,
    st: Mutex<State>,
}

pub struct Posted {
    pub id: String,
    pub muted: bool,
}

fn view(n: &Note) -> SlotView {
    let mut content = n.content.clone();
    // A page draws at its current size: the one it asked for at runtime.
    if let (Content::Page { width, height, .. }, Some((w, h))) = (&mut content, n.size) {
        (*width, *height) = (w, h);
    }
    SlotView { key: n.key, app_id: n.app.clone(), content, timed: n.timeout.is_some(), shown: n.out }
}

/// The stack, newest first: the newest card takes the tab's place (centered on
/// it, flush with the edge like the dock), older ones go towards the side
/// with more room, and the whole stack is kept on screen. Sizes in CSS px,
/// rects in screen px. The edge side reaches past the screen edge as far as
/// the tab does, so only the inner corners show.
fn layout(f: &NoteFrame, sizes: &[(f64, f64)]) -> Vec<Rect> {
    if sizes.is_empty() {
        return Vec::new();
    }
    let s = f.scale;
    let (gap, margin) = ((GAP * s).round(), (MARGIN * s).round());
    let (edge, right) = (f.work.r, f.tab.r);
    let cy = (f.tab.t + f.tab.b) / 2.0;
    let hs: Vec<f64> = sizes.iter().map(|(_, h)| (h * s).round()).collect();
    let down = f.work.b - cy >= cy - f.work.t;
    let mut tops = Vec::with_capacity(hs.len());
    let mut t = cy - hs[0] / 2.0;
    tops.push(t);
    for i in 1..hs.len() {
        t = if down { tops[i - 1] + hs[i - 1] + gap } else { tops[i - 1] - gap - hs[i] };
        tops.push(t);
    }
    let first = tops.iter().copied().fold(f64::INFINITY, f64::min);
    let last = tops.iter().zip(&hs).map(|(t, h)| t + h).fold(f64::NEG_INFINITY, f64::max);
    let mut shift = 0.0;
    if last > f.work.b - margin {
        shift = f.work.b - margin - last;
    }
    if first + shift < f.work.t + margin {
        shift = f.work.t + margin - first;
    }
    tops.iter()
        .zip(sizes)
        .zip(&hs)
        .map(|((t, (w, _)), h)| {
            let t = (t + shift).round();
            Rect { l: edge - (w * s).round(), t, r: right, b: t + h }
        })
        .collect()
}

/// Where a card that doesn't fold into the tab goes: a sliver at its edge
/// side, past the screen edge.
fn sliver(f: &NoteFrame, cur: Rect) -> Rect {
    let cy = (cur.t + cur.b) / 2.0;
    let h = (cur.h() * 0.6).max(32.0 * f.scale);
    Rect { l: cur.r - (10.0 * f.scale).round(), t: (cy - h / 2.0).round(), r: cur.r, b: (cy + h / 2.0).round() }
}

fn play_sound() {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
    let alias: Vec<u16> = "Notification.Default\0".encode_utf16().collect();
    unsafe { PlaySoundW(alias.as_ptr(), std::ptr::null_mut(), SND_ALIAS | SND_ASYNC | SND_NODEFAULT) };
}

impl Notifier {
    pub fn new(app: AppHandle, shell: Arc<Shell>, reg: Arc<Registry>, prefs: Arc<Prefs>, health: Arc<Health>) -> Arc<Self> {
        let me = Arc::new(Self { app, shell: shell.clone(), reg, prefs, health, st: Mutex::new(State::default()) });
        let weak = Arc::downgrade(&me);
        shell.set_on_change(move || {
            if let Some(n) = weak.upgrade() {
                n.refresh();
            }
        });
        me
    }

    /// Shows (or updates, same `id`) a notification from `app`.
    pub fn post(self: &Arc<Self>, app: &str, req: Request) -> Posted {
        let prefs = self.prefs.get();
        let id = {
            let mut st = self.st.lock().unwrap();
            st.seq += 1;
            let seq = st.seq;
            let nid = req.id.clone().unwrap_or_else(|| format!("n{seq}"));
            if prefs.muted.iter().any(|m| m == app) {
                return Posted { id: nid, muted: true };
            }
            let size = match &req.content {
                Content::Page { width, height, .. } => Some((*width, *height)),
                Content::Text { .. } => None,
            };
            if let Some(n) = st.notes.iter_mut().find(|n| n.app == app && n.nid == nid) {
                // Same id: new content in place, timer starts over.
                n.content = req.content;
                n.timeout = req.timeout;
                n.remaining = req.timeout;
                if size.is_some() {
                    n.size = size;
                }
            } else {
                st.notes.push(Note {
                    key: seq,
                    app: app.to_string(),
                    nid: nid.clone(),
                    content: req.content,
                    timeout: req.timeout,
                    remaining: req.timeout,
                    size,
                    slot: None,
                    assigned_at: Instant::now(),
                    order: 0,
                    out: false,
                    rect: Rect { l: 0.0, t: 0.0, r: 0.0, b: 0.0 },
                    hovered: false,
                    drag: None,
                    announced: false,
                });
            }
            nid
        };
        // An update to a card on screen shows right away.
        self.emit();
        // The dock is open: the app's tile pulses; the card waits for it to close.
        if self.shell.note_frame().open {
            if let Some(m) = self.reg.manifest(app) {
                let _ = self.app.emit_to("main", "signal", json!({ "id": app, "color": m.color }));
            }
        }
        self.refresh();
        Posted { id, muted: false }
    }

    /// The app takes a notification back. No event is sent for it.
    pub fn withdraw(self: &Arc<Self>, app: &str, nid: &str) -> bool {
        let key = self.st.lock().unwrap().notes.iter().find(|n| n.app == app && n.nid == nid).map(|n| n.key);
        match key {
            Some(k) => {
                self.finish(k, None, None);
                true
            }
            None => false,
        }
    }

    pub fn dismiss(self: &Arc<Self>, key: u64) {
        self.finish(key, Some("dismissed"), None);
    }

    /// A button was pressed: the card closes and the app hears `action`. A
    /// text card only sends its own buttons, and opens the app when the
    /// button says so; a page names its actions and asks to open (`open`).
    /// Returns the app id and the card's vertical center (screen px) when the
    /// caller should open the app.
    pub fn action(self: &Arc<Self>, key: u64, button: &str, page_open: bool) -> Option<(String, f64)> {
        let (open, app, cy) = {
            let st = self.st.lock().unwrap();
            let n = st.notes.iter().find(|n| n.key == key)?;
            let open = match &n.content {
                Content::Text { buttons, .. } => buttons.iter().find(|b| b.id == button)?.open,
                Content::Page { .. } if valid_token(button) => page_open,
                Content::Page { .. } => return None,
            };
            (open, n.app.clone(), ((n.rect.t + n.rect.b) / 2.0).round())
        };
        self.finish(key, Some("action"), Some(button));
        open.then_some((app, cy))
    }

    /// Removes a notification, folds its card, and tells the app why.
    fn finish(self: &Arc<Self>, key: u64, event: Option<&str>, button: Option<&str>) {
        let f = self.shell.note_frame();
        let (gone, last) = {
            let mut st = self.st.lock().unwrap();
            let Some(i) = st.notes.iter().position(|n| n.key == key) else { return };
            let gone = st.notes.remove(i);
            let last = !st.notes.iter().any(|n| n.out);
            (gone, last)
        };
        if let Some(slot) = gone.slot {
            // The last card on screen becomes the tab again; others go into the edge.
            self.fold(slot, &gone, &f, last && !f.held);
        }
        if let Some(e) = event {
            self.tell(&gone.app, &gone.nid, e, button);
        }
        self.refresh();
    }

    /// Folds a card away, into the tab's shape or into the edge; its window
    /// keeps the content until it's parked. Folding into the tab, the tab
    /// comes back under the card once it's there, and the card goes a moment
    /// later.
    fn fold(self: &Arc<Self>, slot: usize, n: &Note, f: &NoteFrame, into_tab: bool) {
        let into_tab = into_tab && n.out;
        let gen = {
            let mut st = self.st.lock().unwrap();
            st.gen[slot] += 1;
            if n.out {
                st.ghosts[slot] = Some(SlotView { shown: false, ..view(n) });
            }
            if into_tab {
                st.tab_ghost = Some(slot);
            } else if st.tab_ghost == Some(slot) {
                st.tab_ghost = None;
            }
            st.gen[slot]
        };
        self.emit();
        let win = self.shell.note_win(slot);
        if !n.out {
            park(win);
            return;
        }
        let me = self.clone();
        let gone = move |me: &Arc<Self>| {
            let mut st = me.st.lock().unwrap();
            if st.gen[slot] == gen {
                st.ghosts[slot] = None;
                drop(st);
                park(me.shell.note_win(slot));
                me.emit();
                me.refresh();
            }
        };
        if into_tab {
            win.anim.animate(
                f.tab,
                INTO_TAB_SPRING,
                None,
                Some(Box::new(move || {
                    {
                        let mut st = me.st.lock().unwrap();
                        if st.gen[slot] != gen {
                            return;
                        }
                        st.tab_ghost = None;
                    }
                    me.sync_tab();
                    me.shell.raise(me.shell.note_win(slot));
                    std::thread::spawn(move || {
                        std::thread::sleep(TAB_HANDOVER);
                        gone(&me);
                    });
                })),
            );
        } else {
            let to = sliver(f, win.anim.target());
            win.anim.animate(to, FOLD_SPRING, Some(Box::new(move || gone(&me))), None);
        }
    }

    /// The tab hides while a card stands in for it, and comes back when none does.
    fn sync_tab(self: &Arc<Self>) {
        let covered = {
            let st = self.st.lock().unwrap();
            st.tab_ghost.is_some() || st.notes.iter().any(|n| n.out)
        };
        self.shell.note_cover(covered);
    }

    /// A page that leaves out Holster's `×` must close itself, so it has to
    /// load. If it doesn't answer, Holster keeps its `×` on the card; returns
    /// why, for the app.
    pub async fn check_page(&self, req: &mut Request) -> Option<&'static str> {
        let Content::Page { url, dismiss, .. } = &mut req.content else { return None };
        if *dismiss {
            return None;
        }
        let answers = self.health.client().get(url.as_str()).send().await.is_ok_and(|r| r.status().is_success());
        if answers {
            return None;
        }
        *dismiss = true;
        Some("page.url didn't answer, so Holster draws its × on this notification")
    }

    fn tell(&self, app: &str, nid: &str, event: &str, button: Option<&str>) {
        let Some(url) = self.reg.manifest(app).and_then(|m| m.notify_url) else { return };
        let mut body = json!({ "id": nid, "event": event });
        if let Some(b) = button {
            body["button"] = json!(b);
        }
        let client = self.health.client().clone();
        tauri::async_runtime::spawn(async move {
            let _ = client.post(&url).json(&body).send().await;
        });
    }

    fn emit(&self) {
        let _ = self.app.emit("notes", self.slots());
    }

    pub fn slots(&self) -> Vec<Option<SlotView>> {
        let st = self.st.lock().unwrap();
        let mut slots: Vec<Option<SlotView>> = st.ghosts.to_vec();
        for n in &st.notes {
            if let Some(s) = n.slot {
                slots[s] = Some(view(n));
            }
        }
        slots
    }

    /// Settings changed: Do not disturb, muted apps.
    pub fn settings_changed(self: &Arc<Self>) {
        let muted = self.prefs.get().muted;
        let keys: Vec<u64> = self.st.lock().unwrap().notes.iter().filter(|n| muted.contains(&n.app)).map(|n| n.key).collect();
        for k in keys {
            self.finish(k, None, None);
        }
        self.refresh();
    }

    /// Brings the screen in line with the queue: folds everything while the
    /// tab is busy, otherwise admits waiting notifications into free windows
    /// and lays out the stack.
    pub fn refresh(self: &Arc<Self>) {
        let f = self.shell.note_frame();
        let held = f.held || self.prefs.get().do_not_disturb;
        let reg = self.reg.clone();
        let mut to_fold: Vec<(usize, Note)> = Vec::new();
        let mut changed = false;
        {
            let mut st = self.st.lock().unwrap();
            // Apps that left the dock take their notifications with them.
            let before = st.notes.len();
            st.notes.retain(|n| reg.manifest(&n.app).is_some() || n.slot.is_some());
            changed |= st.notes.len() != before;
            if held {
                for n in st.notes.iter_mut().filter(|n| n.slot.is_some()) {
                    let slot = n.slot.take().unwrap();
                    to_fold.push((slot, n.clone()));
                    n.out = false;
                    n.hovered = false;
                    n.drag = None;
                }
            } else {
                // Admit the oldest waiting notifications into free windows.
                let busy: Vec<usize> = st.notes.iter().filter_map(|n| n.slot).collect();
                let mut free: Vec<usize> =
                    (0..NOTE_SLOTS).filter(|s| !busy.contains(s) && st.ghosts[*s].is_none()).collect();
                let mut waiting: Vec<usize> =
                    st.notes.iter().enumerate().filter(|(_, n)| n.slot.is_none()).map(|(i, _)| i).collect();
                waiting.sort_by_key(|&i| st.notes[i].key);
                for i in waiting {
                    let Some(slot) = free.first().copied() else { break };
                    free.remove(0);
                    st.order += 1;
                    let order = st.order;
                    st.gen[slot] += 1;
                    let n = &mut st.notes[i];
                    n.slot = Some(slot);
                    n.order = order;
                    n.assigned_at = Instant::now();
                    n.out = false;
                    changed = true;
                }
            }
        }
        // Folding for a wait: no event, the notification stays queued. With
        // only Do not disturb holding them, the newest folds back into the tab.
        let newest = to_fold.iter().filter(|(_, n)| n.out).map(|(_, n)| n.order).max();
        for (slot, n) in &to_fold {
            self.fold(slot.to_owned(), n, &f, !f.held && Some(n.order) == newest);
        }
        if held {
            self.sync_tab();
            if changed || !to_fold.is_empty() {
                self.emit();
            }
            return;
        }
        if changed {
            // New content goes to its window first, so text cards can measure.
            self.emit();
        }
        self.place(&f);
        self.ensure_ticking();
    }

    /// Lays out the cards that are ready (sized) and moves their windows.
    fn place(self: &Arc<Self>, f: &NoteFrame) {
        let sound = self.prefs.get().notify_sound;
        let mut pops: Vec<(usize, Rect)> = Vec::new();
        let mut moves: Vec<(usize, Rect)> = Vec::new();
        let mut announce = false;
        {
            let mut st = self.st.lock().unwrap();
            let mut ready: Vec<usize> =
                st.notes.iter().enumerate().filter(|(_, n)| n.slot.is_some() && n.size.is_some()).map(|(i, _)| i).collect();
            ready.sort_by_key(|&i| std::cmp::Reverse(st.notes[i].order));
            let sizes: Vec<(f64, f64)> = ready.iter().map(|&i| st.notes[i].size.unwrap()).collect();
            let rects = layout(f, &sizes);
            for (&i, rect) in ready.iter().zip(rects) {
                let n = &mut st.notes[i];
                let slot = n.slot.unwrap();
                let moved = n.rect != rect;
                n.rect = rect;
                if !n.out {
                    n.out = true;
                    if !n.announced {
                        n.announced = true;
                        announce = true;
                    }
                    pops.push((slot, rect));
                } else if moved && n.drag.is_none() {
                    moves.push((slot, rect));
                }
            }
        }
        if !pops.is_empty() {
            self.emit();
        }
        for (slot, rect) in pops.iter().copied() {
            // The card starts as the tab, above it, and grows out of it.
            let win = self.shell.note_win(slot);
            win.anim.jump(f.tab);
            self.shell.raise(win);
            win.anim.animate(rect, POP_SPRING, None, None);
        }
        if !pops.is_empty() {
            self.sync_tab();
        }
        for (slot, rect) in moves {
            self.shell.note_win(slot).anim.animate(rect, GLIDE_SPRING, None, None);
        }
        if announce && sound {
            play_sound();
        }
    }

    /// A text card reports its drawn height (CSS px).
    pub fn measured(self: &Arc<Self>, key: u64, height: f64) {
        let h = height.clamp(TEXT_H.0, TEXT_H.1).round();
        {
            let mut st = self.st.lock().unwrap();
            let Some(n) = st.notes.iter_mut().find(|n| n.key == key) else { return };
            if !matches!(n.content, Content::Text { .. }) || n.size == Some((CARD_W, h)) {
                return;
            }
            n.size = Some((CARD_W, h));
        }
        self.refresh();
    }

    /// A page card asks for a new size (postMessage resize).
    pub fn resize(self: &Arc<Self>, key: u64, w: f64, h: f64) {
        let size = (w.clamp(PAGE_MIN.0, PAGE_MAX.0).round(), h.clamp(PAGE_MIN.1, PAGE_MAX.1).round());
        {
            let mut st = self.st.lock().unwrap();
            let Some(n) = st.notes.iter_mut().find(|n| n.key == key) else { return };
            if !matches!(n.content, Content::Page { .. }) || n.size == Some(size) {
                return;
            }
            n.size = Some(size);
        }
        self.emit();
        self.refresh();
    }

    /// The cursor is on a card: its timer waits.
    pub fn hover(self: &Arc<Self>, key: u64, on: bool) {
        let mut st = self.st.lock().unwrap();
        if let Some(n) = st.notes.iter_mut().find(|n| n.key == key) {
            n.hovered = on;
            if !on {
                if let Some(r) = n.remaining.as_mut() {
                    *r = (*r).max(AFTER_HOVER);
                }
            }
        }
    }

    /// Flicking a card back into the edge. `dx` is how far it has been pulled
    /// towards the edge (CSS px).
    pub fn drag(self: &Arc<Self>, key: u64, phase: &str, dx: f64) {
        let f = self.shell.note_frame();
        let (slot, rect, w) = {
            let mut st = self.st.lock().unwrap();
            let Some(n) = st.notes.iter_mut().find(|n| n.key == key && n.out) else { return };
            let dx = dx.max(0.0);
            n.drag = (phase == "move").then_some(dx);
            (n.slot.unwrap(), n.rect, n.size.map(|s| s.0).unwrap_or(CARD_W))
        };
        let win = self.shell.note_win(slot);
        let px = (dx.max(0.0) * f.scale).round();
        match phase {
            "move" => win.anim.jump(Rect { l: rect.l + px, r: rect.r + px, ..rect }),
            _ if dx >= FLICK_PX || dx >= w * FLICK_SHARE => self.dismiss(key),
            _ => win.anim.animate(rect, GLIDE_SPRING, None, None),
        }
    }

    fn ensure_ticking(self: &Arc<Self>) {
        let start = {
            let mut st = self.st.lock().unwrap();
            let needed = st.notes.iter().any(|n| n.slot.is_some() && (n.timeout.is_some() || n.size.is_none()));
            needed && !std::mem::replace(&mut st.ticking, true)
        };
        if start {
            let me = self.clone();
            std::thread::Builder::new().name("holster-notify".into()).spawn(move || me.tick()).expect("notify timer");
        }
    }

    /// Counts down the cards on screen, ten times a second, only while any
    /// are timed (or waiting to be measured).
    fn tick(self: Arc<Self>) {
        let mut last = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let dt = last.elapsed();
            last = Instant::now();
            let held = self.shell.note_frame().held || self.prefs.get().do_not_disturb;
            let (expired, unmeasured) = {
                let mut st = self.st.lock().unwrap();
                let mut expired = Vec::new();
                let mut unmeasured = false;
                for n in st.notes.iter_mut().filter(|n| n.slot.is_some()) {
                    if n.size.is_none() && n.assigned_at.elapsed() >= MEASURE_WAIT {
                        n.size = Some(match n.content {
                            Content::Page { width, height, .. } => (width, height),
                            Content::Text { .. } => (CARD_W, MEASURE_FALLBACK_H),
                        });
                        unmeasured = true;
                    }
                    if held || !n.out || n.hovered || n.drag.is_some() {
                        continue;
                    }
                    if let Some(r) = n.remaining.as_mut() {
                        *r = r.saturating_sub(dt);
                        if r.is_zero() {
                            expired.push(n.key);
                        }
                    }
                }
                let needed = st.notes.iter().any(|n| n.slot.is_some() && (n.timeout.is_some() || n.size.is_none()));
                if !needed && expired.is_empty() && !unmeasured {
                    st.ticking = false;
                    return;
                }
                (expired, unmeasured)
            };
            for k in expired {
                self.finish(k, Some("expired"), None);
            }
            if unmeasured {
                self.refresh();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_notifications() {
        let r = parse_request(&json!({
            "id": "m-42", "title": " Meeting ", "body": "Design review",
            "buttons": [{ "id": "snooze", "label": "Snooze" }, { "id": "join", "label": "Join", "primary": true }],
            "timeout": 8
        }))
        .unwrap();
        assert_eq!(r.id.as_deref(), Some("m-42"));
        assert_eq!(r.timeout, Some(Duration::from_secs(8)));
        let Content::Text { title, buttons, .. } = r.content else { panic!() };
        assert_eq!(title, "Meeting");
        assert!(buttons[1].primary && !buttons[0].primary);
        assert!(!buttons[0].open && !buttons[1].open);
        let r = parse_request(&json!({ "title": "x", "buttons": [{ "id": "join", "label": "Join", "open": true }] })).unwrap();
        let Content::Text { buttons, .. } = r.content else { panic!() };
        assert!(buttons[0].open);
        assert!(parse_request(&json!({ "title": "x", "buttons": [{ "id": "a", "label": "A", "open": "yes" }] })).is_err());
    }

    #[test]
    fn timeouts() {
        let t = |v: Value| parse_request(&json!({ "title": "x", "timeout": v })).map(|r| r.timeout);
        assert_eq!(t(Value::Null).unwrap(), Some(Duration::from_secs(6)));
        assert_eq!(t(json!(0)).unwrap(), None);
        assert!(t(json!(-1)).is_err());
        assert!(t(json!(99999)).is_err());
        assert!(t(json!("5")).is_err());
    }

    #[test]
    fn rejects_bad_requests() {
        assert!(parse_request(&json!({})).unwrap_err().contains("title"));
        assert!(parse_request(&json!({ "title": "x", "buttons": [{ "id": "a", "label": "A" }, { "id": "a", "label": "B" }] })).is_err());
        assert!(parse_request(&json!({ "title": "x", "buttons": [{}, {}, {}] })).is_err());
        assert!(parse_request(&json!({ "page": { "url": "http://example.com" } })).unwrap_err().contains("127.0.0.1"));
    }

    #[test]
    fn pages_are_clamped() {
        let r = parse_request(&json!({ "page": { "url": "http://127.0.0.1:5000/n", "width": 9999 } })).unwrap();
        assert_eq!(r.content, Content::Page { url: "http://127.0.0.1:5000/n".into(), width: PAGE_MAX.0, height: PAGE_DEFAULT.1, dismiss: true });
    }

    #[test]
    fn pages_can_bring_their_own_dismiss() {
        let page = |d: Value| parse_request(&json!({ "page": { "url": "http://127.0.0.1:5000/n", "dismiss": d } }));
        assert!(matches!(page(json!(false)).unwrap().content, Content::Page { dismiss: false, .. }));
        assert!(matches!(page(Value::Null).unwrap().content, Content::Page { dismiss: true, .. }));
        assert!(page(json!("no")).unwrap_err().contains("page.dismiss"));
    }

    fn frame(tab_y: f64) -> NoteFrame {
        NoteFrame {
            scale: 1.0,
            work: Rect { l: 0.0, t: 0.0, r: 1920.0, b: 1040.0 },
            tab: Rect { l: 1906.0, t: tab_y - 44.0, r: 1930.0, b: tab_y + 44.0 },
            held: false,
            open: false,
        }
    }

    #[test]
    fn stack_grows_towards_the_room() {
        let sizes = [(340.0, 100.0), (340.0, 80.0)];
        let low = layout(&frame(300.0), &sizes);
        // Flush with the edge, reaching past it like the tab.
        assert_eq!((low[0].l, low[0].t, low[0].r), (1580.0, 250.0, 1930.0));
        assert!(low[1].t > low[0].b, "older card below");
        let high = layout(&frame(900.0), &sizes);
        assert!(high[1].b < high[0].t, "older card above");
    }

    #[test]
    fn stack_stays_on_screen() {
        let r = layout(&frame(20.0), &[(340.0, 200.0)]);
        assert_eq!(r[0].t, 8.0);
    }
}
