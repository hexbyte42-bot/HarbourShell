//! The pure model behind Paint: a 1-bit bitmap, the drawing primitives, the eight fill patterns, flood
//! fill, run-length snapshots for undo, and 1-bit PNG encoding / decoding. No SDL in here, so everything
//! can be unit tested.
//!
//! Convention: a pixel value of `true` is ink (drawn in the theme's ink colour), `false` is paper.

pub const SHEET_W: usize = 512;
pub const SHEET_H: usize = 342;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    px: Vec<bool>,
}

impl Bitmap {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![false; w * h] }
    }

    pub fn get(&self, x: i32, y: i32) -> bool {
        self.inside(x, y) && self.px[y as usize * self.w + x as usize]
    }

    pub fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    /// Set a pixel; outside the sheet is ignored.
    pub fn set(&mut self, x: i32, y: i32, v: bool) {
        if self.inside(x, y) {
            self.px[y as usize * self.w + x as usize] = v;
        }
    }

    pub fn clear(&mut self) {
        self.px.iter_mut().for_each(|p| *p = false);
    }

    pub fn invert(&mut self) {
        self.px.iter_mut().for_each(|p| *p = !*p);
    }

    pub fn ink_count(&self) -> usize {
        self.px.iter().filter(|p| **p).count()
    }

    /// Copy all pixels from a bitmap of the same size (rubber-banding restores the base this way).
    pub fn copy_from(&mut self, other: &Bitmap) {
        if self.w == other.w && self.h == other.h {
            self.px.copy_from_slice(&other.px);
        }
    }

    /// A square dab of `size` pixels centred on (x, y) (size 1 = one pixel).
    pub fn dab(&mut self, x: i32, y: i32, size: i32, v: bool) {
        let s = size.max(1);
        let (x0, y0) = (x - (s - 1) / 2, y - (s - 1) / 2);
        for yy in y0..y0 + s {
            for xx in x0..x0 + s {
                self.set(xx, yy, v);
            }
        }
    }

    /// Copy out a rectangle (clipped to the sheet) as its own bitmap.
    pub fn crop(&self, x: i32, y: i32, w: usize, h: usize) -> Bitmap {
        let mut out = Bitmap::new(w, h);
        for yy in 0..h as i32 {
            for xx in 0..w as i32 {
                out.set(xx, yy, self.get(x + xx, y + yy));
            }
        }
        out
    }

    /// Paste `src` with its top-left at (x, y). Paper pixels of `src` overwrite too unless `transparent`.
    pub fn paste(&mut self, src: &Bitmap, x: i32, y: i32, transparent: bool) {
        for yy in 0..src.h as i32 {
            for xx in 0..src.w as i32 {
                let v = src.get(xx, yy);
                if v || !transparent {
                    self.set(x + xx, y + yy, v);
                }
            }
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: usize, h: usize, v: bool) {
        for yy in y..y + h as i32 {
            for xx in x..x + w as i32 {
                self.set(xx, yy, v);
            }
        }
    }

    // ------------------------------------------------------------------ run-length snapshots

    /// Run lengths, alternating paper / ink starting with paper (the first run may be 0).
    pub fn encode(&self) -> Vec<u32> {
        let mut runs = Vec::new();
        let (mut cur, mut n) = (false, 0u32);
        for &p in &self.px {
            if p == cur {
                n += 1;
            } else {
                runs.push(n);
                cur = p;
                n = 1;
            }
        }
        runs.push(n);
        runs
    }

    pub fn decode(w: usize, h: usize, runs: &[u32]) -> Bitmap {
        let mut b = Bitmap::new(w, h);
        let (mut i, mut cur) = (0usize, false);
        for &r in runs {
            let end = (i + r as usize).min(b.px.len());
            if cur {
                b.px[i..end].iter_mut().for_each(|p| *p = true);
            }
            i = end;
            cur = !cur;
        }
        b
    }

    // ------------------------------------------------------------------------------- PNG

    /// A 1-bit greyscale PNG (ink = black = 0, paper = white = 1), stored with deflate "stored" blocks so
    /// no compression crate is needed; the files are small (a full sheet is 22 KB).
    pub fn to_png(&self) -> Vec<u8> {
        let row = (self.w + 7) / 8;
        let mut raw = Vec::with_capacity((row + 1) * self.h);
        for y in 0..self.h {
            raw.push(0); // filter: none
            let mut bytes = vec![0xFFu8; row];
            for x in 0..self.w {
                if self.px[y * self.w + x] {
                    bytes[x / 8] &= !(0x80 >> (x % 8));
                }
            }
            raw.extend(bytes);
        }
        let mut z = vec![0x78, 0x01];
        let mut chunks = raw.chunks(65535).peekable();
        if raw.is_empty() {
            z.extend([1, 0, 0, 0xFF, 0xFF]);
        }
        while let Some(ch) = chunks.next() {
            z.push(if chunks.peek().is_none() { 1 } else { 0 });
            z.extend((ch.len() as u16).to_le_bytes());
            z.extend((!(ch.len() as u16)).to_le_bytes());
            z.extend(ch);
        }
        z.extend(adler32(&raw).to_be_bytes());

        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend((self.w as u32).to_be_bytes());
        ihdr.extend((self.h as u32).to_be_bytes());
        ihdr.extend([1, 0, 0, 0, 0]); // 1 bit, greyscale, deflate, filter 0, no interlace
        push_chunk(&mut out, b"IHDR", &ihdr);
        push_chunk(&mut out, b"IDAT", &z);
        push_chunk(&mut out, b"IEND", &[]);
        out
    }

    /// Decode a PNG made by `to_png` (any 1-bit greyscale, non-interlaced PNG with stored or compressed
    /// data is NOT supported here; the app falls back to SDL_image for those, see `from_rgb_luma`).
    pub fn from_png(data: &[u8]) -> Result<Bitmap, String> {
        if data.len() < 8 || data[..8] != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
            return Err("not a PNG".into());
        }
        let (mut pos, mut w, mut h, mut idat) = (8usize, 0usize, 0usize, Vec::new());
        while pos + 8 <= data.len() {
            let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &data[pos + 4..pos + 8];
            let body = data.get(pos + 8..pos + 8 + len).ok_or("truncated")?;
            match kind {
                b"IHDR" => {
                    w = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                    h = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                    if body[8] != 1 || body[9] != 0 || body[12] != 0 {
                        return Err("not a plain 1-bit PNG".into());
                    }
                }
                b"IDAT" => idat.extend_from_slice(body),
                _ => {}
            }
            pos += 12 + len;
        }
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return Err("bad size".into());
        }
        let raw = inflate_stored(&idat)?;
        let row = (w + 7) / 8;
        if raw.len() < (row + 1) * h {
            return Err("short image data".into());
        }
        let mut b = Bitmap::new(w, h);
        for y in 0..h {
            if raw[y * (row + 1)] != 0 {
                return Err("filtered rows are not supported".into());
            }
            for x in 0..w {
                let bit = raw[y * (row + 1) + 1 + x / 8] & (0x80 >> (x % 8));
                b.px[y * w + x] = bit == 0;
            }
        }
        Ok(b)
    }

    /// Threshold tightly packed RGB into a bitmap (dark = ink), for pictures SDL_image decoded.
    pub fn from_rgb_luma(rgb: &[u8], w: usize, h: usize) -> Bitmap {
        let mut b = Bitmap::new(w, h);
        for i in 0..w * h {
            let l = (299 * rgb[i * 3] as u32 + 587 * rgb[i * 3 + 1] as u32 + 114 * rgb[i * 3 + 2] as u32) / 1000;
            b.px[i] = l < 128;
        }
        b
    }
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &d in data {
        a = (a + d as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend((body.len() as u32).to_be_bytes());
    let mut c = kind.to_vec();
    c.extend(body);
    out.extend(&c);
    out.extend(crc32(&c).to_be_bytes());
}

/// Read a zlib stream made only of stored blocks (what `to_png` writes).
fn inflate_stored(z: &[u8]) -> Result<Vec<u8>, String> {
    if z.len() < 2 {
        return Err("empty data".into());
    }
    let (mut i, mut out) = (2usize, Vec::new());
    loop {
        let hdr = *z.get(i).ok_or("truncated")?;
        if hdr & 6 != 0 {
            return Err("compressed PNG data is not supported".into());
        }
        let len = u16::from_le_bytes([*z.get(i + 1).ok_or("truncated")?, *z.get(i + 2).ok_or("truncated")?]) as usize;
        out.extend_from_slice(z.get(i + 5..i + 5 + len).ok_or("truncated")?);
        i += 5 + len;
        if hdr & 1 != 0 {
            return Ok(out);
        }
    }
}

// ------------------------------------------------------------------------------ patterns

/// The eight fill patterns, each an 8 x 8 tile; a set bit is ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern(pub u8);

pub const PATTERN_COUNT: u8 = 8;

const TILES: [[u8; 8]; 8] = [
    [0x00; 8],                                                         // 0 white
    [0x80, 0x00, 0x08, 0x00, 0x80, 0x00, 0x08, 0x00],                  // 1 sparse dots (12%)
    [0x88, 0x00, 0x22, 0x00, 0x88, 0x00, 0x22, 0x00],                  // 2 light (~12/25%)
    [0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55],                  // 3 gray (50%)
    [0xEE, 0xBB, 0xEE, 0xBB, 0xEE, 0xBB, 0xEE, 0xBB],                  // 4 dark (75%)
    [0xFF; 8],                                                         // 5 black
    [0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00],                  // 6 horizontal stripes
    [0x88, 0x44, 0x22, 0x11, 0x88, 0x44, 0x22, 0x11],                  // 7 diagonal
];

impl Pattern {
    pub fn lit(self, x: i32, y: i32) -> bool {
        let t = TILES[(self.0 % PATTERN_COUNT) as usize];
        t[y.rem_euclid(8) as usize] & (0x80 >> x.rem_euclid(8)) != 0
    }

    pub fn density(self) -> f32 {
        let t = TILES[(self.0 % PATTERN_COUNT) as usize];
        t.iter().map(|b| b.count_ones()).sum::<u32>() as f32 / 64.0
    }

    pub fn name(self) -> &'static str {
        ["White", "Sparse", "Light", "Gray", "Dark", "Black", "Stripes", "Diagonal"][(self.0 % PATTERN_COUNT) as usize]
    }
}

// ------------------------------------------------------------------------------ primitives

/// Bresenham: every pixel from (x0, y0) to (x1, y1) inclusive.
pub fn line_points(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    let mut out = Vec::new();
    loop {
        out.push((x, y));
        if x == x1 && y == y1 {
            return out;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Constrain a drag from (x0, y0) to a multiple of 45 degrees (Shift on a line).
pub fn snap45(x0: i32, y0: i32, x1: i32, y1: i32) -> (i32, i32) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (ax, ay) = (dx.abs(), dy.abs());
    if ay * 2 < ax {
        (x1, y0)
    } else if ax * 2 < ay {
        (x0, y1)
    } else {
        let d = ax.max(ay);
        (x0 + d * dx.signum(), y0 + d * dy.signum())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    RoundRect,
    Ellipse,
}

/// Normalised box with inclusive corners.
pub fn bbox(x0: i32, y0: i32, x1: i32, y1: i32) -> (i32, i32, i32, i32) {
    (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
}

/// Is (x, y) inside the filled shape whose bounding box is (bx0..=bx1, by0..=by1)?
pub fn shape_inside(shape: Shape, b: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    let (bx0, by0, bx1, by1) = b;
    if x < bx0 || x > bx1 || y < by0 || y > by1 {
        return false;
    }
    let (w, h) = ((bx1 - bx0 + 1) as i64, (by1 - by0 + 1) as i64);
    let (px, py) = ((x - bx0) as i64, (y - by0) as i64);
    match shape {
        Shape::Rect => true,
        Shape::Ellipse => {
            // Pixel centres against the ellipse inscribed in the box.
            let (u, v) = (2 * px + 1 - w, 2 * py + 1 - h);
            u * u * h * h + v * v * w * w <= w * w * h * h
        }
        Shape::RoundRect => {
            let r = (w.min(h) / 2).min(8);
            let cx = if px < r { r } else if px >= w - r { w - 1 - r } else { px };
            let cy = if py < r { r } else if py >= h - r { h - 1 - r } else { py };
            let (dx, dy) = (px - cx, py - cy);
            // Corner circle of radius r + 0.5 about the corner pixel centre.
            4 * (dx * dx + dy * dy) <= (2 * r + 1) * (2 * r + 1)
        }
    }
}

/// Thin outline pixels of a shape: inside pixels that touch an outside one (4-neighbourhood).
pub fn shape_outline(shape: Shape, b: (i32, i32, i32, i32)) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in b.1..=b.3 {
        for x in b.0..=b.2 {
            if shape_inside(shape, b, x, y)
                && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| !shape_inside(shape, b, x + dx, y + dy))
            {
                out.push((x, y));
            }
        }
    }
    out
}

/// Draw a shape. `fill` = Some(pattern) fills the inside with that pattern (ink where the pattern is lit,
/// paper elsewhere); the outline is stamped with a `size` square in `ink`.
pub fn draw_shape(bm: &mut Bitmap, shape: Shape, b: (i32, i32, i32, i32), fill: Option<Pattern>, size: i32, ink: bool) {
    if let Some(p) = fill {
        for y in b.1..=b.3 {
            for x in b.0..=b.2 {
                if shape_inside(shape, b, x, y) {
                    bm.set(x, y, p.lit(x, y) == ink);
                }
            }
        }
    }
    for (x, y) in shape_outline(shape, b) {
        bm.dab(x, y, size, ink);
    }
}

pub fn draw_line(bm: &mut Bitmap, x0: i32, y0: i32, x1: i32, y1: i32, size: i32, ink: bool) {
    for (x, y) in line_points(x0, y0, x1, y1) {
        bm.dab(x, y, size, ink);
    }
}

/// Flood fill: every pixel connected (4-way) to (x, y) with the same value is replaced by the pattern
/// (lit = ink). Returns how many pixels were in the region (0 when (x, y) is outside the sheet).
pub fn flood_fill(bm: &mut Bitmap, x: i32, y: i32, pat: Pattern) -> usize {
    if !bm.inside(x, y) {
        return 0;
    }
    let target = bm.get(x, y);
    let mut seen = vec![false; bm.w * bm.h];
    let mut region = Vec::new();
    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        if !bm.inside(sx, sy) || seen[sy as usize * bm.w + sx as usize] || bm.get(sx, sy) != target {
            continue;
        }
        // Scan the whole run, then queue the rows above and below.
        let (mut l, mut r) = (sx, sx);
        while bm.inside(l - 1, sy) && bm.get(l - 1, sy) == target && !seen[sy as usize * bm.w + (l - 1) as usize] {
            l -= 1;
        }
        while bm.inside(r + 1, sy) && bm.get(r + 1, sy) == target && !seen[sy as usize * bm.w + (r + 1) as usize] {
            r += 1;
        }
        for xx in l..=r {
            seen[sy as usize * bm.w + xx as usize] = true;
            region.push((xx, sy));
            stack.push((xx, sy - 1));
            stack.push((xx, sy + 1));
        }
    }
    let n = region.len();
    for (xx, yy) in region {
        bm.set(xx, yy, pat.lit(xx, yy));
    }
    n
}

/// Deterministic spray: `dots` pixels scattered in a disc of `radius` around (x, y). `seed` advances.
pub fn spray(bm: &mut Bitmap, x: i32, y: i32, radius: i32, dots: usize, ink: bool, seed: &mut u32) {
    let mut next = || {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (*seed >> 8) as i32
    };
    let span = radius * 2 + 1;
    let mut placed = 0;
    let mut tries = 0;
    while placed < dots && tries < dots * 8 {
        tries += 1;
        let (dx, dy) = (next().rem_euclid(span) - radius, next().rem_euclid(span) - radius);
        if dx * dx + dy * dy <= radius * radius {
            bm.set(x + dx, y + dy, ink);
            placed += 1;
        }
    }
}

/// Undo / redo history: snapshots as run-length data, at most `cap` kept.
pub struct History {
    cap: usize,
    undo: Vec<Vec<u32>>,
    redo: Vec<Vec<u32>>,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self { cap, undo: Vec::new(), redo: Vec::new() }
    }

    /// Remember the state *before* a change. Clears the redo stack.
    pub fn push(&mut self, before: &Bitmap) {
        self.undo.push(before.encode());
        if self.undo.len() > self.cap {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Step back: returns the previous state, remembering `current` for redo.
    pub fn undo(&mut self, current: &Bitmap) -> Option<Bitmap> {
        let runs = self.undo.pop()?;
        self.redo.push(current.encode());
        Some(Bitmap::decode(current.w, current.h, &runs))
    }

    pub fn redo(&mut self, current: &Bitmap) -> Option<Bitmap> {
        let runs = self.redo.pop()?;
        self.undo.push(current.encode());
        Some(Bitmap::decode(current.w, current.h, &runs))
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_bounds() {
        let mut b = Bitmap::new(10, 8);
        b.set(3, 4, true);
        b.set(-1, 0, true);
        b.set(10, 0, true);
        assert!(b.get(3, 4) && !b.get(4, 4) && !b.get(-1, 0) && !b.get(99, 99));
        assert_eq!(b.ink_count(), 1);
    }

    #[test]
    fn lines_hit_their_end_points_and_stay_connected() {
        for &(x0, y0, x1, y1) in &[(0, 0, 9, 3), (5, 5, 5, 0), (2, 8, 8, 8), (9, 9, 0, 0), (0, 5, 3, 0)] {
            let pts = line_points(x0, y0, x1, y1);
            assert_eq!(pts.first(), Some(&(x0, y0)));
            assert_eq!(pts.last(), Some(&(x1, y1)));
            for w in pts.windows(2) {
                assert!((w[0].0 - w[1].0).abs() <= 1 && (w[0].1 - w[1].1).abs() <= 1, "gap in {w:?}");
            }
            assert_eq!(pts.len() as i32, (x1 - x0).abs().max((y1 - y0).abs()) + 1);
        }
    }

    #[test]
    fn snap45_picks_horizontal_vertical_or_diagonal() {
        assert_eq!(snap45(0, 0, 10, 2), (10, 0));
        assert_eq!(snap45(0, 0, 2, 10), (0, 10));
        assert_eq!(snap45(0, 0, 10, 8), (10, 10));
        assert_eq!(snap45(5, 5, 0, 1), (0, 0));
    }

    #[test]
    fn dab_sizes() {
        let mut b = Bitmap::new(20, 20);
        b.dab(10, 10, 1, true);
        assert_eq!(b.ink_count(), 1);
        b.clear();
        b.dab(10, 10, 4, true);
        assert_eq!(b.ink_count(), 16);
        b.dab(10, 10, 4, false);
        assert_eq!(b.ink_count(), 0);
    }

    #[test]
    fn ellipse_is_symmetric_and_touches_the_box() {
        let b = (2, 3, 21, 14);
        let pts = shape_outline(Shape::Ellipse, b);
        assert!(pts.iter().any(|p| p.0 == 2) && pts.iter().any(|p| p.0 == 21));
        assert!(pts.iter().any(|p| p.1 == 3) && pts.iter().any(|p| p.1 == 14));
        for &(x, y) in &pts {
            assert!(pts.contains(&(2 + 21 - x, y)), "mirror in x of ({x},{y})");
            assert!(pts.contains(&(x, 3 + 14 - y)), "mirror in y of ({x},{y})");
        }
        // A circle's filled area is close to pi r^2.
        let area = (0..40).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|&(x, y)| shape_inside(Shape::Ellipse, (0, 0, 19, 19), x, y)).count();
        assert!((300..=330).contains(&area), "area {area}");
    }

    #[test]
    fn rect_outline_is_the_frame_and_round_rect_cuts_corners() {
        let o = shape_outline(Shape::Rect, (1, 1, 6, 4));
        assert_eq!(o.len(), 6 * 2 + 2 * 2);
        assert!(!shape_inside(Shape::RoundRect, (0, 0, 19, 19), 0, 0));
        assert!(shape_inside(Shape::RoundRect, (0, 0, 19, 19), 10, 0));
        assert!(shape_inside(Shape::RoundRect, (0, 0, 19, 19), 0, 10));
    }

    #[test]
    fn draw_shape_outline_and_pattern_fill() {
        let mut b = Bitmap::new(20, 20);
        draw_shape(&mut b, Shape::Rect, (2, 2, 11, 11), Some(Pattern(5)), 1, true);
        assert_eq!(b.ink_count(), 100, "black fill covers the box");
        let mut b = Bitmap::new(20, 20);
        draw_shape(&mut b, Shape::Rect, (2, 2, 11, 11), None, 1, true);
        assert_eq!(b.ink_count(), 36);
        assert!(!b.get(5, 5));
    }

    #[test]
    fn patterns_have_the_expected_density() {
        let d: Vec<f32> = (0..PATTERN_COUNT).map(|i| Pattern(i).density()).collect();
        assert_eq!(d[0], 0.0);
        assert_eq!(d[5], 1.0);
        assert_eq!(d[3], 0.5);
        assert_eq!(d[6], 0.5);
        assert_eq!(d[7], 0.25);
        assert!(d[1] < d[2] && d[2] < d[3] && d[3] < d[4]);
        assert!(Pattern(3).lit(0, 0) != Pattern(3).lit(1, 0), "gray is a checkerboard");
        assert_eq!(Pattern(3).lit(8, 8), Pattern(3).lit(0, 0), "tiles repeat");
    }

    #[test]
    fn flood_fill_stays_inside_walls_and_applies_the_pattern() {
        let mut b = Bitmap::new(12, 12);
        draw_shape(&mut b, Shape::Rect, (2, 2, 9, 9), None, 1, true);
        let n = flood_fill(&mut b, 5, 5, Pattern(5));
        assert_eq!(n, 36, "the 6x6 inside");
        assert!(b.get(5, 5) && !b.get(0, 0) && !b.get(11, 11));
        // Filling the outside does not touch the inside.
        let mut b2 = Bitmap::new(12, 12);
        draw_shape(&mut b2, Shape::Rect, (2, 2, 9, 9), None, 1, true);
        flood_fill(&mut b2, 0, 0, Pattern(3));
        assert!(!b2.get(5, 5) && b2.get(0, 0) != b2.get(1, 0));
        // A white fill over a black area clears it; the fill over its own colour terminates.
        flood_fill(&mut b, 5, 5, Pattern(0));
        assert!(!b.get(5, 5));
        let mut b3 = Bitmap::new(8, 8);
        flood_fill(&mut b3, 0, 0, Pattern(0));
        assert_eq!(b3.ink_count(), 0);
        assert_eq!(flood_fill(&mut b3, 99, 0, Pattern(5)), 0);
    }

    #[test]
    fn rle_round_trips_and_is_small_for_simple_pictures() {
        let mut b = Bitmap::new(SHEET_W, SHEET_H);
        assert_eq!(b.encode(), vec![(SHEET_W * SHEET_H) as u32]);
        draw_shape(&mut b, Shape::Ellipse, (10, 10, 200, 100), Some(Pattern(3)), 2, true);
        draw_line(&mut b, 0, 0, 500, 300, 3, true);
        let runs = b.encode();
        assert_eq!(runs.iter().map(|r| *r as usize).sum::<usize>(), SHEET_W * SHEET_H);
        assert_eq!(Bitmap::decode(SHEET_W, SHEET_H, &runs), b);
        let ink_first = {
            let mut c = Bitmap::new(4, 1);
            c.set(0, 0, true);
            c
        };
        assert_eq!(ink_first.encode(), vec![0, 1, 3]);
        assert_eq!(Bitmap::decode(4, 1, &[0, 1, 3]), ink_first);
    }

    #[test]
    fn history_undoes_redoes_and_caps() {
        let mut h = History::new(3);
        let mut b = Bitmap::new(8, 8);
        for i in 0..5 {
            h.push(&b);
            b.set(i, 0, true);
        }
        assert_eq!(h.len(), 3, "oldest steps are dropped");
        let b4 = h.undo(&b).unwrap();
        assert_eq!(b4.ink_count(), 4);
        let b3 = h.undo(&b4).unwrap();
        assert_eq!(b3.ink_count(), 3);
        let again = h.redo(&b3).unwrap();
        assert_eq!(again, b4);
        h.push(&again);
        assert!(!h.can_redo(), "a new change clears redo");
        let mut empty = History::new(2);
        assert!(empty.undo(&b).is_none() && empty.redo(&b).is_none());
    }

    #[test]
    fn png_round_trip() {
        let mut b = Bitmap::new(37, 19);
        draw_shape(&mut b, Shape::Ellipse, (1, 1, 30, 15), Some(Pattern(4)), 1, true);
        let png = b.to_png();
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(Bitmap::from_png(&png).unwrap(), b);
        let big = {
            let mut x = Bitmap::new(SHEET_W, SHEET_H);
            draw_line(&mut x, 0, 0, 511, 341, 5, true);
            x
        };
        assert_eq!(Bitmap::from_png(&big.to_png()).unwrap(), big, "more than one stored block");
        assert!(Bitmap::from_png(b"nope").is_err());
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn spray_stays_in_the_disc_and_is_repeatable() {
        let (mut a, mut b) = (Bitmap::new(40, 40), Bitmap::new(40, 40));
        let (mut s1, mut s2) = (7u32, 7u32);
        spray(&mut a, 20, 20, 6, 30, true, &mut s1);
        spray(&mut b, 20, 20, 6, 30, true, &mut s2);
        assert_eq!(a, b);
        assert!(a.ink_count() > 5);
        for y in 0..40 {
            for x in 0..40 {
                if a.get(x, y) {
                    assert!((x - 20) * (x - 20) + (y - 20) * (y - 20) <= 36);
                }
            }
        }
    }

    #[test]
    fn crop_and_paste() {
        let mut b = Bitmap::new(10, 10);
        b.fill_rect(2, 2, 3, 3, true);
        let c = b.crop(2, 2, 3, 3);
        assert_eq!(c.ink_count(), 9);
        b.paste(&c, 6, 6, false);
        assert_eq!(b.ink_count(), 18);
        b.fill_rect(2, 2, 3, 3, false);
        b.paste(&Bitmap::new(3, 3), 6, 6, true);
        assert_eq!(b.ink_count(), 9, "a transparent paste of paper changes nothing");
    }
}
