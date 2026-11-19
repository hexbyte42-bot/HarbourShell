//! A process-wide audio deck. The launcher creates and drops apps whenever
//! they open/close; the ffplay child must outlive the Radio app so listening
//! continues while using other apps. Playback stops when the launcher exits.

use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::net;

#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub label: String,
    pub url: String,
    pub podcast: bool,
    pub paused: bool,
}

#[derive(Default)]
struct Player {
    child: Option<Child>,
    state: Option<NowPlaying>,
    /// Play time before the last pause, and when playing last resumed (the tape counter).
    played: Duration,
    since: Option<Instant>,
}

static PLAYER: OnceLock<Mutex<Player>> = OnceLock::new();

fn player() -> &'static Mutex<Player> {
    PLAYER.get_or_init(|| Mutex::new(Player::default()))
}

fn stop_locked(p: &mut Player) {
    if let Some(mut child) = p.child.take() {
        // A SIGSTOP'd process must receive SIGCONT before termination.
        #[cfg(unix)]
        unsafe { libc::kill(child.id() as i32, libc::SIGCONT); }
        let _ = child.kill();
        let _ = child.wait();
    }
    p.state = None;
    p.played = Duration::ZERO;
    p.since = None;
}

pub fn stop() {
    if let Ok(mut p) = player().lock() {
        stop_locked(&mut p);
    }
}

pub fn play(url: &str, label: &str, podcast: bool) -> Result<(), String> {
    if !net::http_url(url) {
        return Err("Only HTTP(S) audio streams can play".into());
    }
    // A fresh child is created before the previous station is stopped so a
    // missing ffplay installation does not silence something already playing.
    let mut cmd = Command::new("ffplay");
    cmd.args(["-nodisp", "-autoexit", "-loglevel", "error", "-i"])
        .arg(url)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // On the Cardputer (Linux), a quit/crash must not leave orphaned
    // ffplay streams running. SIGKILL also terminates a paused child.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = cmd.spawn().map_err(|e| format!("ffplay: {e}"))?;
    let mut p = player().lock().map_err(|_| "Audio player unavailable".to_string())?;
    stop_locked(&mut p);
    p.child = Some(child);
    p.state = Some(NowPlaying {
        label: label.to_string(), url: url.to_string(), podcast, paused: false,
    });
    p.since = Some(Instant::now());
    Ok(())
}

pub fn toggle_pause() {
    if let Ok(mut p) = player().lock() {
        let Some(child) = p.child.as_ref() else { return };
        let was_paused = p.state.as_ref().is_some_and(|s| s.paused);
        #[cfg(unix)]
        unsafe { libc::kill(child.id() as i32, if was_paused { libc::SIGCONT } else { libc::SIGSTOP }); }
        if let Some(s) = &mut p.state {
            s.paused = !was_paused;
        }
        if was_paused {
            p.since = Some(Instant::now());
        } else if let Some(t) = p.since.take() {
            p.played += t.elapsed();
        }
    }
}

/// Poll a child on the UI thread without waiting for it. Stale / failed
/// streams disappear from the display on the next tick.
pub fn snapshot() -> Option<NowPlaying> {
    let mut p = player().lock().ok()?;
    if let Some(child) = &mut p.child {
        if !matches!(child.try_wait(), Ok(None)) {
            p.child = None;
            p.state = None;
        }
    }
    p.state.clone()
}

/// How long the current stream has played (pauses not counted); `None` when nothing plays.
pub fn elapsed() -> Option<Duration> {
    let p = player().lock().ok()?;
    p.state.as_ref()?;
    Some(p.played + p.since.map(|t| t.elapsed()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_uris_never_spawn_players() {
        assert!(play("file:///etc/passwd", "bad", false).is_err());
    }
}
