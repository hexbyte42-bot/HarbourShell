//! Mouse input (Linux evdev): finds pointing devices — including Bluetooth mice
//! that connect later — and turns their events into [`Input`] values.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::keys::Input;
use super::Shared;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

/// `capabilities/rel` is a hex bitmask; a pointer has REL_X (bit 0) and REL_Y (bit 1).
pub fn has_rel_xy(caps: &str) -> bool {
    u64::from_str_radix(caps.trim().rsplit(' ').next().unwrap_or("0"), 16)
        .map(|v| v & 0b11 == 0b11)
        .unwrap_or(false)
}

/// `capabilities/key` lists 64-bit hex words, most significant first. A mouse
/// has BTN_LEFT (0x110 = bit 272), which HDMI-CEC and IR receivers lack.
pub fn has_btn_left(caps: &str) -> bool {
    let words: Vec<u64> = caps.split_whitespace().filter_map(|w| u64::from_str_radix(w, 16).ok()).collect();
    let bit = BTN_LEFT as usize;
    let (word, off) = (bit / 64, bit % 64);
    words.len() > word && words[words.len() - 1 - word] & (1u64 << off) != 0
}

/// Event nodes that look like mice right now.
pub fn find_mice() -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..32 {
        let base = format!("/sys/class/input/event{i}/device/capabilities");
        let rel = std::fs::read_to_string(format!("{base}/rel")).unwrap_or_default();
        let key = std::fs::read_to_string(format!("{base}/key")).unwrap_or_default();
        if has_rel_xy(&rel) && has_btn_left(&key) {
            out.push(format!("/dev/input/event{i}"));
        }
    }
    out
}

/// Accumulates one SYN frame of events.
#[derive(Default)]
pub struct Frame {
    dx: i32,
    dy: i32,
    wheel: i32,
    buttons: Vec<(u8, bool)>,
}

impl Frame {
    /// Feed one raw event; returns the inputs of a completed frame on `EV_SYN`.
    pub fn feed(&mut self, etype: u16, code: u16, value: i32) -> Vec<Input> {
        match etype {
            EV_REL => match code {
                REL_X => self.dx += value,
                REL_Y => self.dy += value,
                REL_WHEEL => self.wheel += value,
                _ => {}
            },
            EV_KEY if value != 2 => {
                let b = match code {
                    BTN_LEFT => Some(0),
                    BTN_RIGHT => Some(1),
                    BTN_MIDDLE => Some(2),
                    _ => None,
                };
                if let Some(b) = b {
                    self.buttons.push((b, value == 1));
                }
            }
            EV_SYN => return self.take(),
            _ => {}
        }
        Vec::new()
    }

    fn take(&mut self) -> Vec<Input> {
        let mut out = Vec::new();
        if self.dx != 0 || self.dy != 0 {
            out.push(Input::MouseMove { dx: self.dx, dy: self.dy });
        }
        for (button, down) in self.buttons.drain(..) {
            out.push(Input::MouseButton { button, down });
        }
        if self.wheel != 0 {
            out.push(Input::MouseWheel { dy: self.wheel });
        }
        self.dx = 0;
        self.dy = 0;
        self.wheel = 0;
        out
    }
}

struct Dev {
    file: File,
    frame: Frame,
}

pub fn spawn(tx: Sender<Input>, shared: Arc<Shared>) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().name("mouse".into()).spawn(move || {
        let mut devs: HashMap<String, Dev> = HashMap::new();
        let mut next_scan = Instant::now();
        let mut buf = [0u8; 24 * 32];
        while !shared.quit.load(Ordering::Relaxed) {
            if Instant::now() >= next_scan {
                next_scan = Instant::now() + Duration::from_secs(2);
                for path in find_mice() {
                    if !devs.contains_key(&path) {
                        if let Ok(file) = File::open(&path) {
                            eprintln!("[mouse] connected: {path}");
                            devs.insert(path, Dev { file, frame: Frame::default() });
                        }
                    }
                }
            }
            if devs.is_empty() {
                std::thread::sleep(Duration::from_millis(400));
                continue;
            }
            let paths: Vec<String> = devs.keys().cloned().collect();
            let mut pfds: Vec<libc::pollfd> = paths
                .iter()
                .map(|p| libc::pollfd { fd: devs[p].file.as_raw_fd(), events: libc::POLLIN, revents: 0 })
                .collect();
            let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 300) };
            if r <= 0 {
                continue;
            }
            for (i, pfd) in pfds.iter().enumerate() {
                if pfd.revents == 0 {
                    continue;
                }
                let path = &paths[i];
                if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    eprintln!("[mouse] disconnected: {path}");
                    devs.remove(path);
                    continue;
                }
                let dev = devs.get_mut(path).unwrap();
                match dev.file.read(&mut buf) {
                    Ok(0) => {
                        devs.remove(path);
                    }
                    Ok(n) => {
                        for ev in buf[..n].chunks_exact(24) {
                            let etype = u16::from_ne_bytes([ev[16], ev[17]]);
                            let code = u16::from_ne_bytes([ev[18], ev[19]]);
                            let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                            let inputs = dev.frame.feed(etype, code, value);
                            if shared.suspended.load(Ordering::Relaxed) {
                                continue;
                            }
                            for input in inputs {
                                if tx.send(input).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        eprintln!("[mouse] disconnected: {path}");
                        devs.remove(path);
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_capability_bits() {
        assert!(has_rel_xy("903"), "REL_X | REL_Y | wheel bits");
        assert!(has_rel_xy("3\n"));
        assert!(has_rel_xy("0 903"), "multi-word masks use the last word");
        assert!(!has_rel_xy("0"), "a keyboard has no relative axes");
        assert!(!has_rel_xy("1"), "REL_X alone is not a pointer");
        assert!(!has_rel_xy("zz"));
    }

    #[test]
    fn a_mouse_has_btn_left_but_an_ir_receiver_does_not() {
        // BTN_LEFT/RIGHT/MIDDLE = bits 272..274 -> word 4 from the right, bits 16..18.
        assert!(has_btn_left("70000 0 0 0 0"));
        assert!(has_btn_left("0 0 0 70000 0 0 0 0"), "more words in front are fine");
        assert!(!has_btn_left("0 0 0 0 0"), "no buttons");
        assert!(!has_btn_left("ffffffff"), "too few words to reach bit 272");
        assert!(!has_btn_left(""));
    }

    #[test]
    fn a_syn_frame_becomes_inputs() {
        let mut f = Frame::default();
        assert!(f.feed(EV_REL, REL_X, 3).is_empty());
        assert!(f.feed(EV_REL, REL_X, 2).is_empty());
        assert!(f.feed(EV_REL, REL_Y, -4).is_empty());
        assert!(f.feed(EV_KEY, BTN_LEFT, 1).is_empty());
        assert!(f.feed(EV_REL, REL_WHEEL, 1).is_empty());
        let out = f.feed(EV_SYN, 0, 0);
        assert_eq!(
            out,
            vec![
                Input::MouseMove { dx: 5, dy: -4 },
                Input::MouseButton { button: 0, down: true },
                Input::MouseWheel { dy: 1 },
            ]
        );
        assert!(f.feed(EV_SYN, 0, 0).is_empty(), "the frame is cleared");
    }

    #[test]
    fn buttons_map_and_repeats_are_ignored() {
        let mut f = Frame::default();
        f.feed(EV_KEY, BTN_RIGHT, 1);
        f.feed(EV_KEY, BTN_RIGHT, 2);
        f.feed(EV_KEY, BTN_RIGHT, 0);
        f.feed(EV_KEY, BTN_MIDDLE, 1);
        f.feed(EV_KEY, 0x113, 1); // BTN_SIDE: ignored
        let out = f.feed(EV_SYN, 0, 0);
        assert_eq!(
            out,
            vec![
                Input::MouseButton { button: 1, down: true },
                Input::MouseButton { button: 1, down: false },
                Input::MouseButton { button: 2, down: true },
            ]
        );
    }
}
