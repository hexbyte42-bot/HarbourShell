//! LCD output abstraction: a DRM/KMS dumb-buffer backend (preferred) and the legacy `/dev/fb0`
//! backend (automatic fallback). Everything that does not need hardware lives here so it can be
//! unit-tested on the Mac.

use std::io;

/// Half-open pixel rectangle (`x1`/`y1` exclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Rect {
    pub fn full(w: usize, h: usize) -> Self {
        Rect { x0: 0, y0: 0, x1: w, y1: h }
    }
    pub fn width(&self) -> usize {
        self.x1 - self.x0
    }
    pub fn height(&self) -> usize {
        self.y1 - self.y0
    }
    pub fn area(&self) -> usize {
        self.width() * self.height()
    }
}

/// Bounding box of the pixels that differ between two tightly packed RGB565 frames
/// (`w*2` bytes per row). `None` when the frames are identical.
pub fn damage_rect(prev: &[u8], new: &[u8], w: usize, h: usize) -> Option<Rect> {
    let row = w * 2;
    assert!(prev.len() >= row * h && new.len() >= row * h);
    let (mut x0, mut x1) = (usize::MAX, 0usize);
    let (mut y0, mut y1) = (usize::MAX, 0usize);
    for y in 0..h {
        let a = &prev[y * row..(y + 1) * row];
        let b = &new[y * row..(y + 1) * row];
        if a == b {
            continue;
        }
        // first / last differing pixel in this row
        let first = (0..w).find(|&x| a[2 * x..2 * x + 2] != b[2 * x..2 * x + 2]).unwrap();
        let last = (0..w).rev().find(|&x| a[2 * x..2 * x + 2] != b[2 * x..2 * x + 2]).unwrap();
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    if y0 == usize::MAX {
        None
    } else {
        Some(Rect { x0, y0, x1, y1 })
    }
}

/// RGB565 (little endian bytes) -> XRGB8888 pixel value.
pub fn rgb565_to_xrgb8888(lo: u8, hi: u8) -> u32 {
    let p = u16::from_le_bytes([lo, hi]) as u32;
    let (r, g, b) = ((p >> 11) & 31, (p >> 5) & 63, p & 31);
    let (r, g, b) = ((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2));
    (r << 16) | (g << 8) | b
}

/// What the launcher needs from the LCD. Frames are tightly packed RGB565, `width*2` bytes/row.
pub trait Display {
    /// Send a frame; returns the bytes pushed to the device. Unchanged frames cost nothing.
    fn present(&mut self, rgb565: &[u8]) -> io::Result<usize>;
    /// Forget the previous frame so the next `present` rewrites everything.
    fn invalidate(&mut self);
    /// Something else drew on the LCD since our last `present` (a launched app).
    fn taken_over(&self) -> bool;
    /// Make sure the panel is lit. (No FBIOBLANK: it once left the LCD dead.)
    fn unblank(&self) {}
    /// An external app is about to use the LCD: hand it over (DRM: back to fbdev emulation).
    fn enter_external(&mut self) {}
    /// The external app is gone: take the LCD back.
    fn leave_external(&mut self) {}
    /// Re-pushing the whole frame shortly after a change heals SPI glitches on fbdev; DRM does not need it.
    fn wants_refresh(&self) -> bool {
        true
    }
    fn backend(&self) -> &'static str;
}

/// Open the best available backend: DRM unless `HARBOUR_DISPLAY=fbdev`, else (or on any failure)
/// the framebuffer at `fbdev_path`.
#[cfg(target_os = "linux")]
pub fn open(fbdev_path: &str) -> io::Result<Box<dyn Display>> {
    let want = std::env::var("HARBOUR_DISPLAY").unwrap_or_else(|_| "auto".into());
    if want != "fbdev" {
        let dev = std::env::var("HARBOUR_DRM").ok();
        match super::drm::DrmDisplay::open(dev.as_deref(), fbdev_path) {
            Ok(d) => {
                eprintln!("[display] backend: DRM {}", d.describe());
                return Ok(Box::new(d));
            }
            Err(e) => {
                eprintln!("[display] DRM unavailable ({e}); falling back to fbdev {fbdev_path}");
                if want == "drm" {
                    return Err(e);
                }
            }
        }
    }
    let f = super::fb::Framebuffer::open(fbdev_path)?;
    eprintln!("[display] backend: fbdev {fbdev_path}");
    Ok(Box::new(f))
}

#[cfg(not(target_os = "linux"))]
pub fn open(_: &str) -> io::Result<Box<dyn Display>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "LCD output needs Linux"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize) -> Vec<u8> {
        vec![0u8; w * h * 2]
    }

    #[test]
    fn identical_frames_have_no_damage() {
        let a = frame(8, 4);
        assert_eq!(damage_rect(&a, &a.clone(), 8, 4), None);
    }

    #[test]
    fn single_pixel() {
        let a = frame(8, 4);
        let mut b = a.clone();
        b[(2 * 8 + 5) * 2 + 1] = 0xff; // x=5, y=2 (high byte)
        assert_eq!(damage_rect(&a, &b, 8, 4), Some(Rect { x0: 5, y0: 2, x1: 6, y1: 3 }));
    }

    #[test]
    fn bounding_box_of_two_regions() {
        let a = frame(10, 10);
        let mut b = a.clone();
        b[(1 * 10 + 7) * 2] = 1; // (7,1)
        b[(8 * 10 + 2) * 2] = 1; // (2,8)
        let r = damage_rect(&a, &b, 10, 10).unwrap();
        assert_eq!(r, Rect { x0: 2, y0: 1, x1: 8, y1: 9 });
        assert_eq!(r.area(), 6 * 8);
    }

    #[test]
    fn full_change_is_full_rect() {
        let a = frame(6, 3);
        let b = vec![0xffu8; 6 * 3 * 2];
        assert_eq!(damage_rect(&a, &b, 6, 3), Some(Rect::full(6, 3)));
    }

    #[test]
    fn xrgb_conversion() {
        assert_eq!(rgb565_to_xrgb8888(0xff, 0xff), 0x00ff_ffff);
        assert_eq!(rgb565_to_xrgb8888(0, 0), 0);
        assert_eq!(rgb565_to_xrgb8888(0x00, 0xf8), 0x00ff_0000); // pure red
        assert_eq!(rgb565_to_xrgb8888(0x1f, 0x00), 0x0000_00ff); // pure blue
    }
}
