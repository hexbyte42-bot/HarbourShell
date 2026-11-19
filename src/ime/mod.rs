//! Harbour's built-in Japanese input method, phase 1: romaji → kana, no dictionary.
//!
//! This module has no SDL dependency. The launcher translates its `Input` events into [`KeyIn`]
//! (see `launcher::ime_glue`), and gets back text to commit plus a flag saying whether the key was
//! used up. The engine is original code; see NOTICE and THIRD_PARTY.md ("Input method").
//!
//! Model: a composition is a list of units (a kana or a literal character, each remembering the
//! keys that produced it) with a caret, plus the romaji still waiting for its next key (`pending`,
//! always at the caret). Space opens a candidate list of the kana / width variants of the
//! composition; a dictionary will add real conversions to the same list in a later phase.

/// File name of the user-installed dictionary inside [`data_dir`].
pub const DICT_FILE: &str = "harbour-ime.dic";

/// `~/.local/share/harbour/ime` (`HARBOUR_IME_DIR` overrides): the dictionary and the learning history.
pub fn data_dir() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("HARBOUR_IME_DIR") {
        return std::path::PathBuf::from(p);
    }
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/share/harbour/ime")
}

pub mod conv;
pub mod dict;
pub mod kana;
pub mod learn;
pub mod romaji;

/// Input mode, shown as `A` / `あ` / `ア` / `ｱ` / `Ａ`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The IME is out of the way; every key goes to the app untouched.
    Direct,
    Hiragana,
    Katakana,
    HalfKatakana,
    /// Typed ASCII is committed as full-width characters.
    FullAlnum,
}

impl Mode {
    pub fn indicator(self) -> &'static str {
        match self {
            Mode::Direct => "A",
            Mode::Hiragana => "あ",
            Mode::Katakana => "ア",
            Mode::HalfKatakana => "ｱ",
            Mode::FullAlnum => "Ａ",
        }
    }

    pub fn is_direct(self) -> bool {
        self == Mode::Direct
    }
}

/// Keys the engine understands. `Char` is a typed character (layout and Shift already applied).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Space,
    Enter,
    Backspace,
    Delete,
    Esc,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Function keys, 1-based (`F(7)` = F7).
    F(u8),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyIn {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyIn {
    pub fn new(key: Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: false }
    }
    pub fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }
    pub fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
    pub fn alt(mut self) -> Self {
        self.alt = true;
        self
    }
}

/// Result of one key. `commit` is text to hand to the app *before* the key itself (when
/// `consumed` is false the original key is still delivered afterwards).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub commit: String,
    pub consumed: bool,
}

impl Out {
    fn eat() -> Self {
        Out { commit: String::new(), consumed: true }
    }
    fn eat_commit(commit: String) -> Self {
        Out { commit, consumed: true }
    }
    fn pass() -> Self {
        Out::default()
    }
    fn pass_commit(commit: String) -> Self {
        Out { commit, consumed: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit {
    /// Hiragana (or the literal character), whatever the mode: the mode only changes how it is shown.
    kana: String,
    /// The keys that produced it (for the full-width / half-width Latin variants).
    raw: String,
    literal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Hiragana,
    Katakana,
    HalfKatakana,
    FullAlnum,
    HalfAlnum,
}

#[derive(Clone, Debug)]
struct Cand {
    /// `None` = a dictionary / learned conversion; `Some(v)` = a kana or width variant.
    items: Vec<(Option<Variant>, String)>,
    sel: usize,
    /// The hiragana reading the candidates were made for (what a learned choice is filed under).
    reading: String,
}

/// Candidates per page of the candidate window; digit keys 1-5 pick within the page shown.
pub const CAND_PAGE: usize = 5;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What the launcher draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub mode: Mode,
    /// The composition text (with the selected candidate substituted while converting).
    pub preedit: String,
    /// Caret position in characters inside `preedit` (`None` while a candidate is selected).
    pub caret: Option<usize>,
    /// Character range of the romaji still being typed (drawn with a dotted underline).
    pub pending: Option<(usize, usize)>,
    pub cand: Option<CandView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandView {
    pub items: Vec<String>,
    pub sel: usize,
}

pub struct Ime {
    mode: Mode,
    /// The kana mode toggling back from Direct returns to.
    kana_mode: Mode,
    units: Vec<Unit>,
    caret: usize,
    pending: String,
    /// After a capital letter, letters stay Latin until something else is typed ("Windows").
    lit_run: bool,
    /// `pending` is the second `n` of a typed `nn`: it is invisible, and becomes the start of
    /// な/にゃ... if a vowel or `y` follows (`konnichi` → こんにち) and is dropped otherwise (`kannji`).
    carry_n: bool,
    cand: Option<Cand>,
    /// Dictionary, learned words and the segmenter (no dictionary = phase 1 behaviour).
    pub conv: conv::Converter,
    /// `，．` instead of `、。`.
    pub wide_punct: bool,
}

impl Default for Ime {
    fn default() -> Self {
        Self::new()
    }
}

impl Ime {
    pub fn new() -> Self {
        Self { mode: Mode::Direct, kana_mode: Mode::Hiragana, units: vec![], caret: 0, pending: String::new(), lit_run: false, carry_n: false, cand: None, conv: conv::Converter::default(), wide_punct: false }
    }

    /// Install a dictionary (`None` returns to phase 1: kana only).
    pub fn set_dict(&mut self, dict: Option<dict::Dict>) {
        self.conv.dict = dict;
    }

    /// Load the learning history from `path`; characters the font cannot draw are dropped.
    pub fn load_learn(&mut self, path: &std::path::Path) {
        self.conv.learn = learn::Learn::load(path, |s| s.chars().all(dict::displayable));
    }

    /// Write the learning history if it changed (the launcher calls this after a quiet period and on exit).
    pub fn flush_learn(&mut self) -> std::io::Result<()> {
        self.conv.learn.save()
    }

    pub fn learn_dirty(&self) -> bool {
        self.conv.learn.is_dirty()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn is_composing(&self) -> bool {
        !self.units.is_empty() || !self.pending.is_empty()
    }

    pub fn is_converting(&self) -> bool {
        self.cand.is_some()
    }

    /// Switch mode. A composition in progress is committed first (returned).
    pub fn set_mode(&mut self, mode: Mode) -> String {
        let out = self.take_commit();
        self.mode = mode;
        if !mode.is_direct() {
            self.kana_mode = mode;
        }
        out
    }

    /// The toggle key: Direct ⇄ the last kana mode. Returns text to commit.
    pub fn toggle(&mut self) -> String {
        let to = if self.mode.is_direct() { self.kana_mode } else { Mode::Direct };
        self.set_mode(to)
    }

    /// Shift + toggle key: あ → ア → ｱ → Ａ → あ (from Direct it just turns the IME on in あ).
    pub fn cycle_kana(&mut self) -> String {
        let to = match self.mode {
            Mode::Direct | Mode::FullAlnum => Mode::Hiragana,
            Mode::Hiragana => Mode::Katakana,
            Mode::Katakana => Mode::HalfKatakana,
            Mode::HalfKatakana => Mode::FullAlnum,
        };
        self.set_mode(to)
    }

    /// Drop the composition without committing it.
    pub fn cancel(&mut self) {
        self.units.clear();
        self.caret = 0;
        self.pending.clear();
        self.carry_n = false;
        self.lit_run = false;
        self.cand = None;
    }

    // ---- composition text ----------------------------------------------------------------

    fn show(&self, u: &Unit) -> String {
        if u.literal {
            return u.kana.clone();
        }
        match self.mode {
            Mode::Katakana => kana::hira_to_kata(&u.kana),
            Mode::HalfKatakana => kana::to_half_kata(&u.kana),
            _ => u.kana.clone(),
        }
    }

    fn preedit_text(&self) -> String {
        let mut s = String::new();
        for (i, u) in self.units.iter().enumerate() {
            if i == self.caret {
                s.push_str(self.visible_pending());
            }
            s.push_str(&self.show(u));
        }
        if self.caret >= self.units.len() {
            s.push_str(self.visible_pending());
        }
        s
    }

    fn visible_pending(&self) -> &str {
        if self.carry_n {
            ""
        } else {
            &self.pending
        }
    }

    fn hira_text(&self) -> String {
        self.units.iter().map(|u| u.kana.as_str()).collect()
    }

    fn raw_text(&self) -> String {
        self.units.iter().map(|u| u.raw.as_str()).collect()
    }

    fn variants(&self) -> Vec<(Option<Variant>, String)> {
        let hira = self.hira_text();
        let raw = self.raw_text();
        // Real conversions first (dictionary / learned / segmented), then the kana and width forms.
        let mut v: Vec<(Option<Variant>, String)> = self.conv.candidates(&hira).into_iter().map(|s| (None, s)).collect();
        // Without conversions Space goes to katakana first (phase 1 behaviour); with them hiragana is the
        // natural first fallback.
        let first_kata = self.mode != Mode::Katakana && v.is_empty();
        if first_kata {
            v.push((Some(Variant::Katakana), kana::hira_to_kata(&hira)));
            v.push((Some(Variant::Hiragana), hira.clone()));
        } else {
            v.push((Some(Variant::Hiragana), hira.clone()));
            v.push((Some(Variant::Katakana), kana::hira_to_kata(&hira)));
        }
        v.push((Some(Variant::HalfKatakana), kana::to_half_kata(&hira)));
        v.push((Some(Variant::FullAlnum), kana::to_full_alnum(&raw)));
        v.push((Some(Variant::HalfAlnum), raw));
        let mut seen: Vec<String> = Vec::new();
        v.retain(|(_, t)| {
            if seen.contains(t) {
                false
            } else {
                seen.push(t.clone());
                true
            }
        });
        v
    }

    /// Flush a waiting `n` (→ ん) or leftover letters, so the composition is final text.
    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        if self.carry_n {
            self.carry_n = false;
            self.pending.clear();
            return;
        }
        let p = std::mem::take(&mut self.pending);
        if let Some(k) = romaji::lookup(&p) {
            self.insert(Unit { kana: k.to_string(), raw: p, literal: false });
        } else {
            for c in p.chars() {
                self.insert(Unit { kana: c.to_string(), raw: c.to_string(), literal: true });
            }
        }
    }

    fn insert(&mut self, u: Unit) {
        self.units.insert(self.caret, u);
        self.caret += 1;
    }

    /// Everything typed so far as final text, and an empty composition.
    fn take_commit(&mut self) -> String {
        let text = match self.cand.take() {
            Some(c) => self.commit_cand(&c, c.sel),
            None => {
                self.flush_pending();
                self.preedit_text()
            }
        };
        self.cancel();
        text
    }

    /// The text of candidate `i`; a real conversion is remembered for next time.
    fn commit_cand(&mut self, c: &Cand, i: usize) -> String {
        let (variant, text) = &c.items[i];
        if variant.is_none() && self.conv.learning {
            self.conv.learn.record(&c.reading, text, now_secs());
        }
        text.clone()
    }

    fn resolve(&mut self) {
        while !self.pending.is_empty() {
            if let Some(k) = romaji::lookup(&self.pending) {
                if !romaji::is_prefix(&self.pending) {
                    let raw = std::mem::take(&mut self.pending);
                    self.insert(Unit { kana: k.to_string(), raw, literal: false });
                    continue;
                }
            }
            if romaji::is_prefix(&self.pending) {
                return;
            }
            // Not a syllable and cannot become one: the first letter is settled on its own.
            let mut it = self.pending.chars();
            let c0 = it.next().unwrap_or(' ');
            let c1 = it.next();
            let unit = if (c1 == Some(c0) && romaji::doubles(c0)) || (c0 == 't' && c1 == Some('c')) {
                Unit { kana: "っ".into(), raw: c0.to_string(), literal: false }
            } else if c0 == 'n' && c1.is_some() {
                self.carry_n = c1 == Some('n');
                Unit { kana: "ん".into(), raw: "n".into(), literal: false }
            } else {
                Unit { kana: c0.to_string(), raw: c0.to_string(), literal: true }
            };
            self.pending.remove(0);
            self.insert(unit);
        }
    }

    /// A typed character in a kana mode. Returns text to commit at once (digits and symbols typed
    /// with nothing being composed).
    fn feed(&mut self, c: char) -> String {
        if self.carry_n {
            self.carry_n = false;
            if !"aiueoyn".contains(c) {
                self.pending.clear();
            }
        }
        if c.is_ascii_uppercase() || (self.lit_run && c.is_ascii_alphabetic()) {
            self.flush_pending();
            self.insert(Unit { kana: c.to_string(), raw: c.to_string(), literal: true });
            self.lit_run = true;
            return String::new();
        }
        if c.is_ascii_lowercase() || (c == '\'' && self.pending == "n") {
            self.lit_run = false;
            self.pending.push(c);
            self.resolve();
            return String::new();
        }
        self.lit_run = false;
        self.flush_pending();
        if let Some(p) = romaji::punct(c, self.wide_punct) {
            self.insert(Unit { kana: p.to_string(), raw: c.to_string(), literal: false });
            String::new()
        } else if self.is_composing() {
            self.insert(Unit { kana: c.to_string(), raw: c.to_string(), literal: true });
            String::new()
        } else {
            c.to_string()
        }
    }

    // ---- candidates -----------------------------------------------------------------------

    fn open_cand(&mut self, variant: Option<Variant>) {
        self.flush_pending();
        if self.units.is_empty() {
            return;
        }
        let items = self.variants();
        let sel = variant
            .and_then(|v| items.iter().position(|(k, _)| *k == Some(v)))
            .or_else(|| {
                // The wanted variant was a duplicate of an earlier one: select the item with its text.
                variant.and_then(|v| {
                    let text = self.variant_text(v);
                    items.iter().position(|(_, t)| *t == text)
                })
            })
            .unwrap_or(0);
        self.cand = Some(Cand { items, sel, reading: self.hira_text() });
    }

    fn variant_text(&self, v: Variant) -> String {
        match v {
            Variant::Hiragana => self.hira_text(),
            Variant::Katakana => kana::hira_to_kata(&self.hira_text()),
            Variant::HalfKatakana => kana::to_half_kata(&self.hira_text()),
            Variant::FullAlnum => kana::to_full_alnum(&self.raw_text()),
            Variant::HalfAlnum => self.raw_text(),
        }
    }

    fn step_cand(&mut self, back: bool) {
        if let Some(c) = &mut self.cand {
            let n = c.items.len();
            c.sel = if back { (c.sel + n - 1) % n } else { (c.sel + 1) % n };
        }
    }

    // ---- keys ------------------------------------------------------------------------------

    pub fn on_key(&mut self, k: KeyIn) -> Out {
        if self.mode.is_direct() {
            return Out::pass();
        }
        if self.mode == Mode::FullAlnum {
            return match k.key {
                Key::Char(c) if !k.ctrl && !k.alt => Out::eat_commit(kana::to_full_alnum(&c.to_string())),
                Key::Space if !k.ctrl && !k.alt && !k.shift => Out::eat_commit("\u{3000}".into()),
                _ => Out::pass(),
            };
        }
        if !self.is_composing() {
            return match k.key {
                Key::Char(c) if !k.ctrl && !k.alt => {
                    let commit = self.feed(c);
                    Out::eat_commit(commit)
                }
                _ => Out::pass(),
            };
        }
        // Composing from here on.
        if k.alt {
            return Out::pass_commit(self.take_commit());
        }
        if k.ctrl {
            return match k.key {
                Key::Char('u') | Key::Char('U') => {
                    self.cancel();
                    Out::eat()
                }
                Key::Char('k') | Key::Char('K') => {
                    self.open_cand(Some(Variant::Katakana));
                    Out::eat()
                }
                Key::Char('l') | Key::Char('L') => {
                    self.open_cand(Some(Variant::FullAlnum));
                    Out::eat()
                }
                _ => Out::pass_commit(self.take_commit()),
            };
        }
        match k.key {
            Key::Char(c) => {
                if let (Some(cand), Some(d)) = (&self.cand, c.to_digit(10)) {
                    let i = cand.sel / CAND_PAGE * CAND_PAGE + (d as usize).wrapping_sub(1);
                    if d >= 1 && (d as usize) <= CAND_PAGE && i < cand.items.len() {
                        let cand = self.cand.take().expect("checked");
                        let text = self.commit_cand(&cand, i);
                        self.cancel();
                        return Out::eat_commit(text);
                    }
                }
                let mut commit = if self.cand.is_some() { self.take_commit() } else { String::new() };
                commit.push_str(&self.feed(c));
                Out::eat_commit(commit)
            }
            Key::Space => {
                if self.cand.is_some() {
                    self.step_cand(k.shift);
                } else {
                    self.open_cand(None);
                }
                Out::eat()
            }
            Key::Enter => Out::eat_commit(self.take_commit()),
            Key::Esc => {
                if self.cand.take().is_none() {
                    self.cancel();
                }
                Out::eat()
            }
            Key::Backspace => {
                if self.cand.take().is_some() {
                    return Out::eat();
                }
                if self.carry_n {
                    self.carry_n = false;
                    self.pending.clear();
                }
                if !self.pending.is_empty() {
                    self.pending.pop();
                } else if self.caret > 0 {
                    self.units.remove(self.caret - 1);
                    self.caret -= 1;
                }
                self.lit_run = false;
                Out::eat()
            }
            Key::Delete => {
                self.cand = None;
                self.flush_pending();
                if self.caret < self.units.len() {
                    self.units.remove(self.caret);
                }
                Out::eat()
            }
            Key::Left | Key::Right | Key::Home | Key::End => {
                self.cand = None;
                self.flush_pending();
                self.lit_run = false;
                let n = self.units.len();
                self.caret = match k.key {
                    Key::Left => self.caret.saturating_sub(1),
                    Key::Right => (self.caret + 1).min(n),
                    Key::Home => 0,
                    _ => n,
                };
                Out::eat()
            }
            Key::Up | Key::Down if self.cand.is_some() => {
                self.step_cand(k.key == Key::Up);
                Out::eat()
            }
            Key::F(n @ 6..=10) => {
                let v = match n {
                    6 => Variant::Hiragana,
                    7 => Variant::Katakana,
                    8 => Variant::HalfKatakana,
                    9 => Variant::FullAlnum,
                    _ => Variant::HalfAlnum,
                };
                self.open_cand(Some(v));
                Out::eat()
            }
            // Tab and anything else: settle the text first, then the key goes on to the app.
            // Tab itself is eaten so a half-typed word never triggers the launcher's Tab actions.
            Key::Tab => Out::eat_commit(self.take_commit()),
            _ => Out::pass_commit(self.take_commit()),
        }
    }

    pub fn view(&self) -> Option<View> {
        if !self.is_composing() {
            return None;
        }
        if let Some(c) = &self.cand {
            return Some(View {
                mode: self.mode,
                preedit: c.items[c.sel].1.clone(),
                caret: None,
                pending: None,
                cand: Some(CandView { items: c.items.iter().map(|(_, t)| t.clone()).collect(), sel: c.sel }),
            });
        }
        let before: usize = self.units[..self.caret].iter().map(|u| self.show(u).chars().count()).sum();
        let plen = self.visible_pending().chars().count();
        Some(View {
            mode: self.mode,
            preedit: self.preedit_text(),
            caret: Some(before + plen),
            pending: if plen > 0 { Some((before, before + plen)) } else { None },
            cand: None,
        })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_dict;
