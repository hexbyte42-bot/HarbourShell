//! The built-in CSI camera of the full Cardputer Zero (imx219 behind libcamera), through the
//! Raspberry Pi tools when installed: `rpicam-vid` (or `libcamera-vid`) streams MJPEG into a
//! pipe for the preview, `rpicam-still` (or `libcamera-still`) takes the full-resolution photo.
//! The tools are only looked up on PATH while probing and started when the camera is opened;
//! the child is killed when the stream is dropped. Not testable on the Lite model (no sensor).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::backend::*;
use super::convert::Raw;

pub const CSI_PATH: &str = "csi";
const SENSOR_DRIVERS: [&str; 6] = ["imx219", "imx477", "imx708", "ov5647", "imx296", "imx519"];

/// First of `names` that is an executable file in one of the PATH directories (nothing is run).
pub fn find_tool(names: &[&str], path_var: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    for n in names {
        for dir in path_var.split(':').filter(|d| !d.is_empty()) {
            let p = Path::new(dir).join(n);
            if std::fs::metadata(&p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

/// A camera sensor driver bound to an I2C device (`<root>/bus/i2c/drivers/imx219/10-0010`).
pub fn sensor_present(sys_root: &Path) -> Option<String> {
    for d in SENSOR_DRIVERS {
        let dir = sys_root.join("bus/i2c/drivers").join(d);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            if rd.flatten().any(|e| e.file_name().to_string_lossy().chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)) {
                return Some(d.to_string());
            }
        }
    }
    None
}

pub struct Csi {
    sys_root: PathBuf,
    path_var: String,
}

impl Csi {
    pub fn system() -> Csi {
        Csi { sys_root: PathBuf::from("/sys"), path_var: std::env::var("PATH").unwrap_or_default() }
    }

    fn vid(&self) -> Option<PathBuf> {
        find_tool(&["rpicam-vid", "libcamera-vid"], &self.path_var)
    }
    fn still(&self) -> Option<PathBuf> {
        find_tool(&["rpicam-still", "libcamera-still"], &self.path_var)
    }

    /// The device and/or a note for the probe list.
    pub fn probe(&self) -> (Option<Device>, Option<String>) {
        let Some(sensor) = sensor_present(&self.sys_root) else { return (None, None) };
        if self.vid().is_none() {
            return (None, Some(format!("csi: {sensor} found, but rpicam-vid is not installed")));
        }
        let dev = Device { path: CSI_PATH.into(), name: "Built-in camera".into(), uvc: false, formats: vec![MJPG], sizes: vec![(320, 240), (640, 480), (1280, 720)] };
        (Some(dev), Some(format!("csi: {sensor} ok")))
    }

    pub fn open(&self, size: Option<(u32, u32)>) -> Result<Box<dyn Source>, String> {
        let tool = self.vid().ok_or("rpicam-vid is not installed")?;
        let (w, h) = size.unwrap_or((640, 480));
        let mut child = Command::new(tool).args(vid_args(w, h)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("rpicam-vid: {e}"))?;
        let out = child.stdout.take().ok_or("rpicam-vid: no pipe")?;
        let (tx, rx) = sync_channel::<Vec<u8>>(2);
        std::thread::Builder::new()
            .name("camera-csi-read".into())
            .spawn(move || {
                let (mut out, mut split, mut buf) = (out, MjpegSplitter::default(), vec![0u8; 64 * 1024]);
                while let Ok(n) = out.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for f in split.push(&buf[..n]) {
                        let _ = tx.try_send(f); // full: the app is slower, drop the frame
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Box::new(CsiSource { child, rx, w, h, started: Instant::now() }))
    }

    /// Full-resolution PNG through `rpicam-still` (the preview stream must be stopped first:
    /// the sensor has one owner). Writes to `out`.
    pub fn take_photo(&self, out: &Path) -> Result<(), String> {
        let tool = self.still().ok_or("rpicam-still is not installed")?;
        run_with_timeout(Command::new(tool).args(still_args(out)), Duration::from_secs(20))
    }
}

pub fn vid_args(w: u32, h: u32) -> Vec<String> {
    ["-t", "0", "--codec", "mjpeg", "--width", &w.to_string(), "--height", &h.to_string(), "--framerate", "15", "--nopreview", "-o", "-"].iter().map(|s| s.to_string()).collect()
}

pub fn still_args(out: &Path) -> Vec<String> {
    ["-n", "-t", "800", "--encoding", "png", "-o", &out.to_string_lossy()].iter().map(|s| s.to_string()).collect()
}

fn run_with_timeout(cmd: &mut Command, limit: Duration) -> Result<(), String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(s)) => return Err(format!("photo tool failed ({s})")),
            Ok(None) if t0.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("photo tool timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

struct CsiSource {
    child: Child,
    rx: Receiver<Vec<u8>>,
    w: u32,
    h: u32,
    started: Instant,
}

impl Source for CsiSource {
    fn describe(&self) -> (String, u32, u32) {
        ("MJPG".into(), self.w, self.h)
    }

    fn next(&mut self, timeout: Duration) -> Result<Option<Raw>, String> {
        let mut latest = match self.rx.recv_timeout(timeout) {
            Ok(f) => f,
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(match self.child.try_wait() {
                    Ok(Some(s)) => format!("rpicam-vid stopped ({s}) after {} s", self.started.elapsed().as_secs()),
                    _ => "rpicam-vid stopped".into(),
                })
            }
        };
        while let Ok(f) = self.rx.try_recv() {
            latest = f; // drop older frames
        }
        Ok(Some(Raw::Mjpeg(latest)))
    }
}

impl Drop for CsiSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Cuts a concatenated MJPEG stream into frames (SOI `FFD8` .. EOI `FFD9`).
#[derive(Default)]
pub struct MjpegSplitter {
    buf: Vec<u8>,
}

impl MjpegSplitter {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();
        loop {
            let Some(s) = self.buf.windows(2).position(|w| w == [0xFF, 0xD8]) else {
                let keep = self.buf.last() == Some(&0xFF); // a marker may be split across chunks
                self.buf.clear();
                if keep {
                    self.buf.push(0xFF);
                }
                break;
            };
            self.buf.drain(..s);
            let Some(e) = self.buf.windows(2).skip(2).position(|w| w == [0xFF, 0xD9]) else { break };
            let end = e + 2 + 2;
            frames.push(self.buf.drain(..end).collect());
        }
        if self.buf.len() > 4 << 20 {
            self.buf.clear(); // garbage without an end marker
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("harbour_csi_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn splitter_cuts_frames_across_chunks() {
        let f1 = [0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9];
        let f2 = [0xFF, 0xD8, 9, 0xFF, 0xD9];
        let mut s = MjpegSplitter::default();
        let mut all = Vec::new();
        all.extend([7, 7]); // junk before the first frame
        all.extend(f1);
        all.extend(f2);
        let mut got = Vec::new();
        for chunk in all.chunks(3) {
            got.extend(s.push(chunk));
        }
        assert_eq!(got, vec![f1.to_vec(), f2.to_vec()]);
        assert!(s.push(&[0xFF, 0xD8, 5]).is_empty(), "an unfinished frame waits");
    }

    #[test]
    fn finds_tools_and_sensors_without_running_anything() {
        let root = tmp("sys");
        assert_eq!(sensor_present(&root), None);
        std::fs::create_dir_all(root.join("bus/i2c/drivers/imx219/10-0010")).unwrap();
        std::fs::create_dir_all(root.join("bus/i2c/drivers/imx219/module")).unwrap();
        assert_eq!(sensor_present(&root).as_deref(), Some("imx219"));

        let bin = tmp("bin");
        let tool = bin.join("libcamera-vid");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        assert_eq!(find_tool(&["rpicam-vid", "libcamera-vid"], bin.to_str().unwrap()), None, "not executable yet");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_tool(&["rpicam-vid", "libcamera-vid"], &format!("/nonexistent:{}", bin.display())), Some(tool));

        let csi = Csi { sys_root: root.clone(), path_var: bin.display().to_string() };
        let (dev, note) = csi.probe();
        assert_eq!(dev.unwrap().name, "Built-in camera");
        assert_eq!(note.as_deref(), Some("csi: imx219 ok"));
        let none = Csi { sys_root: root, path_var: String::new() };
        let (dev, note) = none.probe();
        assert!(dev.is_none() && note.unwrap().contains("not installed"));
    }

    #[test]
    fn command_lines_follow_the_pi_tools() {
        let a = vid_args(640, 480).join(" ");
        assert_eq!(a, "-t 0 --codec mjpeg --width 640 --height 480 --framerate 15 --nopreview -o -");
        assert!(still_args(Path::new("/p/x.png")).join(" ").ends_with("--encoding png -o /p/x.png"));
    }

    #[test]
    fn streams_frames_from_a_child_and_kills_it() {
        let bin = tmp("fake");
        let tool = bin.join("rpicam-vid");
        // Prints two tiny "frames" then sleeps like a stream that never ends.
        std::fs::write(&tool, "#!/bin/sh\nprintf '\\377\\330abc\\377\\331\\377\\330de\\377\\331'\nsleep 30\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let csi = Csi { sys_root: PathBuf::from("/nonexistent"), path_var: bin.display().to_string() };
        let mut src = csi.open(Some((320, 240))).unwrap();
        assert_eq!(src.describe(), ("MJPG".to_string(), 320, 240));
        let mut seen = 0;
        for _ in 0..20 {
            if let Some(Raw::Mjpeg(f)) = src.next(Duration::from_millis(200)).unwrap() {
                assert_eq!(&f[..2], &[0xFF, 0xD8]);
                seen += 1;
                if seen >= 1 {
                    break;
                }
            }
        }
        assert!(seen >= 1);
        let t = Instant::now();
        drop(src); // kills the child, must not wait for the 30 s sleep
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn photo_tool_runs_with_a_timeout() {
        let bin = tmp("still");
        let tool = bin.join("rpicam-still");
        std::fs::write(&tool, "#!/bin/sh\nfor a; do last=$a; done\necho png > \"$last\"\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let csi = Csi { sys_root: PathBuf::from("/nonexistent"), path_var: bin.display().to_string() };
        let out = bin.join("x.png");
        csi.take_photo(&out).unwrap();
        assert!(out.exists());
        let none = Csi { sys_root: PathBuf::from("/nonexistent"), path_var: String::new() };
        assert!(none.take_photo(&out).unwrap_err().contains("not installed"));
    }
}
