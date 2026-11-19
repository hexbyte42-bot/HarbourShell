//! Push-to-talk voice to Muse: record from the microphone while a key is held, hand the WAV to the
//! Muse SDK (`musegadget voice-call --in-wav ... --out-wav ...`), play the reply WAV.
//!
//! Everything that touches hardware or spawns a process sits behind [`VoiceBackend`]; the
//! [`VoiceController`] state machine and all the tests use [`FakeVoice`].
//!
//! Facts read from the SDK on the device (read-only, 2026-11-18): the input must be PCM16LE mono
//! 24 kHz; the output WAV is PCM16LE mono 24 kHz; the CLI prints only `Voice call ended: <reason>.`
//! (no transcript); `--vm-id` must match a leased Muse VM exactly.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::launcher::apps::recorder::{peak_rms, scan_devices, wav_header, Device};

pub const VOICE_RATE: u32 = 24_000;
/// Shorter clips are dropped ("too short"): a bump of the key is not a message.
pub const MIN_SECS: f32 = 0.4;
/// A held key records at most this long.
pub const MAX_SECS: u64 = 30;
/// `voice-call` limits (seconds). Whether these fit Muse's behaviour is unverified.
pub const IDLE_TIMEOUT_S: u32 = 20;
pub const OVERALL_TIMEOUT_S: u32 = 120;

pub type VR<T> = Result<T, String>;

pub struct Clip {
    pub path: PathBuf,
    pub seconds: f32,
}

/// A recording in progress.
pub trait Recording: Send {
    /// Recent input level 0.0 ..= 1.0.
    fn level(&self) -> f32;
    /// Stop and return the clip (the file is a complete WAV).
    fn finish(self: Box<Self>) -> VR<Clip>;
    /// Stop and throw the audio away.
    fn cancel(self: Box<Self>);
}

pub trait VoiceBackend: Send + Sync {
    fn start_recording(&self, dir: &Path) -> VR<Box<dyn Recording>>;
    /// Blocking: run the Muse call with `clip` as input and write the spoken reply to `out`.
    /// Returns the end reason the SDK printed.
    fn call(&self, clip: &Path, out: &Path) -> VR<String>;
    /// Blocking: play a WAV.
    fn play(&self, wav: &Path) -> VR<()>;
}

// ------------------------------------------------------------------ the SDK command line

/// The exact argument list of one voice call (no shell, no secrets: the SDK keeps its own token).
pub fn call_args(vm_id: &str, input: &Path, output: &Path) -> Vec<String> {
    vec![
        "voice-call".into(),
        "--vm-id".into(),
        vm_id.into(),
        "--in-wav".into(),
        input.to_string_lossy().into_owned(),
        "--out-wav".into(),
        output.to_string_lossy().into_owned(),
        "--idle-timeout".into(),
        IDLE_TIMEOUT_S.to_string(),
        "--overall-timeout".into(),
        OVERALL_TIMEOUT_S.to_string(),
    ]
}

/// Plain words for a failed `voice-call` (the SDK keeps its messages short and free of secrets).
pub fn explain_call_failure(code: Option<i32>, stdout: &str, stderr: &str) -> String {
    let all = format!("{stderr}\n{stdout}");
    let has = |s: &str| all.contains(s);
    if has("Not paired") {
        "Muse is not paired: run `musegadget pair` first".into()
    } else if has("must exactly match one leased VM") {
        "muse.vm_id does not match a Muse VM (see docs/MUSE.md)".into()
    } else if has("VM lookup failed") {
        "Muse VM lookup failed (network or login)".into()
    } else if let Some(i) = all.find("Voice call failed (") {
        let rest = &all[i..];
        format!("Muse voice call failed ({})", rest.split('(').nth(1).and_then(|r| r.split(')').next()).unwrap_or("error"))
    } else if has("WAV mode requires") || has("input WAV must be") {
        "the recording was not in the format the SDK wants".into()
    } else if let Some(line) = all.lines().find(|l| !l.trim().is_empty()) {
        format!("voice-call failed (exit {}): {}", code.map_or("?".to_string(), |c| c.to_string()), line.trim().chars().take(60).collect::<String>())
    } else {
        format!("voice-call failed (exit {})", code.map_or("?".to_string(), |c| c.to_string()))
    }
}

/// `Voice call ended: <reason>.` -> `<reason>`.
pub fn parse_end_reason(stdout: &str) -> Option<String> {
    stdout.lines().rev().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("Voice call ended: ").map(|r| r.trim_end_matches('.').to_string())
    })
}

// ------------------------------------------------------------------ the real backend

pub struct SystemVoice {
    pub sdk_command: String,
    pub vm_id: String,
}

struct ArecordRec {
    child: std::process::Child,
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    join: Option<std::thread::JoinHandle<(std::fs::File, u64)>>,
    path: PathBuf,
    err: Arc<Mutex<String>>,
}

fn pick_mic(devices: &[Device]) -> Option<&Device> {
    devices.iter().find(|d| d.usb).or_else(|| devices.first())
}

impl VoiceBackend for SystemVoice {
    fn start_recording(&self, dir: &Path) -> VR<Box<dyn Recording>> {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot save the recording: {e}"))?;
        let scan = scan_devices(false);
        let dev = pick_mic(&scan.devices).ok_or_else(|| "No microphone found".to_string())?.clone();
        let path = dir.join(format!("in-{}.wav", std::process::id()));
        let mut file = std::fs::File::create(&path).map_err(|e| format!("cannot save the recording: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        file.write_all(&wav_header(VOICE_RATE, 1, 0)).map_err(|e| format!("cannot save the recording: {e}"))?;
        let mut child = Command::new("arecord")
            .args(["-q", "-D", &dev.plug(), "-f", "S16_LE", "-r", &VOICE_RATE.to_string(), "-c", "1", "-t", "raw"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "No microphone tool: arecord is missing (apt install alsa-utils)".to_string() } else { format!("cannot start arecord: {e}") })?;
        let mut out = child.stdout.take().ok_or("no audio pipe from arecord")?;
        let err = Arc::new(Mutex::new(String::new()));
        if let Some(mut e) = child.stderr.take() {
            let t = err.clone();
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                if let Ok(mut g) = t.lock() {
                    *g = s;
                }
            });
        }
        let (stop, level) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)));
        let (st, lv) = (stop.clone(), level.clone());
        let join = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut bytes = 0u64;
            let mut carry: Option<u8> = None;
            while !st.load(Ordering::Relaxed) {
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let _ = file.write_all(&buf[..n]);
                bytes += n as u64;
                let mut samples: Vec<i16> = Vec::with_capacity(n / 2 + 1);
                let mut it = buf[..n].iter();
                if let Some(c) = carry.take() {
                    if let Some(&b) = it.next() {
                        samples.push(i16::from_le_bytes([c, b]));
                    }
                }
                let rest: Vec<u8> = it.copied().collect();
                for p in rest.chunks_exact(2) {
                    samples.push(i16::from_le_bytes([p[0], p[1]]));
                }
                if rest.len() % 2 == 1 {
                    carry = rest.last().copied();
                }
                let (peak, _) = peak_rms(&samples);
                lv.store((peak * 1000.0) as u32, Ordering::Relaxed);
            }
            (file, bytes)
        });
        Ok(Box::new(ArecordRec { child, stop, level, join: Some(join), path, err }))
    }

    fn call(&self, clip: &Path, out: &Path) -> VR<String> {
        let mut child = Command::new(&self.sdk_command)
            .args(call_args(&self.vm_id, clip, out))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    format!("Muse SDK not found ({}): install musegadget or set muse.sdk_command", self.sdk_command)
                } else {
                    format!("cannot run the Muse SDK: {e}")
                }
            })?;
        // A watchdog: the SDK has its own limits, but a hung process must not hang the panel.
        let pid = child.id();
        let done = Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_secs(OVERALL_TIMEOUT_S as u64 + 30) {
                if d2.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        });
        let (mut so, mut se) = (String::new(), String::new());
        if let Some(mut o) = child.stdout.take() {
            let _ = o.read_to_string(&mut so);
        }
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut se);
        }
        let status = child.wait().map_err(|e| format!("Muse SDK: {e}"))?;
        done.store(true, Ordering::Relaxed);
        if !status.success() {
            return Err(explain_call_failure(status.code(), &so, &se));
        }
        let len = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
        if len <= 44 {
            return Err("Muse sent no spoken reply".into());
        }
        Ok(parse_end_reason(&so).unwrap_or_else(|| "done".into()))
    }

    fn play(&self, wav: &Path) -> VR<()> {
        let try_player = |cmd: &str, args: &[&str]| -> Option<VR<()>> {
            match Command::new(cmd).args(args).arg(wav).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
                Ok(s) if s.success() => Some(Ok(())),
                Ok(_) => Some(Err(format!("{cmd} could not play the reply"))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => Some(Err(format!("cannot run {cmd}: {e}"))),
            }
        };
        try_player("aplay", &["-q"])
            .or_else(|| try_player("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]))
            .unwrap_or_else(|| Err("No audio player (apt install alsa-utils)".into()))
    }
}

impl Recording for ArecordRec {
    fn level(&self) -> f32 {
        self.level.load(Ordering::Relaxed) as f32 / 1000.0
    }

    fn finish(mut self: Box<Self>) -> VR<Clip> {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let (mut file, bytes) = self.join.take().and_then(|j| j.join().ok()).ok_or("the recording thread failed")?;
        use std::io::{Seek, SeekFrom};
        let _ = file.seek(SeekFrom::Start(0));
        let _ = file.write_all(&wav_header(VOICE_RATE, 1, bytes as u32));
        let _ = file.flush();
        if bytes == 0 {
            let e = self.err.lock().map(|s| s.lines().next().unwrap_or("").to_string()).unwrap_or_default();
            let _ = std::fs::remove_file(&self.path);
            return Err(format!("No sound from the microphone{}", if e.is_empty() { String::new() } else { format!(" ({})", e.chars().take(50).collect::<String>()) }));
        }
        Ok(Clip { path: self.path.clone(), seconds: bytes as f32 / 2.0 / VOICE_RATE as f32 })
    }

    fn cancel(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.join.take().map(|j| j.join());
        let _ = std::fs::remove_file(&self.path);
    }
}

// ------------------------------------------------------------------ the fake (tests, HARBOUR_MOCK)

/// No audio, no processes: a synthetic level, tiny WAV files, canned results.
pub struct FakeVoice {
    pub fail_mic: Option<String>,
    pub fail_call: Option<String>,
    pub call_ms: u64,
    pub play_ms: u64,
    /// What happened, in order: "rec", "call:<vm>", "play".
    pub log: Mutex<Vec<String>>,
}

impl FakeVoice {
    pub fn new() -> Self {
        Self { fail_mic: None, fail_call: None, call_ms: 0, play_ms: 0, log: Mutex::new(vec![]) }
    }
    /// For HARBOUR_MOCK screenshots: visible durations.
    pub fn demo() -> Self {
        Self { call_ms: 1800, play_ms: 1500, ..Self::new() }
    }
}

struct FakeRec {
    t0: Instant,
    path: PathBuf,
    fixed_secs: Option<f32>,
}

impl Recording for FakeRec {
    fn level(&self) -> f32 {
        // A breathing, slightly jittery level.
        let t = self.t0.elapsed().as_millis() as f32 / 1000.0;
        (0.45 + 0.35 * (t * 7.0).sin() * (t * 2.3).cos()).clamp(0.0, 1.0)
    }
    fn finish(self: Box<Self>) -> VR<Clip> {
        let secs = self.fixed_secs.unwrap_or(self.t0.elapsed().as_secs_f32());
        let mut f = std::fs::File::create(&self.path).map_err(|e| e.to_string())?;
        f.write_all(&wav_header(VOICE_RATE, 1, 2)).and_then(|_| f.write_all(&[0, 0])).map_err(|e| e.to_string())?;
        Ok(Clip { path: self.path.clone(), seconds: secs })
    }
    fn cancel(self: Box<Self>) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl VoiceBackend for FakeVoice {
    fn start_recording(&self, dir: &Path) -> VR<Box<dyn Recording>> {
        if let Some(e) = &self.fail_mic {
            return Err(e.clone());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        self.log.lock().unwrap().push("rec".into());
        Ok(Box::new(FakeRec { t0: Instant::now(), path: dir.join("fake-in.wav"), fixed_secs: None }))
    }
    fn call(&self, clip: &Path, out: &Path) -> VR<String> {
        self.log.lock().unwrap().push(format!("call:{}", clip.file_name().and_then(|n| n.to_str()).unwrap_or("")));
        std::thread::sleep(Duration::from_millis(self.call_ms));
        if let Some(e) = &self.fail_call {
            return Err(e.clone());
        }
        let mut f = std::fs::File::create(out).map_err(|e| e.to_string())?;
        f.write_all(&wav_header(VOICE_RATE, 1, 2)).and_then(|_| f.write_all(&[0, 0])).map_err(|e| e.to_string())?;
        Ok("completed".into())
    }
    fn play(&self, _wav: &Path) -> VR<()> {
        self.log.lock().unwrap().push("play".into());
        std::thread::sleep(Duration::from_millis(self.play_ms));
        Ok(())
    }
}

// ------------------------------------------------------------------ the controller

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceState {
    Idle,
    Recording,
    Sending,
    Playing,
}

/// What the chat should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceEvent {
    Info(String),
    Error(String),
}

enum Ev {
    Playing,
    Done(String),
    Failed(String),
}

pub struct VoiceController {
    backend: Arc<dyn VoiceBackend>,
    dir: PathBuf,
    state: VoiceState,
    rec: Option<Box<dyn Recording>>,
    started: Option<Instant>,
    rx: Option<Receiver<Ev>>,
}

impl VoiceController {
    pub fn new(backend: Arc<dyn VoiceBackend>, dir: PathBuf) -> Self {
        Self { backend, dir, state: VoiceState::Idle, rec: None, started: None, rx: None }
    }

    pub fn state(&self) -> VoiceState {
        self.state
    }

    pub fn active(&self) -> bool {
        self.state != VoiceState::Idle
    }

    pub fn level(&self) -> f32 {
        self.rec.as_ref().map_or(0.0, |r| r.level())
    }

    /// The key went down long enough: start recording. Only from idle.
    pub fn press(&mut self) -> Result<(), String> {
        if self.state != VoiceState::Idle {
            return Err("busy".into());
        }
        let rec = self.backend.start_recording(&self.dir)?;
        self.rec = Some(rec);
        self.started = Some(Instant::now());
        self.state = VoiceState::Recording;
        Ok(())
    }

    /// The key came up: stop, and send the clip to Muse on a worker thread.
    pub fn release(&mut self) -> Vec<VoiceEvent> {
        if self.state != VoiceState::Recording {
            return vec![];
        }
        let Some(rec) = self.rec.take() else { return vec![] };
        self.started = None;
        let clip = match rec.finish() {
            Ok(c) => c,
            Err(e) => {
                self.state = VoiceState::Idle;
                return vec![VoiceEvent::Error(e)];
            }
        };
        if clip.seconds < MIN_SECS {
            let _ = std::fs::remove_file(&clip.path);
            self.state = VoiceState::Idle;
            return vec![VoiceEvent::Error("too short: hold the key while you speak".into())];
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.state = VoiceState::Sending;
        let backend = self.backend.clone();
        let out = self.dir.join(format!("out-{}.wav", std::process::id()));
        let secs = clip.seconds;
        std::thread::spawn(move || {
            let r = backend.call(&clip.path, &out);
            let _ = std::fs::remove_file(&clip.path);
            match r {
                Err(e) => {
                    let _ = tx.send(Ev::Failed(e));
                }
                Ok(reason) => {
                    let _ = tx.send(Ev::Playing);
                    let p = backend.play(&out);
                    let _ = std::fs::remove_file(&out);
                    let _ = tx.send(match p {
                        Ok(()) => Ev::Done(reason),
                        Err(e) => Ev::Failed(e),
                    });
                }
            }
        });
        vec![VoiceEvent::Info(format!("voice: sent {secs:.1} s"))]
    }

    /// Poll the worker; also stops a recording that ran past [`MAX_SECS`].
    pub fn tick(&mut self) -> Vec<VoiceEvent> {
        let mut out = vec![];
        if self.state == VoiceState::Recording && self.started.map_or(false, |t| t.elapsed() >= Duration::from_secs(MAX_SECS)) {
            out.extend(self.release());
        }
        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(Ev::Playing) => self.state = VoiceState::Playing,
                    Ok(Ev::Done(r)) => {
                        out.push(VoiceEvent::Info(format!("voice: Muse answered ({r}); played, no transcript from the SDK")));
                        self.state = VoiceState::Idle;
                        self.rx = None;
                        break;
                    }
                    Ok(Ev::Failed(e)) => {
                        out.push(VoiceEvent::Error(e));
                        self.state = VoiceState::Idle;
                        self.rx = None;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(_) => {
                        out.push(VoiceEvent::Error("the voice worker stopped".into()));
                        self.state = VoiceState::Idle;
                        self.rx = None;
                        break;
                    }
                }
            }
        }
        out
    }

    /// The panel closed while recording: throw the audio away.
    pub fn cancel_recording(&mut self) {
        if let Some(r) = self.rec.take() {
            r.cancel();
            self.started = None;
            self.state = VoiceState::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agent-voice-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn wait_idle(c: &mut VoiceController) -> Vec<VoiceEvent> {
        let mut all = vec![];
        for _ in 0..400 {
            all.extend(c.tick());
            if c.state() == VoiceState::Idle {
                return all;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("voice did not finish");
    }

    #[test]
    fn the_sdk_command_line_is_exact_and_carries_no_secret() {
        let a = call_args("vm-123", Path::new("/t/in.wav"), Path::new("/t/out.wav"));
        assert_eq!(a, ["voice-call", "--vm-id", "vm-123", "--in-wav", "/t/in.wav", "--out-wav", "/t/out.wav", "--idle-timeout", "20", "--overall-timeout", "120"]);
    }

    #[test]
    fn sdk_failures_become_plain_english() {
        let e = |code, so: &str, se: &str| explain_call_failure(code, so, se);
        assert!(e(Some(1), "", "Not paired; run `musegadget pair` first.").contains("not paired"));
        assert!(e(Some(1), "", "--vm-id must exactly match one leased VM").contains("muse.vm_id"));
        assert!(e(Some(1), "", "Voice call failed (ConnectionError).").contains("(ConnectionError)"));
        assert!(e(Some(1), "", "VM lookup failed; refresh").contains("lookup"));
        assert!(e(Some(7), "", "").contains("exit 7"));
        assert_eq!(parse_end_reason("hello\nVoice call ended: idle_timeout.\n").as_deref(), Some("idle_timeout"));
        assert_eq!(parse_end_reason("nothing"), None);
    }

    #[test]
    fn hold_record_send_play_with_fakes_and_no_processes() {
        let backend = Arc::new(FakeVoice::new());
        let mut c = VoiceController::new(backend.clone(), dir("ok"));
        assert_eq!(c.state(), VoiceState::Idle);
        c.press().unwrap();
        assert_eq!(c.state(), VoiceState::Recording);
        assert!(c.press().is_err(), "a second press while busy is refused");
        assert!((0.0..=1.0).contains(&c.level()));
        std::thread::sleep(Duration::from_millis(500));
        let ev = c.release();
        assert!(matches!(&ev[0], VoiceEvent::Info(t) if t.starts_with("voice: sent")), "{ev:?}");
        assert_eq!(c.state(), VoiceState::Sending);
        let ev = wait_idle(&mut c);
        assert!(ev.iter().any(|e| matches!(e, VoiceEvent::Info(t) if t.contains("Muse answered") && t.contains("no transcript"))), "{ev:?}");
        assert_eq!(*backend.log.lock().unwrap(), ["rec", "call:fake-in.wav", "play"]);
        let _ = std::fs::remove_dir_all(dir("ok"));
    }

    #[test]
    fn too_short_clips_and_failures_are_reported_and_leave_idle() {
        // Too short.
        let mut c = VoiceController::new(Arc::new(FakeVoice::new()), dir("short"));
        c.press().unwrap();
        assert_eq!(c.release(), vec![VoiceEvent::Error("too short: hold the key while you speak".into())]);
        assert_eq!(c.state(), VoiceState::Idle);
        // No microphone.
        let mut f = FakeVoice::new();
        f.fail_mic = Some("No microphone found".into());
        let mut c = VoiceController::new(Arc::new(f), dir("nomic"));
        assert_eq!(c.press(), Err("No microphone found".into()));
        assert_eq!(c.state(), VoiceState::Idle);
        // The SDK fails: the error comes back, nothing is played.
        let mut f = FakeVoice::new();
        f.fail_call = Some("Muse SDK not found (musegadget): install musegadget or set muse.sdk_command".into());
        let f = Arc::new(f);
        let mut c = VoiceController::new(f.clone(), dir("fail"));
        c.press().unwrap();
        std::thread::sleep(Duration::from_millis(450));
        c.release();
        let ev = wait_idle(&mut c);
        assert!(matches!(&ev[0], VoiceEvent::Error(t) if t.contains("SDK not found")), "{ev:?}");
        assert!(!f.log.lock().unwrap().contains(&"play".to_string()));
        for t in ["short", "nomic", "fail"] {
            let _ = std::fs::remove_dir_all(dir(t));
        }
    }

    #[test]
    fn closing_the_panel_cancels_the_recording() {
        let mut c = VoiceController::new(Arc::new(FakeVoice::new()), dir("cancel"));
        c.press().unwrap();
        c.cancel_recording();
        assert_eq!(c.state(), VoiceState::Idle);
        assert!(c.release().is_empty());
        let _ = std::fs::remove_dir_all(dir("cancel"));
    }
}
