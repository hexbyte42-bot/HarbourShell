//! Reading → candidates. Phase 2: exact dictionary matches (learned first), plus one
//! lowest-cost segmentation of the whole reading (word costs only, no connection costs: that is phase 3).

use super::dict::{displayable, Dict};
use super::learn::Learn;

/// Cost added per word in the segmentation, so one long word beats several short ones of similar cost.
pub const SEG_PENALTY: i64 = 4000;
/// Cost of a character with no dictionary word (kept as it was typed).
pub const UNKNOWN_COST: i64 = 12000;
const MAX_WORD_CHARS: usize = 16;
const MAX_READING_CHARS: usize = 48;
/// Learned words get this much cheaper than the dictionary's best, plus a little more per use.
const KATAKANA_PENALTY: i64 = 4000;
const KANA_MARGIN: i64 = 1500;
const LEARN_BONUS: i64 = 3000;

#[derive(Default)]
pub struct Converter {
    pub dict: Option<Dict>,
    pub learn: Learn,
    /// Record committed choices (off in the terminal and for passwords, set by the launcher).
    pub learning: bool,
}

impl Converter {
    pub fn has_dict(&self) -> bool {
        self.dict.is_some()
    }

    /// Dictionary and learned words for exactly `key`, cheapest first.
    pub fn words(&self, key: &str) -> Vec<(i64, String)> {
        let mut out: Vec<(i64, String)> = Vec::new();
        if let Some(d) = &self.dict {
            out.extend(d.lookup(key).into_iter().map(|c| (c.cost as i64, c.surface)));
        }
        let base = out.first().map(|w| w.0).unwrap_or(5000);
        for e in self.learn.get(key) {
            let cost = base - LEARN_BONUS - 300 * (e.count.min(10) as i64) - (e.last as i64 % 7);
            if let Some(w) = out.iter_mut().find(|w| w.1 == e.surface) {
                w.0 = w.0.min(cost);
            } else {
                out.push((cost, e.surface.clone()));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        out
    }

    /// The word the segmenter uses for `key`. The dictionary lists the kana spelling of many words at
    /// (almost) zero cost and katakana spellings cheaply; without connection costs those would win every
    /// split, so a kana spelling gives way to a real conversion that is nearly as cheap, and a katakana
    /// spelling of the reading is penalised.
    fn best_word(&self, key: &str) -> Option<(i64, String)> {
        let words = self.words(key);
        let kata = super::kana::hira_to_kata(key);
        let adj = |w: &(i64, String)| if w.1 == kata { w.0 + KATAKANA_PENALTY } else { w.0 };
        // One-kana words are almost always particles (は, が, に): without part-of-speech information
        // the noun 葉 must not take their place.
        if key.chars().count() == 1 {
            if let Some(w) = words.iter().find(|w| w.1 == key) {
                return Some((w.0, w.1.clone()));
            }
        }
        let best = words.iter().map(|w| (adj(w), &w.1)).min()?;
        if best.1 == key {
            let alt = words.iter().filter(|w| w.1 != key).map(|w| (adj(w), &w.1)).min();
            if let Some(a) = alt {
                if a.0 <= best.0 + KANA_MARGIN {
                    return Some((a.0, a.1.clone()));
                }
            }
        }
        Some((best.0, best.1.clone()))
    }

    /// Whole-reading candidates, best first, without duplicates: exact matches, then the segmentation
    /// (first when the exact list is empty). Empty without a dictionary or learned words.
    pub fn candidates(&self, reading: &str) -> Vec<String> {
        if !self.has_dict() && self.learn.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<String> = self.words(reading).into_iter().map(|w| w.1).collect();
        if let Some(seg) = self.segment(reading) {
            if !out.contains(&seg) && seg != reading {
                out.push(seg);
            }
        }
        out.retain(|s| s.chars().all(displayable));
        out
    }

    /// Lowest-cost split of `reading` into dictionary words (unknown characters stay as typed).
    /// `None` when there is nothing to split or the reading is too long to be worth it.
    pub fn segment(&self, reading: &str) -> Option<String> {
        let chars: Vec<char> = reading.chars().collect();
        let n = chars.len();
        if n < 2 || n > MAX_READING_CHARS {
            return None;
        }
        let mut best: Vec<Option<(i64, usize, String)>> = vec![None; n + 1];
        best[0] = Some((0, 0, String::new()));
        for i in 0..n {
            let Some((base, _, _)) = best[i].clone() else { continue };
            for j in i + 1..=(i + MAX_WORD_CHARS).min(n) {
                let key: String = chars[i..j].iter().collect();
                let cand = self.best_word(&key).or_else(|| {
                    // A single character nobody knows: keep it, at a steep price.
                    (j == i + 1).then(|| (UNKNOWN_COST, key.clone()))
                });
                let Some((cost, surface)) = cand else { continue };
                let total = base + cost + SEG_PENALTY;
                if best[j].as_ref().map(|b| total < b.0).unwrap_or(true) {
                    best[j] = Some((total, i, surface));
                }
            }
        }
        // Walk back.
        let mut parts: Vec<String> = Vec::new();
        let mut at = n;
        while at > 0 {
            let (_, from, surf) = best[at].clone()?;
            parts.push(surf);
            at = from;
        }
        parts.reverse();
        Some(parts.concat())
    }
}
