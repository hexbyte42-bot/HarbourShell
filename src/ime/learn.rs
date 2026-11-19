//! Learning history: which surface the user picked for a reading.
//!
//! File: `reading TAB surface TAB count TAB last_used_unix` per line (UTF-8, `~/.local/share/harbour/ime/learn`).
//! Lines that do not parse are skipped, so a damaged file loses only those lines. Writes are atomic
//! (`tmp` + rename) and are done by the launcher after a quiet period, not on every commit, to spare the SD card.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const MAX_ENTRIES: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearnEntry {
    pub surface: String,
    pub count: u32,
    pub last: u64,
}

#[derive(Default)]
pub struct Learn {
    map: HashMap<String, Vec<LearnEntry>>,
    path: Option<PathBuf>,
    dirty: bool,
    total: usize,
}

impl Learn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load `path` (a missing file is an empty history). `keep` filters surfaces the font cannot draw.
    pub fn load(path: &Path, keep: impl Fn(&str) -> bool) -> Self {
        let mut l = Learn { path: Some(path.to_path_buf()), ..Default::default() };
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let mut it = line.split('\t');
                let (Some(r), Some(s), Some(c), Some(t)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
                let (Ok(count), Ok(last)) = (c.parse::<u32>(), t.parse::<u64>()) else { continue };
                if r.is_empty() || s.is_empty() || !keep(s) {
                    continue;
                }
                l.map.entry(r.to_string()).or_default().push(LearnEntry { surface: s.to_string(), count, last });
                l.total += 1;
            }
        }
        l
    }

    pub fn get(&self, reading: &str) -> &[LearnEntry] {
        self.map.get(reading).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn record(&mut self, reading: &str, surface: &str, now: u64) {
        let v = self.map.entry(reading.to_string()).or_default();
        if let Some(e) = v.iter_mut().find(|e| e.surface == surface) {
            e.count = e.count.saturating_add(1);
            e.last = now;
        } else {
            v.push(LearnEntry { surface: surface.to_string(), count: 1, last: now });
            self.total += 1;
        }
        self.dirty = true;
        if self.total > MAX_ENTRIES {
            self.prune();
        }
    }

    /// Drop the least used, oldest entries down to 90 % of the cap.
    fn prune(&mut self) {
        let mut all: Vec<(u32, u64, String, String)> = self
            .map
            .iter()
            .flat_map(|(r, v)| v.iter().map(move |e| (e.count, e.last, r.clone(), e.surface.clone())))
            .collect();
        all.sort();
        let drop_n = self.total.saturating_sub(MAX_ENTRIES * 9 / 10);
        for (_, _, r, s) in all.into_iter().take(drop_n) {
            if let Some(v) = self.map.get_mut(&r) {
                v.retain(|e| e.surface != s);
                if v.is_empty() {
                    self.map.remove(&r);
                }
            }
        }
        self.total = self.map.values().map(|v| v.len()).sum();
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.total = 0;
        self.dirty = true;
    }

    /// Write the history if it changed. No path (tests, no HOME) means nothing to do.
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else { return Ok(()) };
        if !self.dirty {
            return Ok(());
        }
        let mut rows: Vec<(&String, &LearnEntry)> = self.map.iter().flat_map(|(r, v)| v.iter().map(move |e| (r, e))).collect();
        rows.sort_by(|a, b| a.0.cmp(b.0).then(b.1.count.cmp(&a.1.count)).then(a.1.surface.cmp(&b.1.surface)));
        let mut text = String::new();
        for (r, e) in rows {
            text.push_str(&format!("{}\t{}\t{}\t{}\n", r, e.surface, e.count, e.last));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        self.dirty = false;
        Ok(())
    }
}
