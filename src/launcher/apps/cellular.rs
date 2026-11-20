//! Cellular — phone, SMS and modem settings for the QDC507 4G module.
//!
//! The layout follows the agreed three-tab design. Focus is two-level:
//! at the tab level Tab cycles Phone → Message → Settings and Enter
//! descends into the tab's content; once inside, Tab cycles the view's
//! own focusable items (dialer ↔ history, the thread rows, the New
//! message fields, the settings rows) and Esc climbs back level by
//! level. Note the shell's own rule: a short Esc never leaves an app —
//! leaving is the host's long-press Esc (0.9 s) or the menu's Quit.
//!
//! All modem work goes through the harbour-celld daemon (see
//! `crate::system::cell`); this app only talks IPC, on a worker thread,
//! and never blocks the UI on it. Voice calls go through the daemon too:
//! it owns the call state machine and the audio route, and this app
//! polls `call.state` once a second and renders what the daemon says.
//!
//! Character input arrives through `text()` (the shell's IME channel);
//! `key()` handles navigation and control keys only, so nothing is ever
//! inserted twice.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::rect::Rect;
use sdl2::render::Canvas;
use sdl2::video::Window;
use serde::{Deserialize, Serialize};

use crate::assets::AssetStore;
use crate::chrome;
use crate::font::FontSystem;
use crate::grid;
use crate::launcher::app::{AppAction, Ctx, LauncherApp};
use crate::launcher::i18n::tr_s;
use crate::launcher::lineedit::LineEdit;
use crate::launcher::widgets::{self, MenuItem};
use crate::platform::keys::arrow_alias;
use crate::system::cell::{self, SmsMessage};
use crate::system::CellStatus;
use crate::theme::ThemeManager;
use crate::ui;

const LEFT: i32 = grid::MARGIN + 1;
const RIGHT: i32 = grid::SCREEN_W - grid::MARGIN - 1;
/// Content starts under the shared folder-tab strip (FOLDER_BODY_Y = 36).
const CONTENT_Y: i32 = widgets::FOLDER_BODY_Y + 2;
/// The screen's usable bottom (DESIGN: content may run to y 168).
const BOTTOM: i32 = grid::APP_FRAME_BOTTOM;
const CONTENT_H: i32 = BOTTOM - CONTENT_Y;
const REFRESH_EVERY: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Small pure helpers (unit-tested below)
// ---------------------------------------------------------------------------

/// "HH:MM" today, "Yest." yesterday, the weekday within a week, "MM-DD"
/// beyond that; the full form is "YYYY-MM-DD HH:MM".
fn fmt_ts(ts: i64, full: bool) -> String {
    let Some(dt) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    if full {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    let days = (Local::now().date_naive() - dt.date_naive()).num_days();
    match days {
        0 => dt.format("%H:%M").to_string(),
        1 => tr_s("Yest."),
        2..=6 => dt.format("%a").to_string(),
        _ => dt.format("%m-%d").to_string(),
    }
}

fn fmt_clock(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// One SMS conversation: all messages with one address.
#[derive(Debug, Clone, PartialEq)]
struct Thread {
    address: String,
    last_ts: i64,
    preview: String,
    unread: usize,
}

/// Group messages into threads, newest activity first. `read` holds the
/// ids of incoming messages the user has already seen.
fn threads(messages: &[SmsMessage], read: &HashSet<i64>) -> Vec<Thread> {
    let mut out: Vec<Thread> = Vec::new();
    for m in messages {
        let preview: String = m.body.lines().next().unwrap_or("").chars().take(48).collect();
        match out.iter_mut().find(|t| t.address == m.address) {
            Some(t) => {
                if m.ts >= t.last_ts {
                    t.last_ts = m.ts;
                    t.preview = preview;
                }
                if m.direction == "in" && !read.contains(&m.id) {
                    t.unread += 1;
                }
            }
            None => out.push(Thread {
                address: m.address.clone(),
                last_ts: m.ts,
                preview,
                unread: usize::from(m.direction == "in" && !read.contains(&m.id)),
            }),
        }
    }
    out.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
    out
}

fn thread_messages<'a>(messages: &'a [SmsMessage], address: &str) -> Vec<&'a SmsMessage> {
    let mut v: Vec<&SmsMessage> = messages.iter().filter(|m| m.address == address).collect();
    v.sort_by_key(|m| m.ts);
    v
}

const GSM_BASIC: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";

/// (characters, per-segment capacity, segment count) for the composer
/// header: GSM-7 packs 160 (153 in multipart), UCS-2 packs 70 (67).
fn sms_segments(text: &str) -> (usize, usize, usize) {
    let n = text.chars().count();
    if n == 0 {
        return (0, 160, 0);
    }
    if text.chars().all(|c| GSM_BASIC.contains(c)) {
        if n <= 160 { (n, 160, 1) } else { (n, 153, n.div_ceil(153)) }
    } else if n <= 70 {
        (n, 70, 1)
    } else {
        (n, 67, n.div_ceil(67))
    }
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2} GB", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.0} MB", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.0} KB", n as f64 / 1e3)
    } else {
        format!("{n} B")
    }
}

fn fmt_rate(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1} MB/s", n / 1e6)
    } else if n >= 1_000.0 {
        format!("{:.0} KB/s", n / 1e3)
    } else {
        format!("{:.0} B/s", n)
    }
}

/// Wrap `body` to lines of at most `max_w` pixels, honouring newlines.
fn wrap_text(fs: &FontSystem, body: &str, max_w: i32) -> Vec<String> {
    let mut out = Vec::new();
    for para in body.split('\n') {
        let para = para.strip_suffix('\r').unwrap_or(para);
        if para.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for ch in para.chars() {
            let trial = format!("{line}{ch}");
            if !line.is_empty() && fs.measure_text(&trial) > max_w {
                out.push(std::mem::take(&mut line));
                line.push(ch);
            } else {
                line = trial;
            }
        }
        out.push(line);
    }
    out
}

/// Draw `edit`'s text in a field `w` wide, shifted so the caret is visible.
fn draw_input(
    c: &mut Canvas<Window>,
    fs: &FontSystem,
    tm: &ThemeManager,
    edit: &LineEdit,
    x: i32,
    y: i32,
    w: i32,
) {
    draw_input_col(c, fs, tm, edit, x, y, w, tm.get_color("foreground"));
}

fn draw_input_col(
    c: &mut Canvas<Window>,
    fs: &FontSystem,
    tm: &ThemeManager,
    edit: &LineEdit,
    x: i32,
    y: i32,
    w: i32,
    ink: sdl2::pixels::Color,
) {
    let _ = tm;
    let shift = (edit.caret_x(fs) - w + 12).max(0);
    let mut acc = 0;
    let mut shown = String::new();
    for ch in edit.text.chars() {
        if acc >= shift {
            shown.push(ch);
        }
        acc += fs.measure_text(&ch.to_string());
    }
    fs.draw_text(c, x, y, &shown, ink);
    let before: String = edit.text.chars().take(edit.caret).collect();
    let cx = fs.measure_text(&before) - shift;
    if (0..w).contains(&cx) {
        ui::fill_rect(c, Rect::new(x + cx, y + 1, 1, 11), ink);
    }
}

fn draw_centered(fs: &FontSystem, c: &mut Canvas<Window>, y: i32, text: &str, col: sdl2::pixels::Color) {
    let w = fs.measure_text(text);
    fs.draw_text(c, 160 - w / 2, y, text, col);
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Phone,
    Message,
    Settings,
}

impl Tab {
    fn step(self, d: i32) -> Tab {
        let all = [Tab::Phone, Tab::Message, Tab::Settings];
        let i = all.iter().position(|t| *t == self).unwrap() as i32;
        all[(i + d).rem_euclid(3) as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhoneFocus {
    Dial,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    In,
    Out,
    Missed,
}

#[derive(Debug, Clone)]
struct CallEntry {
    id: i64,
    number: String,
    ts: i64,
    kind: CallKind,
}

/// An on-screen call. The daemon owns the truth; `phase` and
/// `secs_base` are its last report, and the timer interpolates locally
/// between the one-second polls while the call is connected.
struct CallUi {
    number: String,
    phase: cell::CallPhase,
    secs_base: u64,
    secs_at: Instant,
    volume: u8,
    muted: bool,
    dtmf: String,
}

impl CallUi {
    fn secs(&self) -> u64 {
        if self.phase == cell::CallPhase::Active {
            self.secs_base + self.secs_at.elapsed().as_secs()
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MsgView {
    List,
    Thread,
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetItem {
    Data,
    Route,
    Voice,
    SpkGain,
    MicGain,
    Reboot,
}

const SET_ITEMS: [SetItem; 6] = [
    SetItem::Data,
    SetItem::Route,
    SetItem::Voice,
    SetItem::SpkGain,
    SetItem::MicGain,
    SetItem::Reboot,
];

/// Persisted UI preferences (~/.config/harbour/cellular.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Prefs {
    #[serde(default)]
    route_4g_first: bool,
    #[serde(default = "default_spk")]
    spk_gain: u8,
    #[serde(default = "default_mic")]
    mic_gain: u8,
    #[serde(default)]
    read_ids: Vec<i64>,
}

fn default_spk() -> u8 {
    70
}

fn default_mic() -> u8 {
    100
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { route_4g_first: false, spk_gain: 70, mic_gain: 100, read_ids: Vec::new() }
    }
}

impl Prefs {
    fn path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/harbour/cellular.json"))
    }

    fn load() -> Prefs {
        let Some(p) = Self::path() else { return Prefs::default() };
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Prefs::default(),
        }
    }

    fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string(self) {
                let _ = std::fs::write(&p, s);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Worker (daemon IPC off the UI thread)
// ---------------------------------------------------------------------------

enum Job {
    Refresh,
    SetData(bool),
    Send { to: String, text: String },
    Dial(String),
    Answer,
    Hangup,
    Dtmf(String),
    SetVolume(u8),
    SetMute(bool),
    LoadVoice,
    CallPoll,
    DeleteCall(i64),
    DeleteThread(String),
    RebootModem,
}

enum Done {
    Data {
        status: Result<CellStatus, String>,
        messages: Result<Vec<SmsMessage>, String>,
        stats: Option<(u64, u64)>,
        calls: Result<Vec<cell::CallLogEntry>, String>,
        voice: Result<cell::VoiceState, String>,
    },
    Sent,
    CallState(Result<cell::CallState, String>),
    VoiceLoaded(Result<(), String>),
    Deleted(&'static str),
    RebootSent,
    Failed(String),
}

fn run_job(job: Job) -> Done {
    match job {
        Job::Refresh => Done::Data {
            status: cell::status(),
            messages: cell::sms_list(200, 0),
            stats: cell::iface_stats("usb0"),
            calls: cell::call_list(50, 0),
            voice: cell::voice_status(),
        },
        Job::SetData(on) => match cell::set_data(on) {
            Ok(()) => Done::Data {
                status: cell::status(),
                messages: cell::sms_list(200, 0),
                stats: cell::iface_stats("usb0"),
                calls: cell::call_list(50, 0),
                voice: cell::voice_status(),
            },
            Err(e) => Done::Failed(e),
        },
        Job::Send { to, text } => match cell::sms_send(&to, &text) {
            Ok(_) => Done::Sent,
            Err(e) => Done::Failed(e),
        },
        Job::Dial(number) => match cell::call_dial(&number) {
            Ok(()) => Done::CallState(cell::call_state()),
            Err(e) => Done::Failed(e),
        },
        Job::Answer => match cell::call_answer() {
            Ok(()) => Done::CallState(cell::call_state()),
            Err(e) => Done::Failed(e),
        },
        Job::Hangup => match cell::call_hangup() {
            Ok(()) => Done::CallState(cell::call_state()),
            Err(e) => Done::Failed(e),
        },
        Job::Dtmf(digits) => match cell::call_dtmf(&digits) {
            Ok(()) => Done::CallState(cell::call_state()),
            Err(e) => Done::Failed(e),
        },
        Job::SetVolume(v) => match cell::call_set_volume(v) {
            Ok(()) => Done::CallState(cell::call_state()),
            Err(e) => Done::Failed(e),
        },
        Job::SetMute(m) => match cell::call_set_mute(m) {
            Ok(()) => Done::CallState(cell::call_state()),
            Err(e) => Done::Failed(e),
        },
        Job::LoadVoice => Done::VoiceLoaded(cell::voice_load()),
        Job::CallPoll => Done::CallState(cell::call_state()),
        Job::DeleteCall(id) => match cell::call_delete(id) {
            Ok(()) => Done::Deleted("Call record deleted"),
            Err(e) => Done::Failed(e),
        },
        Job::RebootModem => match cell::modem_reboot() {
            Ok(()) => Done::RebootSent,
            Err(e) => Done::Failed(e),
        },
        Job::DeleteThread(addr) => match cell::sms_delete_address(&addr) {
            Ok(_) => Done::Deleted("Thread deleted"),
            Err(e) => Done::Failed(e),
        },
    }
}

pub struct CellularApp {
    tab: Tab,
    /// Two-level focus: Tab only moves the tab highlight; Enter descends
    /// into the tab's content (`entered`), Esc at a content top level
    /// climbs back to the tab level.
    entered: bool,
    /// The text event trailing the N keypress that opened the New view:
    /// swallowed once so the "n" does not land in the To field.
    swallow_text: Option<Instant>,
    // Data
    status: Option<CellStatus>,
    messages: Vec<SmsMessage>,
    prefs: Prefs,
    read: HashSet<i64>,
    // Phone tab
    dial: LineEdit,
    phone_focus: PhoneFocus,
    history: Vec<CallEntry>,
    history_sel: usize,
    call: Option<CallUi>,
    incoming: Option<String>,
    voice: Option<cell::VoiceState>,
    last_call_poll: Option<Instant>,
    // Message tab
    msg_view: MsgView,
    thread_sel: usize,
    thread_top: usize,
    thread_addr: String,
    thread_input: LineEdit,
    thread_back: i32,
    new_to: LineEdit,
    new_body: LineEdit,
    new_focus_body: bool,
    // Settings tab
    set_sel: usize,
    set_editing: Option<SetItem>,
    confirm_reboot: bool,
    // Throughput sampling
    last_stats: Option<(u64, u64, Instant)>,
    rx_rate: f64,
    tx_rate: f64,
    total_bytes: u64,
    // Plumbing
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<Done>>,
    busy: bool,
    note: Option<String>,
    note_at: Instant,
    last_refresh: Option<Instant>,
    last_tick: Instant,
}

pub fn make() -> Box<dyn LauncherApp> {
    Box::new(CellularApp::new())
}

impl CellularApp {
    fn new() -> CellularApp {
        let prefs = Prefs::load();
        let read: HashSet<i64> = prefs.read_ids.iter().copied().collect();
        CellularApp {
            tab: Tab::Phone,
            entered: false,
            swallow_text: None,
            status: None,
            messages: Vec::new(),
            prefs,
            read,
            dial: LineEdit::default(),
            phone_focus: PhoneFocus::Dial,
            history: Vec::new(),
            history_sel: 0,
            call: None,
            incoming: None,
            voice: None,
            last_call_poll: None,
            msg_view: MsgView::List,
            thread_sel: 0,
            thread_top: 0,
            thread_addr: String::new(),
            thread_input: LineEdit::default(),
            thread_back: 0,
            new_to: LineEdit::default(),
            new_body: LineEdit::default(),
            new_focus_body: false,
            set_sel: 0,
            set_editing: None,
            confirm_reboot: false,
            last_stats: None,
            rx_rate: 0.0,
            tx_rate: 0.0,
            total_bytes: 0,
            jobs: None,
            done: None,
            busy: false,
            note: None,
            note_at: Instant::now(),
            last_refresh: None,
            last_tick: Instant::now(),
        }
    }

    fn set_note(&mut self, note: &str) {
        self.note = Some(note.to_string());
        self.note_at = Instant::now();
    }

    fn dispatch(&mut self, job: Job) {
        self.busy = true;
        self.dispatch_inner(job);
    }

    /// Like `dispatch`, but does not mark the UI busy: the one-second
    /// call-state poll must not gate the five-second data refresh.
    fn dispatch_bg(&mut self, job: Job) {
        self.dispatch_inner(job);
    }

    fn dispatch_inner(&mut self, job: Job) {
        if self.jobs.is_none() {
            let (job_tx, job_rx) = std::sync::mpsc::channel::<Job>();
            let (done_tx, done_rx) = std::sync::mpsc::channel::<Done>();
            std::thread::spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    if done_tx.send(run_job(job)).is_err() {
                        break;
                    }
                }
            });
            self.jobs = Some(job_tx);
            self.done = Some(done_rx);
        }
        if let Some(tx) = &self.jobs {
            let _ = tx.send(job);
        }
    }

    fn refresh(&mut self) {
        self.last_refresh = Some(Instant::now());
        self.dispatch(Job::Refresh);
    }

    fn unread(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| m.direction == "in" && !self.read.contains(&m.id))
            .count()
    }

    fn mark_thread_read(&mut self, address: &str) {
        let mut changed = false;
        for m in self.messages.iter().filter(|m| m.address == address && m.direction == "in") {
            if self.read.insert(m.id) {
                changed = true;
            }
        }
        if changed {
            self.prefs.read_ids = self.read.iter().copied().collect();
            self.prefs.save();
        }
    }

    fn drain(&mut self) -> bool {
        let mut changed = false;
        let mut results = Vec::new();
        if let Some(rx) = &self.done {
            while let Ok(d) = rx.try_recv() {
                results.push(d);
            }
        }
        for d in results {
            changed = true;
            self.busy = false;
            match d {
                Done::Data { status, messages, stats, calls, voice } => {
                    match status {
                        Ok(st) => self.status = Some(st),
                        Err(_) => self.status = Some(CellStatus::default()),
                    }
                    if let Ok(msgs) = messages {
                        self.messages = msgs;
                        let n = threads(&self.messages, &self.read).len();
                        if self.thread_sel >= n {
                            self.thread_sel = n.saturating_sub(1);
                        }
                        if self.msg_view == MsgView::Thread
                            && thread_messages(&self.messages, &self.thread_addr).is_empty()
                        {
                            self.msg_view = MsgView::List;
                        }
                    }
                    if let Some((rx, tx)) = stats {
                        let now = Instant::now();
                        if let Some((prx, ptx, when)) = self.last_stats {
                            let dt = now.duration_since(when).as_secs_f64();
                            if dt > 0.5 {
                                self.rx_rate = (rx.saturating_sub(prx)) as f64 / dt;
                                self.tx_rate = (tx.saturating_sub(ptx)) as f64 / dt;
                            }
                        }
                        self.total_bytes = rx + tx;
                        self.last_stats = Some((rx, tx, now));
                    }
                    if let Ok(calls) = calls {
                        self.history = calls
                            .iter()
                            .map(|e| CallEntry {
                                id: e.id,
                                number: e.address.clone(),
                                ts: e.ts,
                                kind: match (e.direction.as_str(), e.outcome.as_str()) {
                                    ("in", "answered") => CallKind::In,
                                    ("in", _) => CallKind::Missed,
                                    _ => CallKind::Out,
                                },
                            })
                            .collect();
                        if self.history_sel >= self.history.len() {
                            self.history_sel = self.history.len().saturating_sub(1);
                        }
                    }
                    if let Ok(v) = voice {
                        self.voice = Some(v);
                    }
                }
                Done::Sent => {
                    self.set_note(&tr_s("Message sent"));
                    self.thread_back = 0;
                    self.refresh();
                }
                Done::CallState(Ok(cs)) => {
                    self.apply_call_state(cs);
                }
                Done::CallState(Err(_)) => {}
                Done::VoiceLoaded(Ok(())) => {
                    self.set_note(&tr_s("Voice runtime loaded"));
                    self.refresh();
                }
                Done::VoiceLoaded(Err(e)) => {
                    self.set_note(&e);
                }
                Done::Deleted(msg) => {
                    self.set_note(&tr_s(msg));
                    self.refresh();
                }
                Done::RebootSent => {
                    self.set_note(&tr_s("Modem rebooting — back in about a minute"));
                }
                Done::Failed(e) => {
                    self.set_note(&e);
                }
            }
        }
        changed
    }

    fn send_sms(&mut self, to: &str, text: &str) -> AppAction {
        let (to, text) = (to.trim().to_string(), text.trim().to_string());
        if to.is_empty() || text.is_empty() {
            return AppAction::Toast(tr_s("Enter a number and a message"));
        }
        self.set_note(&tr_s("Sending..."));
        self.dispatch(Job::Send { to, text });
        AppAction::None
    }

    /// Dial for real: show the call screen optimistically; the daemon's
    /// one-second state poll corrects phase and timer from here on.
    fn start_call(&mut self, number: &str) {
        let number = number.trim().to_string();
        if number.is_empty() {
            return;
        }
        self.call = Some(CallUi {
            number: number.clone(),
            phase: cell::CallPhase::Dialing,
            secs_base: 0,
            secs_at: Instant::now(),
            volume: self.prefs.spk_gain,
            muted: false,
            dtmf: String::new(),
        });
        self.dispatch(Job::Dial(number));
    }

    /// Reconcile the UI with the daemon's call state (polled 1/s).
    fn apply_call_state(&mut self, cs: cell::CallState) {
        use cell::CallPhase::*;
        match cs.phase {
            Idle => {
                if self.call.take().is_some() {
                    self.set_note(&tr_s("Call ended"));
                    self.refresh();
                }
                self.incoming = None;
            }
            Incoming => {
                if self.call.is_none() {
                    let num = if cs.number.is_empty() {
                        tr_s("Unknown number")
                    } else {
                        cs.number.clone()
                    };
                    if self.incoming.as_deref() != Some(num.as_str()) {
                        self.incoming = Some(num);
                    }
                }
            }
            Dialing | Alerting | Active => {
                self.incoming = None;
                match &mut self.call {
                    Some(call) => {
                        if !cs.number.is_empty() {
                            call.number = cs.number.clone();
                        }
                        call.phase = cs.phase;
                        call.secs_base = cs.duration;
                        call.secs_at = Instant::now();
                    }
                    None => {
                        self.call = Some(CallUi {
                            number: cs.number.clone(),
                            phase: cs.phase,
                            secs_base: cs.duration,
                            secs_at: Instant::now(),
                            volume: self.prefs.spk_gain,
                            muted: false,
                            dtmf: String::new(),
                        });
                        self.tab = Tab::Phone;
                        self.entered = true;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Input — Tab / Enter / Esc are the whole navigation model
// ---------------------------------------------------------------------------

fn is_enter(key: Keycode) -> bool {
    matches!(key, Keycode::Return | Keycode::KpEnter)
}

impl CellularApp {
    fn key_phone(&mut self, key: Keycode, mods: Mod) -> AppAction {
        if self.call.is_some() {
            match key {
                k if is_enter(k) => {
                    // Answer from the call screen itself when it shows a
                    // ringing incoming call — the popup is the primary
                    // surface, but the keys must work here too.
                    if matches!(self.call.as_ref().map(|c| &c.phase), Some(cell::CallPhase::Incoming)) {
                        self.dispatch(Job::Answer);
                    }
                }
                Keycode::Escape => {
                    // Hang up for real; the state poll clears the screen
                    // once the daemon confirms the call is over.
                    self.set_note(&tr_s("Hanging up..."));
                    self.dispatch(Job::Hangup);
                }
                Keycode::M => {
                    if let Some(call) = &mut self.call {
                        call.muted = !call.muted;
                        let m = call.muted;
                        self.dispatch_bg(Job::SetMute(m));
                    }
                }
                Keycode::Left | Keycode::Z => {
                    if let Some(call) = &mut self.call {
                        call.volume = call.volume.saturating_sub(5);
                        self.prefs.spk_gain = call.volume;
                        self.prefs.save();
                        let v = call.volume;
                        self.dispatch_bg(Job::SetVolume(v));
                    }
                }
                Keycode::Right | Keycode::C => {
                    if let Some(call) = &mut self.call {
                        call.volume = (call.volume + 5).min(100);
                        self.prefs.spk_gain = call.volume;
                        self.prefs.save();
                        let v = call.volume;
                        self.dispatch_bg(Job::SetVolume(v));
                    }
                }
                _ => {}
            }
            return AppAction::None;
        }
        match arrow_alias(key) {
            Keycode::Escape => {
                if !self.dial.is_empty() {
                    self.dial.clear();
                } else if self.phone_focus == PhoneFocus::History {
                    self.phone_focus = PhoneFocus::Dial;
                } else {
                    // Content top level: climb back to the tab level. (Esc
                    // never leaves the app itself — that is the host's
                    // long-press Esc / menu Quit.)
                    self.entered = false;
                }
                AppAction::None
            }
            Keycode::Backspace if self.phone_focus == PhoneFocus::History => {
                if let Some(e) = self.history.get(self.history_sel) {
                    let id = e.id;
                    self.dispatch(Job::DeleteCall(id));
                }
                AppAction::None
            }
            k if is_enter(k) => {
                if self.phone_focus == PhoneFocus::History {
                    if let Some(e) = self.history.get(self.history_sel) {
                        let n = e.number.clone();
                        self.dial = LineEdit::new(&n);
                        self.start_call(&n);
                    }
                } else if !self.dial.is_empty() {
                    let n = self.dial.text.clone();
                    self.start_call(&n);
                }
                AppAction::None
            }
            Keycode::Up => {
                if self.phone_focus == PhoneFocus::History {
                    if self.history_sel > 0 {
                        self.history_sel -= 1;
                    } else {
                        self.phone_focus = PhoneFocus::Dial;
                    }
                }
                AppAction::None
            }
            Keycode::Down => {
                if self.phone_focus == PhoneFocus::Dial {
                    if !self.history.is_empty() {
                        self.phone_focus = PhoneFocus::History;
                        self.history_sel = 0;
                    }
                } else if self.history_sel + 1 < self.history.len() {
                    self.history_sel += 1;
                }
                AppAction::None
            }
            Keycode::Left | Keycode::Right | Keycode::Backspace | Keycode::Delete
            | Keycode::Home | Keycode::End => {
                self.dial.key(key, mods);
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn key_message(&mut self, key: Keycode, mods: Mod) -> AppAction {
        match self.msg_view {
            MsgView::List => match arrow_alias(key) {
                Keycode::Escape => {
                    self.entered = false;
                    AppAction::None
                }
                Keycode::Up => {
                    self.thread_sel = self.thread_sel.saturating_sub(1);
                    AppAction::None
                }
                Keycode::Down => {
                    let n = threads(&self.messages, &self.read).len();
                    if self.thread_sel + 1 < n {
                        self.thread_sel += 1;
                    }
                    AppAction::None
                }
                k if is_enter(k) => {
                    let ts = threads(&self.messages, &self.read);
                    if let Some(t) = ts.get(self.thread_sel) {
                        let addr = t.address.clone();
                        self.thread_addr = addr.clone();
                        self.mark_thread_read(&addr);
                        self.thread_input = LineEdit::default();
                        self.thread_back = 0;
                        self.msg_view = MsgView::Thread;
                    }
                    AppAction::None
                }
                Keycode::Backspace => {
                    let ts = threads(&self.messages, &self.read);
                    if let Some(t) = ts.get(self.thread_sel) {
                        let addr = t.address.clone();
                        self.dispatch(Job::DeleteThread(addr));
                    }
                    AppAction::None
                }
                Keycode::N => {
                    self.new_to = LineEdit::default();
                    self.new_body = LineEdit::default();
                    self.new_focus_body = false;
                    self.msg_view = MsgView::New;
                    // The same keypress also arrives via text(); keep that
                    // trailing "n" out of the To field.
                    self.swallow_text = Some(Instant::now());
                    AppAction::None
                }
                _ => AppAction::None,
            },
            MsgView::Thread => match key {
                Keycode::Escape => {
                    self.msg_view = MsgView::List;
                    AppAction::None
                }
                k if is_enter(k) => {
                    let text = self.thread_input.text.clone();
                    let addr = self.thread_addr.clone();
                    let a = self.send_sms(&addr, &text);
                    if a == AppAction::None {
                        self.thread_input = LineEdit::default();
                        self.thread_back = 0;
                    }
                    a
                }
                Keycode::Up => {
                    self.thread_back += 26;
                    AppAction::None
                }
                Keycode::Down => {
                    self.thread_back = (self.thread_back - 26).max(0);
                    AppAction::None
                }
                _ => {
                    self.thread_input.key(key, mods);
                    AppAction::None
                }
            },
            MsgView::New => match key {
                Keycode::Escape => {
                    if self.new_focus_body {
                        self.new_focus_body = false;
                    } else {
                        self.msg_view = MsgView::List;
                    }
                    AppAction::None
                }
                k if is_enter(k) => {
                    if !self.new_focus_body {
                        if !self.new_to.text.trim().is_empty() {
                            self.new_focus_body = true;
                        }
                        AppAction::None
                    } else {
                        let to = self.new_to.text.clone();
                        let body = self.new_body.text.clone();
                        let a = self.send_sms(&to, &body);
                        if a == AppAction::None {
                            self.thread_addr = to.trim().to_string();
                            self.thread_input = LineEdit::default();
                            self.thread_back = 0;
                            self.msg_view = MsgView::Thread;
                        }
                        a
                    }
                }
                _ => {
                    if self.new_focus_body {
                        self.new_body.key(key, mods);
                    } else {
                        self.new_to.key(key, mods);
                    }
                    AppAction::None
                }
            },
        }
    }

    fn key_settings(&mut self, key: Keycode) -> AppAction {
        if let Some(item) = self.set_editing {
            match arrow_alias(key) {
                Keycode::Left => match item {
                    SetItem::SpkGain => {
                        self.prefs.spk_gain = self.prefs.spk_gain.saturating_sub(5)
                    }
                    SetItem::MicGain => {
                        self.prefs.mic_gain = self.prefs.mic_gain.saturating_sub(5)
                    }
                    _ => {}
                },
                Keycode::Right => match item {
                    SetItem::SpkGain => {
                        self.prefs.spk_gain = (self.prefs.spk_gain + 5).min(100)
                    }
                    SetItem::MicGain => {
                        self.prefs.mic_gain = (self.prefs.mic_gain + 5).min(100)
                    }
                    _ => {}
                },
                k if is_enter(k) || k == Keycode::Escape => {
                    self.prefs.save();
                    self.set_editing = None;
                }
                _ => {}
            }
            return AppAction::None;
        }
        match arrow_alias(key) {
            Keycode::Escape => {
                self.entered = false;
                AppAction::None
            }
            Keycode::Up => {
                self.set_sel = self.set_sel.saturating_sub(1);
                AppAction::None
            }
            Keycode::Down => {
                if self.set_sel + 1 < SET_ITEMS.len() {
                    self.set_sel += 1;
                }
                AppAction::None
            }
            k if is_enter(k) => {
                match SET_ITEMS[self.set_sel] {
                    SetItem::Data => {
                        if let Some(st) = &self.status {
                            if st.present {
                                let on = !st.data_on;
                                self.set_note(&if on {
                                    tr_s("Turning data on...")
                                } else {
                                    tr_s("Turning data off...")
                                });
                                self.dispatch(Job::SetData(on));
                            }
                        }
                    }
                    SetItem::Route => {
                        self.prefs.route_4g_first = !self.prefs.route_4g_first;
                        self.prefs.save();
                    }
                    SetItem::Voice => {
                        if self.voice.as_ref().map(|v| v.drivers_loaded).unwrap_or(false) {
                            self.set_note(&tr_s("Voice runtime loaded"));
                        } else {
                            self.set_note(&tr_s("Loading voice runtime..."));
                            self.dispatch(Job::LoadVoice);
                        }
                    }
                    SetItem::SpkGain | SetItem::MicGain => {
                        self.set_editing = Some(SET_ITEMS[self.set_sel]);
                    }
                    SetItem::Reboot => {
                        self.confirm_reboot = true;
                    }
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    /// Typed characters, routed by context. Digits in the dialer and DTMF
    /// in a call are filtered; text fields take everything.
    fn text_input(&mut self, text: &str) {
        if self.incoming.is_some() || self.confirm_reboot {
            return;
        }
        if !self.entered {
            return;
        }
        if let Some(t) = self.swallow_text.take() {
            if t.elapsed() < Duration::from_millis(500) && (text == "n" || text == "N") {
                return;
            }
        }
        match self.tab {
            Tab::Phone => {
                if self.call.is_some() {
                    let digits: String = text
                        .chars()
                        .filter(|c| c.is_ascii_digit() || matches!(c, '*' | '#'))
                        .collect();
                    if !digits.is_empty() {
                        let active = self
                            .call
                            .as_ref()
                            .map(|c| c.phase == cell::CallPhase::Active)
                            .unwrap_or(false);
                        if let Some(call) = &mut self.call {
                            call.dtmf.push_str(&digits);
                            if call.dtmf.len() > 24 {
                                call.dtmf.drain(..call.dtmf.len() - 24);
                            }
                        }
                        if active {
                            self.dispatch_bg(Job::Dtmf(digits));
                        }
                    }
                } else if self.phone_focus == PhoneFocus::Dial {
                    let filtered: String =
                        text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '+' | '*' | '#')).collect();
                    if !filtered.is_empty() {
                        self.dial.insert(&filtered);
                    }
                }
            }
            Tab::Message => match self.msg_view {
                MsgView::Thread => self.thread_input.insert(text),
                MsgView::New => {
                    if self.new_focus_body {
                        self.new_body.insert(text);
                    } else {
                        self.new_to.insert(text);
                    }
                }
                MsgView::List => {}
            },
            Tab::Settings => {}
        }
    }
}

// ---------------------------------------------------------------------------
// LauncherApp
// ---------------------------------------------------------------------------

impl LauncherApp for CellularApp {
    fn title(&self) -> String {
        tr_s("Cellular")
    }

    fn menu(&self) -> Vec<MenuItem> {
        vec![
            MenuItem::new("new", tr_s("New message")),
            MenuItem::new("refresh", tr_s("Refresh")),
        ]
    }

    fn menu_action(&mut self, id: &str, _ctx: &Ctx) -> AppAction {
        match id {
            "new" => {
                self.tab = Tab::Message;
                self.entered = true;
                self.new_to = LineEdit::default();
                self.new_body = LineEdit::default();
                self.new_focus_body = false;
                self.msg_view = MsgView::New;
                AppAction::None
            }
            "refresh" => {
                self.refresh();
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    fn key(&mut self, key: Keycode, mods: Mod, _repeat: bool, _ctx: &Ctx) -> AppAction {
        // The incoming-call modal captures everything; Tab is ignored there.
        if let Some(number) = self.incoming.clone() {
            if is_enter(key) {
                self.incoming = None;
                self.tab = Tab::Phone;
                self.entered = true;
                self.call = Some(CallUi {
                    number,
                    phase: cell::CallPhase::Incoming,
                    secs_base: 0,
                    secs_at: Instant::now(),
                    volume: self.prefs.spk_gain,
                    muted: false,
                    dtmf: String::new(),
                });
                self.dispatch(Job::Answer);
            } else if key == Keycode::Escape {
                self.incoming = None;
                self.dispatch(Job::Hangup);
            }
            return AppAction::None;
        }
        if self.confirm_reboot {
            if is_enter(key) {
                self.confirm_reboot = false;
                self.set_note(&tr_s("Rebooting modem..."));
                self.dispatch(Job::RebootModem);
            } else if key == Keycode::Escape {
                self.confirm_reboot = false;
            }
            return AppAction::None;
        }
        if key == Keycode::Tab {
            let shift = mods.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
            let dir = if shift { -1 } else { 1 };
            if !self.entered {
                // Tab level: Tab switches the top-level tab and lands on
                // the new tab's tab level, at its top view.
                self.tab = self.tab.step(dir);
                self.msg_view = MsgView::List;
                self.set_editing = None;
                return AppAction::None;
            }
            // Entered: Tab cycles the focusable content of this view —
            // it no longer switches Phone / Message / Settings (Esc back
            // to the tab level first for that).
            match self.tab {
                Tab::Phone => {
                    if self.call.is_none() && !self.history.is_empty() {
                        self.phone_focus = match self.phone_focus {
                            PhoneFocus::Dial => PhoneFocus::History,
                            PhoneFocus::History => PhoneFocus::Dial,
                        };
                        if self.phone_focus == PhoneFocus::History
                            && self.history_sel >= self.history.len()
                        {
                            self.history_sel = 0;
                        }
                    }
                }
                Tab::Message => match self.msg_view {
                    MsgView::List => {
                        let n = threads(&self.messages, &self.read).len();
                        if n > 0 {
                            self.thread_sel =
                                (self.thread_sel as i32 + dir).rem_euclid(n as i32) as usize;
                        }
                    }
                    MsgView::New => self.new_focus_body = !self.new_focus_body,
                    MsgView::Thread => {}
                },
                Tab::Settings => {
                    if self.set_editing.is_some() {
                        self.prefs.save();
                        self.set_editing = None;
                    }
                    let n = SET_ITEMS.len() as i32;
                    self.set_sel = (self.set_sel as i32 + dir).rem_euclid(n) as usize;
                }
            }
            return AppAction::None;
        }
        if !self.entered {
            // Tab level: Enter descends into the content; everything else
            // (arrows, letters, digits) does nothing here.
            if is_enter(key) {
                self.entered = true;
            }
            return AppAction::None;
        }
        match self.tab {
            Tab::Phone => self.key_phone(key, mods),
            Tab::Message => self.key_message(key, mods),
            Tab::Settings => self.key_settings(key),
        }
    }

    fn text(&mut self, text: &str, _ctx: &Ctx) -> AppAction {
        self.text_input(text);
        AppAction::None
    }

    fn tick(&mut self, _ctx: &Ctx) -> bool {
        let mut changed = self.drain();
        if self.note.is_some() && !self.busy && self.note_at.elapsed() >= Duration::from_secs(4) {
            self.note = None;
            changed = true;
        }
        if self.call.is_some() && self.last_tick.elapsed() >= Duration::from_millis(500) {
            self.last_tick = Instant::now();
            changed = true;
        }
        if self.last_call_poll.map(|t| t.elapsed() >= Duration::from_secs(1)).unwrap_or(true) {
            self.last_call_poll = Some(Instant::now());
            self.dispatch_bg(Job::CallPoll);
        }
        if !self.busy && self.last_refresh.map(|t| t.elapsed() >= REFRESH_EVERY).unwrap_or(true) {
            self.refresh();
            changed = true;
        }
        changed
    }

    fn describe(&self) -> String {
        match &self.status {
            Some(st) if st.present => {
                let op = st.operator.clone().unwrap_or_else(|| tr_s("Unknown carrier"));
                let ip = st.ip.clone().unwrap_or_else(|| tr_s("no IP"));
                format!("{op}, {}, {ip}", if st.data_on { tr_s("data on") } else { tr_s("data off") })
            }
            _ => tr_s("Cellular modem not available"),
        }
    }

    fn wants_tab(&self) -> bool {
        true
    }

    fn draw(&mut self, c: &mut Canvas<Window>, _assets: &mut AssetStore, ctx: &Ctx) {
        match self.tab {
            Tab::Message if self.msg_view == MsgView::Thread => self.draw_thread(c, ctx),
            Tab::Message if self.msg_view == MsgView::New => self.draw_new(c, ctx),
            _ => {
                self.draw_tabbar(c, ctx);
                match self.tab {
                    Tab::Phone => self.draw_phone(c, ctx),
                    Tab::Message => self.draw_message_list(c, ctx),
                    Tab::Settings => self.draw_settings(c, ctx),
                }
            }
        }
        if self.confirm_reboot {
            self.draw_confirm(c, ctx);
        }
        if self.incoming.is_some() {
            self.draw_incoming(c, ctx);
        }
        if let Some(note) = self.note.clone() {
            let inner = chrome::inner_rect(&chrome::app_rect());
            chrome::draw_command_bar(c, ctx.fs, ctx.tm, &inner, &note, "");
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing (Harbour design language, docs/DESIGN.md: the shared folder-tab
// strip, the theme's selection plate, sunken wells for fields, zebra bands
// for section headers, secondary ink for hints, no footer — transient
// messages ride the shell's command bar).

/// The tones this screen draws with, named after the design doc's roles.
struct Pal {
    ink: sdl2::pixels::Color,
    face: sdl2::pixels::Color,
    faint: sdl2::pixels::Color,
    sec: sdl2::pixels::Color,
    line: sdl2::pixels::Color,
    alt: sdl2::pixels::Color,
    sunken: sdl2::pixels::Color,
    accent: sdl2::pixels::Color,
    sel_bg: sdl2::pixels::Color,
    sel_fg: sdl2::pixels::Color,
}

fn pal(tm: &ThemeManager) -> Pal {
    let sel = widgets::selection(tm);
    Pal {
        ink: tm.get_color("foreground"),
        face: tm.get_color("widget_face_active"),
        faint: tm.get_color("text_faint"),
        sec: widgets::secondary_ink(tm),
        line: tm.get_color("line"),
        alt: tm.get_color("surface_alt"),
        sunken: tm.get_color("surface_sunken"),
        accent: tm.get_color("accent"),
        sel_bg: sel.bg,
        sel_fg: sel.fg,
    }
}

fn outline(c: &mut Canvas<Window>, x: i32, y: i32, w: i32, h: i32, col: sdl2::pixels::Color) {
    ui::fill_rect(c, Rect::new(x, y, w as u32, 1), col);
    ui::fill_rect(c, Rect::new(x, y + h - 1, w as u32, 1), col);
    ui::fill_rect(c, Rect::new(x, y, 1, h as u32), col);
    ui::fill_rect(c, Rect::new(x + w - 1, y, 1, h as u32), col);
}

/// A text field: a sunken well normally; the focused field itself takes
/// the theme's selection colours (fill select_bg, text select_fg).
fn draw_well(c: &mut Canvas<Window>, x: i32, y: i32, w: i32, h: i32, p: &Pal, focused: bool) {
    if focused {
        ui::fill_rect(c, Rect::new(x, y, w as u32, h as u32), p.sel_bg);
        outline(c, x, y, w, h, p.ink);
    } else {
        ui::fill_rect(c, Rect::new(x, y, w as u32, h as u32), p.sunken);
        outline(c, x, y, w, h, p.line);
    }
}

/// A zebra section band closed by a rule (DESIGN: header bars).
fn draw_band(c: &mut Canvas<Window>, fs: &FontSystem, x: i32, y: i32, w: i32, h: i32, text: &str, p: &Pal) {
    ui::fill_rect(c, Rect::new(x, y, w as u32, h as u32), p.alt);
    fs.draw_text(c, x + 4, y + (h - 12) / 2 + 1, text, p.ink);
    ui::fill_rect(c, Rect::new(x, y + h, w as u32, 1), p.line);
}

/// A dialog card: solid offset shadow, face fill, ink outline (DESIGN dialogs).
fn draw_card(c: &mut Canvas<Window>, x: i32, y: i32, w: i32, h: i32, p: &Pal) {
    ui::fill_rect(c, Rect::new(x + 3, y + 3, w as u32, h as u32), p.line);
    ui::fill_rect(c, Rect::new(x, y, w as u32, h as u32), p.face);
    outline(c, x, y, w, h, p.ink);
}

impl CellularApp {
    /// The shared folder-tab strip (widgets::Tabs, as Monitor / Infoscope /
    /// Interceptor draw it); the signal status rides on the strip, right of
    /// the tabs, like Infoscope's status line.
    fn draw_tabbar(&self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let p = pal(ctx.tm);
        let unread = self.unread();
        let msg_label;
        let mut labels: Vec<&str> = vec!["Phone"];
        if unread > 0 {
            msg_label = format!("Message ({unread})");
            labels.push(&msg_label);
        } else {
            labels.push("Message");
        }
        labels.push("Settings");
        let active = match self.tab {
            Tab::Phone => 0,
            Tab::Message => 1,
            Tab::Settings => 2,
        };
        let tabs = widgets::Tabs::folder_fit(ctx.fs, &labels, active, ctx.tm);
        tabs.draw_strip(c, ctx.fs, ctx.tm);
        if !self.entered {
            // Focus sits on the tab itself (Enter descends into the content):
            // underline the active tab's label.
            let r = tabs.tab_rect(active);
            let label = labels[active];
            let lw = if tabs.display {
                ctx.fs.measure_display_text(label)
            } else {
                ctx.fs.measure_text(label)
            };
            let cx = r.x() + (r.width() as i32 - lw) / 2;
            ui::fill_rect(c, Rect::new(cx, grid::BAR_H + widgets::FOLDER_TABS_H - 4, lw as u32, 2), p.ink);
        }
        // Status: network type + the shared signal-bars cell.
        let (tech, bars) = match &self.status {
            Some(st) if st.present && st.registered => {
                let t = st.tech.as_deref().unwrap_or("");
                (if t.contains("LTE") { "LTE" } else { t }, st.bars.unwrap_or(0))
            }
            _ => ("--", 0),
        };
        let tw = ctx.fs.measure_text(tech);
        let bars_x = RIGHT - 14;
        ctx.fs.draw_text(c, bars_x - 4 - tw, widgets::folder_strip_text_y(), tech, p.sec);
        chrome::draw_cell(c, bars_x, grid::BAR_H + 4, bars, p.ink, p.faint);
    }

    fn draw_phone(&self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        if let Some(call) = &self.call {
            self.draw_incall(c, ctx, &call.number, call.phase, call.secs(), call.volume, call.muted, &call.dtmf);
            return;
        }
        // Dialer (left column). The focused input box itself takes the
        // selection colours.
        let dial_focus = self.entered && self.phone_focus == PhoneFocus::Dial;
        fs.draw_text(c, LEFT, CONTENT_Y + 2, "DIAL NUMBER", p.sec);
        draw_well(c, LEFT, CONTENT_Y + 14, 140, 18, &p, dial_focus);
        if dial_focus {
            draw_input_col(c, fs, ctx.tm, &self.dial, LEFT + 5, CONTENT_Y + 17, 130, p.sel_fg);
        } else {
            draw_input(c, fs, ctx.tm, &self.dial, LEFT + 5, CONTENT_Y + 17, 130);
        }
        fs.draw_text(c, LEFT, CONTENT_Y + 38, "Type digits / + / * / #", p.sec);
        fs.draw_text(c, LEFT, CONTENT_Y + 52, "[Enter] Call", p.sec);
        match &self.status {
            None => fs.draw_text(c, LEFT, CONTENT_Y + 78, "Loading...", p.faint),
            Some(st) if !st.present => {
                fs.draw_text(c, LEFT, CONTENT_Y + 78, "Modem not available", p.faint)
            }
            Some(st) => {
                let op = match &st.operator {
                    Some(o) => format!("{o} {}", st.tech.as_deref().unwrap_or("")),
                    None => "Not registered".to_string(),
                };
                fs.draw_text(c, LEFT, CONTENT_Y + 78, &chrome::fit_text(fs, &op, 140), p.sec);
                let sig = match (st.band.as_deref(), st.rsrp) {
                    (Some(b), Some(r)) => format!("{b}  {r} dBm"),
                    (Some(b), None) => b.to_string(),
                    _ => String::new(),
                };
                fs.draw_text(c, LEFT, CONTENT_Y + 92, &chrome::fit_text(fs, &sig, 140), p.sec);
            }
        }
        // Divider + recent calls (right column).
        ui::fill_rect(c, Rect::new(152, CONTENT_Y, 1, (CONTENT_H - 4) as u32), p.line);
        draw_band(c, fs, 158, CONTENT_Y, RIGHT - 158, 15, "RECENT CALLS", &p);
        if self.history.is_empty() {
            fs.draw_text(c, 158, CONTENT_Y + 24, "No recent calls", p.faint);
        }
        for (i, e) in self.history.iter().take(6).enumerate() {
            let y = CONTENT_Y + 22 + i as i32 * 15;
            let sel = self.entered && self.phone_focus == PhoneFocus::History && self.history_sel == i;
            if sel {
                ui::fill_rect(c, Rect::new(157, y, (RIGHT - 157) as u32, 14), p.sel_bg);
            }
            let col = if sel { p.sel_fg } else { p.ink };
            let arrow = match e.kind {
                CallKind::Out => "↗",
                CallKind::In => "↙",
                CallKind::Missed => "✕",
            };
            let line = format!("{arrow} {}", e.number);
            fs.draw_text(c, 160, y + 1, &chrome::fit_text(fs, &line, 105), col);
            let when = fmt_ts(e.ts, false);
            let tw = fs.measure_text(&when);
            fs.draw_text(c, RIGHT - tw, y + 1, &when, if sel { p.sel_fg } else { p.sec });
        }
        if self.entered && self.phone_focus == PhoneFocus::History {
            fs.draw_text(c, 158, CONTENT_Y + 22 + 6 * 15 + 4, "[Up/Dn] Select  [Enter] Dial  [Bksp] Del", p.sec);
        }
    }

    fn draw_incall(
        &self,
        c: &mut Canvas<Window>,
        ctx: &Ctx,
        number: &str,
        phase: cell::CallPhase,
        secs: u64,
        volume: u8,
        muted: bool,
        dtmf: &str,
    ) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        let y = CONTENT_Y;
        ui::fill_rect(c, Rect::new(LEFT + 1, y + 3, 6, 6), p.accent);
        let title = match phase {
            cell::CallPhase::Active => format!("IN CALL  {:02}:{:02}", secs / 60, secs % 60),
            cell::CallPhase::Dialing => "DIALING".to_string(),
            cell::CallPhase::Alerting => "RINGING".to_string(),
            cell::CallPhase::Incoming => "INCOMING CALL".to_string(),
            cell::CallPhase::Idle => "CALL".to_string(),
        };
        fs.draw_display_text(c, LEFT + 12, y, &title, p.ink);
        let num = chrome::fit_text(fs, number, 300);
        fs.draw_display_text(c, LEFT, y + 20, &num, p.ink);
        let sub = match phase {
            cell::CallPhase::Active => "VoLTE voice connected",
            cell::CallPhase::Dialing => "Dialing via VoLTE...",
            cell::CallPhase::Alerting => "Waiting for answer...",
            _ => "",
        };
        fs.draw_text(c, LEFT, y + 40, sub, p.sec);
        // Speaker volume: a zebra track with the filled part in ink.
        fs.draw_text(c, LEFT, y + 62, "SPK VOL", p.ink);
        ui::fill_rect(c, Rect::new(LEFT + 56, y + 62, 120, 10), p.alt);
        ui::fill_rect(c, Rect::new(LEFT + 56, y + 62, (volume as i32 * 120 / 100) as u32, 10), p.ink);
        outline(c, LEFT + 56, y + 62, 120, 10, p.line);
        fs.draw_text(c, LEFT + 184, y + 62, &format!("{volume}%"), p.ink);
        fs.draw_text(c, LEFT + 232, y + 62, "(Left/Right)", p.sec);
        let mic = if muted { "MIC      [Muted]" } else { "MIC      [Active]" };
        fs.draw_text(c, LEFT, y + 84, mic, p.ink);
        let dt = if dtmf.is_empty() { "DTMF: [Ready 0-9/*/#]".to_string() } else { format!("DTMF: {dtmf}") };
        fs.draw_text(c, LEFT + 130, y + 84, &dt, p.ink);
        let hints = if matches!(phase, cell::CallPhase::Incoming) {
            "[Enter] Answer   [Esc] Decline"
        } else {
            "[Esc] Hang Up   [M] Mute   [0-9] DTMF"
        };
        fs.draw_text(c, LEFT, y + 106, hints, p.sec);
    }

    fn draw_message_list(&mut self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        let list = threads(&self.messages, &self.read);
        if list.is_empty() {
            fs.draw_text(c, LEFT, CONTENT_Y + 4, "No messages yet.", p.faint);
            fs.draw_text(c, LEFT, CONTENT_Y + 20, "When SMS arrives it lands here.", p.sec);
            fs.draw_text(c, LEFT, CONTENT_Y + 44, "[N] New message", p.sec);
            return;
        }
        // Two 12 px text lines (origins y+2 / y+14, cells ending at y+25)
        // plus the rule row: the plate must span the full text block.
        const ROW_H: i32 = 27;
        let vis = ((CONTENT_H - 14) / ROW_H).max(1) as usize;
        self.thread_top = widgets::follow_scroll(self.thread_sel, self.thread_top, vis, list.len());
        let top = self.thread_top;
        for (vi, t) in list.iter().skip(top).take(vis).enumerate() {
            let idx = top + vi;
            let y = CONTENT_Y + vi as i32 * ROW_H;
            let sel = self.entered && idx == self.thread_sel;
            if sel {
                // House plate convention (widgets::draw_row_plate): the row
                // minus its last pixel row, which belongs to the rule.
                ui::fill_rect(c, Rect::new(LEFT, y, (RIGHT - LEFT) as u32, (ROW_H - 1) as u32), p.sel_bg);
            }
            let (main, sub) = if sel { (p.sel_fg, p.sel_fg) } else { (p.ink, p.sec) };
            fs.draw_display_text(c, LEFT + 4, y + 2, &chrome::fit_text(fs, &t.address, 220), main);
            fs.draw_text(c, LEFT + 4, y + 14, &chrome::fit_text(fs, &t.preview, 270), sub);
            let when = fmt_ts(t.last_ts, false);
            let tw = fs.measure_text(&when);
            fs.draw_text(c, RIGHT - 6 - tw, y + 3, &when, sub);
            if t.unread > 0 {
                ui::fill_rect(c, Rect::new(RIGHT - 10, y + 14, 6, 6), p.accent);
                let n = format!("{}", t.unread);
                let nw = fs.measure_text(&n);
                fs.draw_text(c, RIGHT - 12 - nw, y + 12, &n, if sel { p.sel_fg } else { p.accent });
            }
            if vi + 1 < vis && idx + 1 < list.len() {
                widgets::draw_row_rule(c, ctx.tm, Rect::new(LEFT, y, (RIGHT - LEFT) as u32, ROW_H as u32));
            }
        }
        if list.len() > vis {
            widgets::ScrollBar::new(Rect::new(RIGHT + 2, CONTENT_Y, 4, (CONTENT_H - 14) as u32), list.len(), vis, top)
                .draw(c, ctx.tm, None);
        }
        if self.entered {
            fs.draw_text(c, LEFT, CONTENT_Y + CONTENT_H - 12, "[Enter] Open  [N] New  [Bksp] Delete thread", p.sec);
        }
    }

    /// Header for the full-screen sub-views (Thread / New message): a zebra
    /// band closed by a rule, like a list header (DESIGN).
    fn draw_sub_header(&self, c: &mut Canvas<Window>, ctx: &Ctx, title: &str, right: &str) {
        let p = pal(ctx.tm);
        ui::fill_rect(
            c,
            Rect::new(0, grid::BAR_H, grid::SCREEN_W as u32, (CONTENT_Y - grid::BAR_H) as u32),
            ctx.tm.get_color("home_bg"),
        );
        ui::fill_rect(c, Rect::new(LEFT - 1, CONTENT_Y - 16, (RIGHT - LEFT + 2) as u32, 15), p.alt);
        ctx.fs.draw_display_text(c, LEFT + 3, CONTENT_Y - 15, title, p.ink);
        if !right.is_empty() {
            let tw = ctx.fs.measure_text(right);
            ctx.fs.draw_text(c, RIGHT - 3 - tw, CONTENT_Y - 14, right, p.sec);
        }
        ui::fill_rect(c, Rect::new(LEFT - 1, CONTENT_Y - 1, (RIGHT - LEFT + 2) as u32, 1), p.line);
    }

    fn draw_thread(&mut self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        let msgs = thread_messages(&self.messages, &self.thread_addr);
        let (n, _cap, segs) = sms_segments(&self.thread_input.text);
        let seg = if n > 0 { format!("{n} chars, {segs} seg") } else { String::new() };
        let title = format!("< {}", self.thread_addr);
        self.draw_sub_header(c, ctx, &chrome::fit_text(fs, &title, 200), &seg);
        const VIEW_Y: i32 = CONTENT_Y + 2;
        const VIEW_H: i32 = 100;
        // Measure bottom-up so the newest bubble sits at the view's bottom;
        // thread_back scrolls the whole stack up.
        let mut blocks: Vec<(bool, Vec<String>)> = Vec::new();
        for m in &msgs {
            blocks.push((m.direction == "out", wrap_text(fs, &m.body, 250)));
        }
        let mut total = 0;
        for (_, lines) in &blocks {
            total += lines.len() as i32 * 12 + 8 + 4;
        }
        self.thread_back = self.thread_back.min((total - VIEW_H).max(0));
        let mut by = VIEW_Y + VIEW_H - total + self.thread_back;
        // Bubbles are laid out bottom-up and the stack can start far above
        // the view; clip to the view so a tall bubble never paints over
        // the sub-header or the shell status bar (cf. Infoscope's list).
        c.set_clip_rect(Rect::new(0, VIEW_Y, grid::SCREEN_W as u32, VIEW_H as u32));
        for (outgoing, lines) in &blocks {
            let bh = lines.len() as i32 * 12 + 8;
            if by + bh >= VIEW_Y - 4 && by <= VIEW_Y + VIEW_H {
                let bw = lines.iter().map(|l| fs.measure_text(l)).max().unwrap_or(0) + 12;
                let (bx, fill) = if *outgoing { (RIGHT - bw, p.alt) } else { (LEFT, p.face) };
                ui::fill_rect(c, Rect::new(bx, by, bw as u32, bh as u32), fill);
                outline(c, bx, by, bw, bh, p.ink);
                for (i, l) in lines.iter().enumerate() {
                    fs.draw_text(c, bx + 6, by + 4 + i as i32 * 12, l, p.ink);
                }
            }
            by += bh + 4;
        }
        c.set_clip_rect(None);
        // Composer: a rule, then a sunken well with the input.
        let comp_y = VIEW_Y + VIEW_H + 4;
        ui::fill_rect(c, Rect::new(LEFT, comp_y, (RIGHT - LEFT) as u32, 1), p.line);
        draw_well(c, LEFT + 10, comp_y + 4, RIGHT - LEFT - 10, 16, &p, true);
        fs.draw_text(c, LEFT, comp_y + 6, ">", p.ink);
        draw_input_col(c, fs, ctx.tm, &self.thread_input, LEFT + 14, comp_y + 6, RIGHT - LEFT - 18, p.sel_fg);
    }

    fn draw_new(&self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        self.draw_sub_header(c, ctx, "New message", "");
        let y = CONTENT_Y + 4;
        fs.draw_text(c, LEFT, y + 2, "To:", p.ink);
        draw_well(c, 44, y, 200, 16, &p, !self.new_focus_body);
        if self.new_focus_body {
            draw_input(c, fs, ctx.tm, &self.new_to, 48, y + 2, 192);
        } else {
            draw_input_col(c, fs, ctx.tm, &self.new_to, 48, y + 2, 192, p.sel_fg);
        }
        fs.draw_text(c, LEFT, y + 26, "Message:", p.ink);
        draw_well(c, LEFT, y + 42, RIGHT - LEFT, 44, &p, self.new_focus_body);
        if self.new_focus_body {
            draw_input_col(c, fs, ctx.tm, &self.new_body, LEFT + 4, y + 46, RIGHT - LEFT - 8, p.sel_fg);
        } else {
            draw_input(c, fs, ctx.tm, &self.new_body, LEFT + 4, y + 46, RIGHT - LEFT - 8);
        }
        let (n, _cap, segs) = sms_segments(&self.new_body.text);
        let hint = format!("{n} chars, {segs} seg   [Tab] Field  [Enter] Next / Send");
        fs.draw_text(c, LEFT, y + 94, &hint, p.sec);
    }

    fn draw_settings(&self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        enum SRow {
            Section(&'static str),
            Info(String, String),
            Item(SetItem, String, String),
        }
        let st = self.status.as_ref();
        let dash = "—".to_string();
        let data_val = match st {
            Some(s) if s.present => if s.data_on { "On".into() } else { "Off".into() },
            _ => dash.clone(),
        };
        let rows: Vec<SRow> = vec![
            SRow::Section("NETWORK"),
            SRow::Item(SetItem::Data, "Mobile data".into(), data_val),
            SRow::Item(
                SetItem::Route,
                "Route priority".into(),
                if self.prefs.route_4g_first { "4G first".into() } else { "Wi-Fi first".into() },
            ),
            SRow::Info(
                "Throughput".into(),
                format!("Dn {}  Up {}", fmt_rate(self.rx_rate), fmt_rate(self.tx_rate)),
            ),
            SRow::Info("Usage (session)".into(), fmt_bytes(self.total_bytes)),
            SRow::Info("IPv4".into(), st.and_then(|s| s.ip.clone()).unwrap_or_else(|| dash.clone())),
            SRow::Info("IPv6".into(), st.and_then(|s| s.ipv6.clone()).unwrap_or_else(|| dash.clone())),
            SRow::Section("VOICE"),
            SRow::Item(SetItem::Voice, "Voice runtime".into(), match &self.voice {
                Some(v) if v.drivers_loaded => "Loaded".into(),
                Some(_) => "Not loaded".into(),
                None => "...".into(),
            }),
            SRow::Item(SetItem::SpkGain, "Speaker gain".into(), format!("{}%", self.prefs.spk_gain)),
            SRow::Item(SetItem::MicGain, "Mic gain".into(), format!("{}%", self.prefs.mic_gain)),
            SRow::Section("MODEM"),
            SRow::Item(SetItem::Reboot, "Reboot modem".into(), String::new()),
        ];
        const SEC_H: i32 = 15;
        const ROW_H: i32 = 13;
        let height = |r: &SRow| if matches!(r, SRow::Section(_)) { SEC_H } else { ROW_H };
        let total: i32 = rows.iter().map(|r| height(r)).sum();
        let sel_item = SET_ITEMS[self.set_sel];
        let mut sel_y = 0;
        {
            let mut yy = 0;
            for r in &rows {
                if matches!(r, SRow::Item(it, _, _) if *it == sel_item) {
                    sel_y = yy;
                }
                yy += height(r);
            }
        }
        let scroll = (sel_y - 26).max(0).min((total - CONTENT_H).max(0));
        let mut y = CONTENT_Y - scroll;
        for r in &rows {
            let h = height(r);
            if y + h >= CONTENT_Y && y <= BOTTOM {
                match r {
                    SRow::Section(name) => {
                        draw_band(c, fs, LEFT - 1, y, RIGHT - LEFT + 2, SEC_H - 1, name, &p);
                    }
                    SRow::Info(label, value) => {
                        fs.draw_text(c, LEFT + 4, y + 1, label, p.sec);
                        let v = chrome::fit_text(fs, value, 190);
                        let tw = fs.measure_text(&v);
                        fs.draw_text(c, RIGHT - 4 - tw, y + 1, &v, p.ink);
                    }
                    SRow::Item(item, label, value) => {
                        let sel = self.entered && *item == sel_item;
                        if sel {
                            ui::fill_rect(c, Rect::new(LEFT, y, (RIGHT - LEFT) as u32, (ROW_H - 1) as u32), p.sel_bg);
                        }
                        let (main, sub) = if sel { (p.sel_fg, p.sel_fg) } else { (p.ink, p.sec) };
                        fs.draw_text(c, LEFT + 4, y + 1, label, main);
                        let val = if self.set_editing == Some(*item) {
                            format!("< {value} >")
                        } else {
                            value.clone()
                        };
                        let v = chrome::fit_text(fs, &val, 190);
                        let tw = fs.measure_text(&v);
                        fs.draw_text(c, RIGHT - 4 - tw, y + 1, &v, sub);
                    }
                }
            }
            y += h;
        }
        if total > CONTENT_H {
            widgets::ScrollBar::new(
                Rect::new(RIGHT + 2, CONTENT_Y, 4, CONTENT_H as u32),
                (total / ROW_H) as usize,
                (CONTENT_H / ROW_H) as usize,
                (scroll / ROW_H) as usize,
            )
            .draw(c, ctx.tm, None);
        }
    }

    fn draw_confirm(&self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        let (w, h) = (220, 64);
        let (x, y) = ((grid::SCREEN_W - w) / 2, 56);
        draw_card(c, x, y, w, h, &p);
        fs.draw_display_text(c, x + 12, y + 8, "Reboot modem?", p.ink);
        fs.draw_text(c, x + 12, y + 28, "Data drops for ~30 seconds.", p.sec);
        fs.draw_text(c, x + 12, y + 46, "[Enter] Reboot    [Esc] Cancel", p.ink);
    }

    fn draw_incoming(&self, c: &mut Canvas<Window>, ctx: &Ctx) {
        let fs = ctx.fs;
        let p = pal(ctx.tm);
        let Some(num) = &self.incoming else { return };
        let (w, h) = (232, 84);
        let (x, y) = ((grid::SCREEN_W - w) / 2, 46);
        draw_card(c, x, y, w, h, &p);
        let title = "INCOMING CALL";
        let tw = fs.measure_display_text(title);
        fs.draw_display_text(c, x + (w - tw) / 2, y + 8, title, p.ink);
        let n = chrome::fit_text(fs, num, 200);
        let nw = fs.measure_display_text(&n);
        fs.draw_display_text(c, x + (w - nw) / 2, y + 28, &n, p.ink);
        let sub = "VoLTE HD Voice";
        let sw = fs.measure_text(sub);
        fs.draw_text(c, x + (w - sw) / 2, y + 48, sub, p.sec);
        let keys = "[Enter] Answer     [Esc] Decline";
        let kw = fs.measure_text(keys);
        fs.draw_text(c, x + (w - kw) / 2, y + 64, keys, p.ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSystem;

    fn msg(id: i64, direction: &str, address: &str, body: &str, ts: i64) -> SmsMessage {
        SmsMessage {
            id,
            direction: direction.into(),
            address: address.into(),
            body: body.into(),
            ts,
            status: "received".into(),
        }
    }

    #[test]
    fn threads_group_order_and_count_unread() {
        let msgs = vec![
            msg(1, "in", "10010", "balance notice", 100),
            msg(2, "out", "10010", "102", 200),
            msg(3, "in", "10010", "data used", 300),
            msg(4, "in", "10690123", "code 492810", 250),
        ];
        let read: HashSet<i64> = [1].into_iter().collect();
        let ts = threads(&msgs, &read);
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].address, "10010");
        assert_eq!(ts[0].preview, "data used");
        assert_eq!(ts[0].unread, 1);
        assert_eq!(ts[1].unread, 1);
        let in_thread = thread_messages(&msgs, "10010");
        assert_eq!(in_thread.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn segments_gsm_and_ucs2_boundaries() {
        assert_eq!(sms_segments(""), (0, 160, 0));
        assert_eq!(sms_segments("hello"), (5, 160, 1));
        assert_eq!(sms_segments(&"a".repeat(161)), (161, 153, 2));
        assert_eq!(sms_segments("流量"), (2, 70, 1));
        assert_eq!(sms_segments(&"流".repeat(71)), (71, 67, 2));
    }

    #[test]
    fn tab_steps_cycle() {
        assert_eq!(Tab::Phone.step(1), Tab::Message);
        assert_eq!(Tab::Message.step(1), Tab::Settings);
        assert_eq!(Tab::Settings.step(1), Tab::Phone);
        assert_eq!(Tab::Phone.step(-1), Tab::Settings);
    }

    #[test]
    fn prefs_defaults_fill_gains() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.spk_gain, 70);
        assert_eq!(p.mic_gain, 100);
        assert!(!p.route_4g_first);
        assert_eq!(Prefs::default().spk_gain, 70);
    }

    #[test]
    fn wrap_keeps_newlines_and_breaks_long_lines() {
        let fs = FontSystem::new();
        assert_eq!(
            wrap_text(&fs, "第一行\r\n第二行", 200),
            vec!["第一行".to_string(), "第二行".to_string()]
        );
        let body = "这是一条比较长的短信内容，用来验证按像素宽度折行的行为是否正确无误。";
        let lines = wrap_text(&fs, body, 120);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), body);
    }

    #[test]
    fn timestamps_have_the_expected_shapes() {
        assert_eq!(fmt_ts(1_795_109_715, true).len(), 16);
        assert_eq!(fmt_ts(i64::MAX, false), "");
        assert_eq!(fmt_clock(1_795_109_715).len(), 5);
    }
}
