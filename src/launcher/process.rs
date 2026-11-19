//! Running external apps. The app gets its own process group so the launcher's
//! failsafe (hold Esc) can interrupt or kill it, and it never goes through a shell.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::desktop_entry::exec_argv;
use crate::platform::Shared;

#[derive(Debug)]
pub enum RunError {
    UnsafeExec,
    Spawn(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::UnsafeExec => write!(f, "Exec is not allowed"),
            RunError::Spawn(e) => write!(f, "{e}"),
        }
    }
}

fn open_log() -> Option<(std::fs::File, std::fs::File)> {
    let dir = std::path::PathBuf::from(std::env::var("HOME").ok()?).join(".cache/harbour-shell");
    std::fs::create_dir_all(&dir).ok()?;
    let f = std::fs::File::create(dir.join("last-app.log")).ok()?;
    let g = f.try_clone().ok()?;
    Some((f, g))
}

/// Run `exec` to completion, blocking the caller. `workdir` is the APPLaunch
/// resource root so relative paths inside apps keep working.
pub fn run_blocking(
    exec: &str,
    workdir: &Path,
    shared: &Arc<Shared>,
) -> Result<ExitStatus, RunError> {
    run_watched(exec, workdir, shared, &mut |_| {})
}

/// Like [`run_blocking`], calling `on_tick(time since start)` about every 40 ms while
/// the program runs (the launcher draws its loading gauge from it).
pub fn run_watched(
    exec: &str,
    workdir: &Path,
    shared: &Arc<Shared>,
    on_tick: &mut dyn FnMut(Duration),
) -> Result<ExitStatus, RunError> {
    let t0 = std::time::Instant::now();
    let argv = exec_argv(exec).ok_or(RunError::UnsafeExec)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(Stdio::null());
    // Keep the app's output in ~/.cache/harbour-shell/last-app.log for debugging.
    match open_log() {
        Some((out, err)) => {
            cmd.stdout(Stdio::from(out)).stderr(Stdio::from(err));
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    if workdir.is_dir() {
        cmd.current_dir(workdir);
    }
    // New session/process group: the app owns the terminal-less environment and
    // `kill(-pgid, ..)` reaches everything it forks.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| RunError::Spawn(format!("{}: {e}", argv[0])))?;
    shared.child_pgid.store(child.id() as i32, Ordering::SeqCst);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                on_tick(t0.elapsed());
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                shared.child_pgid.store(0, Ordering::SeqCst);
                return Err(RunError::Spawn(e.to_string()));
            }
        }
    };
    // Reap anything the app left in its process group.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    shared.child_pgid.store(0, Ordering::SeqCst);
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_a_command_and_reports_exit() {
        let shared = Arc::new(Shared::default());
        let st = run_blocking("/usr/bin/true", Path::new("/"), &shared).unwrap();
        assert!(st.success());
        assert_eq!(shared.child_pgid.load(Ordering::SeqCst), 0);
        let st = run_blocking("/usr/bin/false", Path::new("/"), &shared).unwrap();
        assert!(!st.success());
    }

    #[test]
    fn refuses_unsafe_exec() {
        let shared = Arc::new(Shared::default());
        assert!(matches!(
            run_blocking("/bin/sh -c id; ls", Path::new("/"), &shared),
            Err(RunError::UnsafeExec)
        ));
    }
}
