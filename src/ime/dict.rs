//! Binary dictionary for the input method: format, builder, and a memory-mapped reader.
//!
//! The dictionary is **never part of this repository or of the release bundles**. The user is asked at
//! install time whether to download one (see `packaging/ime_install.sh`); the generator binary
//! (`harbour_ime_dict`) converts the downloaded text into this format and the text is deleted.
//!
//! Layout (little endian):
//! ```text
//! header (32 bytes): "HIME" | version u32 | nkeys u32 | index_off u32 | pool_off u32 | total_len u32 | fnv1a32(pool) u32 | 0
//! index  (nkeys * 4): u32 offset of each record, relative to pool_off, sorted by key bytes
//! pool   : record = klen u8 | key utf-8 | n u8 | n * (cost u16 | slen u8 | surface utf-8)   (candidates cheapest first)
//! ```
//! Lookups are a binary search over the index, so only the touched pages are ever resident (the file is
//! mapped, not read). Every access is bounds-checked: a damaged file can make lookups return nothing,
//! never panic or read outside the mapping.

use std::fs::File;
use std::path::Path;

pub const MAGIC: &[u8; 4] = b"HIME";
pub const VERSION: u32 = 1;
const HEADER: usize = 32;

/// One candidate for a reading. Lower cost = more likely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictCand {
    pub cost: u16,
    pub surface: String,
}

enum Data {
    Mapped { ptr: *mut libc::c_void, len: usize },
    Owned(Vec<u8>),
}

// The mapping is read-only and never modified after creation.
unsafe impl Send for Data {}
unsafe impl Sync for Data {}

impl Data {
    fn bytes(&self) -> &[u8] {
        match self {
            // SAFETY: ptr/len come from a successful read-only mmap that lives until Drop.
            Data::Mapped { ptr, len } => unsafe { std::slice::from_raw_parts(*ptr as *const u8, *len) },
            Data::Owned(v) => v,
        }
    }
}

impl Drop for Data {
    fn drop(&mut self) {
        if let Data::Mapped { ptr, len } = self {
            // SAFETY: matches the mmap in `Dict::open`.
            unsafe {
                libc::munmap(*ptr, *len);
            }
        }
    }
}

pub struct Dict {
    data: Data,
    nkeys: usize,
    index_off: usize,
    pool_off: usize,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl Dict {
    /// Map `path`. Checks the header and sizes only (not the checksum: that would read the whole file
    /// on every start); see [`Dict::verify`].
    pub fn open(path: &Path) -> Result<Dict, String> {
        let f = File::open(path).map_err(|e| format!("open: {e}"))?;
        let len = f.metadata().map_err(|e| format!("stat: {e}"))?.len() as usize;
        if len < HEADER {
            return Err("file too short".into());
        }
        #[cfg(unix)]
        let data = {
            use std::os::unix::io::AsRawFd;
            // SAFETY: read-only private mapping of a file we just opened; unmapped in Drop.
            let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, f.as_raw_fd(), 0) };
            if ptr == libc::MAP_FAILED {
                return Err("mmap failed".into());
            }
            Data::Mapped { ptr, len }
        };
        #[cfg(not(unix))]
        let data = Data::Owned(std::fs::read(path).map_err(|e| format!("read: {e}"))?);
        Self::from_data(data)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Dict, String> {
        Self::from_data(Data::Owned(bytes))
    }

    fn from_data(data: Data) -> Result<Dict, String> {
        let b = data.bytes();
        if b.len() < HEADER || &b[..4] != MAGIC {
            return Err("not a Harbour IME dictionary".into());
        }
        if u32_at(b, 4) != Some(VERSION) {
            return Err("unsupported dictionary version".into());
        }
        let nkeys = u32_at(b, 8).ok_or("header")? as usize;
        let index_off = u32_at(b, 12).ok_or("header")? as usize;
        let pool_off = u32_at(b, 16).ok_or("header")? as usize;
        let total = u32_at(b, 20).ok_or("header")? as usize;
        if total != b.len() {
            return Err("size mismatch (truncated file?)".into());
        }
        let index_end = nkeys.checked_mul(4).and_then(|n| n.checked_add(index_off)).ok_or("header")?;
        if index_off < HEADER || index_end > pool_off || pool_off > b.len() {
            return Err("bad section table".into());
        }
        Ok(Dict { data, nkeys, index_off, pool_off })
    }

    /// Full integrity check (reads every page): used by the installer right after generation.
    pub fn verify(&self) -> bool {
        let b = self.data.bytes();
        u32_at(b, 24) == Some(fnv1a32(&b[self.pool_off..]))
    }

    pub fn len(&self) -> usize {
        self.nkeys
    }

    pub fn is_empty(&self) -> bool {
        self.nkeys == 0
    }

    pub fn file_size(&self) -> usize {
        self.data.bytes().len()
    }

    /// The record start for index slot `i` (absolute offset into the file).
    fn record(&self, i: usize) -> Option<usize> {
        let off = u32_at(self.data.bytes(), self.index_off + i * 4)? as usize;
        self.pool_off.checked_add(off)
    }

    fn key_at(&self, rec: usize) -> Option<&[u8]> {
        let b = self.data.bytes();
        let klen = *b.get(rec)? as usize;
        b.get(rec + 1..rec + 1 + klen)
    }

    fn find(&self, key: &[u8]) -> Option<usize> {
        let (mut lo, mut hi) = (0usize, self.nkeys);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let rec = self.record(mid)?;
            match self.key_at(rec)?.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(rec),
            }
        }
        None
    }

    /// Candidates for exactly `key` (hiragana), cheapest first. Empty when absent.
    pub fn lookup(&self, key: &str) -> Vec<DictCand> {
        self.try_lookup(key).unwrap_or_default()
    }

    fn try_lookup(&self, key: &str) -> Option<Vec<DictCand>> {
        let rec = self.find(key.as_bytes())?;
        let b = self.data.bytes();
        let mut p = rec + 1 + *b.get(rec)? as usize;
        let n = *b.get(p)? as usize;
        p += 1;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let cost = u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]);
            let slen = *b.get(p + 2)? as usize;
            let s = std::str::from_utf8(b.get(p + 3..p + 3 + slen)?).ok()?;
            out.push(DictCand { cost, surface: s.to_string() });
            p += 3 + slen;
        }
        Some(out)
    }
}

// ---- building -------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct BuildOpts {
    /// Entries costlier than this are dropped (rare words).
    pub max_cost: u32,
    /// Candidates kept per reading.
    pub per_key: usize,
}

impl Default for BuildOpts {
    fn default() -> Self {
        Self { max_cost: 9500, per_key: 10 }
    }
}

/// Can the 12x12 Shinonome font (JIS X 0208) plus the Latin font draw `c`?
pub fn displayable(c: char) -> bool {
    c.is_ascii_graphic() || c == '\u{3000}' || crate::jis::unicode_to_jis(c).is_some()
}

fn reading_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && s.chars().all(|c| ('\u{3041}'..='\u{3093}').contains(&c) || c == 'ー')
}

/// One line of the Mozc OSS dictionary text: `reading TAB left_id TAB right_id TAB cost TAB surface`.
pub fn parse_mozc_line(line: &str) -> Option<(&str, u32, &str)> {
    let mut it = line.split('\t');
    let reading = it.next()?;
    it.next()?;
    it.next()?;
    let cost = it.next()?.trim().parse::<u32>().ok()?;
    let surface = it.next()?;
    Some((reading, cost, surface))
}

/// Accumulates entries, filters them for the device, and writes the binary format.
///
/// Memory matters here: the generator runs on the device (352 MB RAM, ~120 MB free). Entries are kept in
/// one byte arena (`reading_len u8 | reading | cost u16 | surface_len u8 | surface`) with a `u32` offset
/// list that is sorted in place, instead of a map of strings, and the output is streamed to the file.
#[derive(Default)]
pub struct Builder {
    arena: Vec<u8>,
    offsets: Vec<u32>,
    pub opts: BuildOpts,
    pub dropped: usize,
}

impl Builder {
    pub fn new(opts: BuildOpts) -> Self {
        Self { arena: Vec::new(), offsets: Vec::new(), opts, dropped: 0 }
    }

    pub fn add(&mut self, reading: &str, cost: u32, surface: &str) {
        if cost > self.opts.max_cost
            || !reading_ok(reading)
            || surface.is_empty()
            || surface.len() > 120
            || !surface.chars().all(displayable)
            || self.arena.len() > u32::MAX as usize - 512
        {
            self.dropped += 1;
            return;
        }
        self.offsets.push(self.arena.len() as u32);
        self.arena.push(reading.len() as u8);
        self.arena.extend_from_slice(reading.as_bytes());
        self.arena.extend_from_slice(&(cost.min(u16::MAX as u32) as u16).to_le_bytes());
        self.arena.push(surface.len() as u8);
        self.arena.extend_from_slice(surface.as_bytes());
    }

    pub fn add_mozc_text(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((r, c, s)) = parse_mozc_line(line) {
                self.add(r, c, s);
            }
        }
    }

    /// Entries accepted so far (before de-duplication).
    pub fn entry_count(&self) -> usize {
        self.offsets.len()
    }

    fn parts(&self, off: u32) -> (&[u8], u16, &[u8]) {
        let a = &self.arena;
        let o = off as usize;
        let rl = a[o] as usize;
        let reading = &a[o + 1..o + 1 + rl];
        let c = o + 1 + rl;
        let cost = u16::from_le_bytes([a[c], a[c + 1]]);
        let sl = a[c + 2] as usize;
        (reading, cost, &a[c + 3..c + 3 + sl])
    }

    /// Sort, merge duplicates (cheapest wins), keep `per_key` per reading and return the index and pool.
    fn pack(&mut self, pool_base: usize) -> (u32, Vec<u8>, Vec<u8>) {
        let mut offs = std::mem::take(&mut self.offsets);
        offs.sort_unstable_by(|&x, &y| {
            let (rx, cx, sx) = self.parts(x);
            let (ry, cy, sy) = self.parts(y);
            rx.cmp(ry).then(cx.cmp(&cy)).then(sx.cmp(sy))
        });
        let per_key = self.opts.per_key.clamp(1, 255);
        let mut index: Vec<u8> = Vec::new();
        let mut pool: Vec<u8> = Vec::new();
        let mut nkeys = 0u32;
        let mut i = 0;
        while i < offs.len() {
            let key = self.parts(offs[i]).0;
            let mut j = i;
            while j < offs.len() && self.parts(offs[j]).0 == key {
                j += 1;
            }
            // Entries of one reading are sorted by cost: drop repeats of a surface (the cheaper came first).
            let mut kept: Vec<(u16, &[u8])> = Vec::new();
            for &o in &offs[i..j] {
                let (_, c, s) = self.parts(o);
                if kept.len() < per_key && !kept.iter().any(|k| k.1 == s) {
                    kept.push((c, s));
                }
            }
            index.extend_from_slice(&((pool_base + pool.len()) as u32).to_le_bytes());
            pool.push(key.len() as u8);
            pool.extend_from_slice(key);
            pool.push(kept.len() as u8);
            for (c, s) in kept {
                pool.extend_from_slice(&c.to_le_bytes());
                pool.push(s.len() as u8);
                pool.extend_from_slice(s);
            }
            nkeys += 1;
            i = j;
        }
        self.arena = Vec::new();
        (nkeys, index, pool)
    }

    /// Write the dictionary to `w`; returns (readings, bytes written).
    pub fn finish_to(mut self, w: &mut impl std::io::Write) -> std::io::Result<(usize, usize)> {
        let (nkeys, index, pool) = self.pack(0);
        let pool_off = HEADER + index.len();
        let total = pool_off + pool.len();
        let sum = fnv1a32(&pool);
        let mut head = Vec::with_capacity(HEADER);
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&VERSION.to_le_bytes());
        head.extend_from_slice(&nkeys.to_le_bytes());
        head.extend_from_slice(&(HEADER as u32).to_le_bytes());
        head.extend_from_slice(&(pool_off as u32).to_le_bytes());
        head.extend_from_slice(&(total as u32).to_le_bytes());
        head.extend_from_slice(&sum.to_le_bytes());
        head.extend_from_slice(&[0; 4]);
        w.write_all(&head)?;
        w.write_all(&index)?;
        w.write_all(&pool)?;
        Ok((nkeys as usize, total))
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        self.finish_to(&mut out).expect("writing to a Vec cannot fail");
        out
    }
}

/// Boundaries (first character of the reading) that split the Mozc data into ranges built one at a time,
/// so the generator never holds the whole dictionary in memory. Concatenating the ranges in order keeps
/// the keys sorted, because UTF-8 byte order is code point order.
const BUCKETS: &[char] = &['か', 'さ', 'た', 'は', 'ま', 'ら'];

fn bucket_of(reading: &str) -> usize {
    let c = reading.chars().next().unwrap_or('\0');
    BUCKETS.iter().take_while(|&&b| c >= b).count()
}

/// Build `out` from Mozc OSS dictionary text files, using about a sixth of the memory of building at once.
/// Returns (readings, bytes, entries dropped by the filters).
pub fn build_from_mozc_files(inputs: &[std::path::PathBuf], opts: &BuildOpts, out: &Path) -> Result<(usize, usize, usize), String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let tmp_pool = out.with_extension("pool.tmp");
    let tmp_out = out.with_extension("tmp");
    let r = (|| -> Result<(usize, usize, usize), String> {
        let mut pool_file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&tmp_pool).map_err(|e| format!("{}: {e}", tmp_pool.display()))?;
        let mut index: Vec<u8> = Vec::new();
        let (mut nkeys, mut pool_len, mut dropped) = (0usize, 0usize, 0usize);
        let mut sum = 0x811c9dc5u32;
        for bucket in 0..=BUCKETS.len() {
            let mut b = Builder::new(opts.clone());
            for f in inputs {
                let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
                for line in text.lines() {
                    if let Some((r, c, s)) = parse_mozc_line(line) {
                        if bucket_of(r) == bucket {
                            b.add(r, c, s);
                        }
                    }
                }
            }
            dropped += b.dropped;
            let (n, idx, pool) = b.pack(pool_len);
            nkeys += n as usize;
            index.extend_from_slice(&idx);
            for &byte in &pool {
                sum ^= byte as u32;
                sum = sum.wrapping_mul(0x0100_0193);
            }
            pool_file.write_all(&pool).map_err(|e| e.to_string())?;
            pool_len += pool.len();
        }
        if nkeys == 0 {
            return Err("no usable entries (wrong input files?)".into());
        }
        pool_file.flush().map_err(|e| e.to_string())?;
        pool_file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let pool_off = HEADER + index.len();
        let total = pool_off + pool_len;
        if total > u32::MAX as usize {
            return Err("dictionary too large".into());
        }
        let mut head = Vec::with_capacity(HEADER);
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&VERSION.to_le_bytes());
        head.extend_from_slice(&(nkeys as u32).to_le_bytes());
        head.extend_from_slice(&(HEADER as u32).to_le_bytes());
        head.extend_from_slice(&(pool_off as u32).to_le_bytes());
        head.extend_from_slice(&(total as u32).to_le_bytes());
        head.extend_from_slice(&sum.to_le_bytes());
        head.extend_from_slice(&[0; 4]);
        let mut w = std::io::BufWriter::new(File::create(&tmp_out).map_err(|e| format!("{}: {e}", tmp_out.display()))?);
        w.write_all(&head).map_err(|e| e.to_string())?;
        w.write_all(&index).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = pool_file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        w.flush().map_err(|e| e.to_string())?;
        drop(w);
        // Check what we wrote before it replaces anything.
        let d = Dict::open(&tmp_out)?;
        if !d.verify() || d.len() != nkeys {
            return Err("generated file failed its own check".into());
        }
        drop(d);
        std::fs::rename(&tmp_out, out).map_err(|e| e.to_string())?;
        Ok((nkeys, total, dropped))
    })();
    let _ = std::fs::remove_file(&tmp_pool);
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp_out);
    }
    r
}
