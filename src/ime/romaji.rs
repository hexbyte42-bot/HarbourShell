//! Romaji → kana table, written from scratch for Harbour.
//!
//! The table is generated from consonant rows (`k` + a/i/u/e/o, `ky` + ...) plus a short list of
//! irregular spellings (Hepburn `shi chi tsu fu ji`, Kunrei `si ti tu hu zi`, loan-word sounds
//! such as `fa thi dhu`, small kana via `x`/`l`). The sounds themselves are facts of Japanese
//! romanisation; nothing here is taken from another input method's tables.

use std::sync::OnceLock;

const V: [&str; 5] = ["a", "i", "u", "e", "o"];

/// (consonant prefix, kana for a/i/u/e/o). Empty cell = no such syllable.
const ROWS: &[(&str, [&str; 5])] = &[
    ("", ["あ", "い", "う", "え", "お"]),
    ("k", ["か", "き", "く", "け", "こ"]),
    ("s", ["さ", "し", "す", "せ", "そ"]),
    ("t", ["た", "ち", "つ", "て", "と"]),
    ("n", ["な", "に", "ぬ", "ね", "の"]),
    ("h", ["は", "ひ", "ふ", "へ", "ほ"]),
    ("m", ["ま", "み", "む", "め", "も"]),
    ("y", ["や", "い", "ゆ", "いぇ", "よ"]),
    ("r", ["ら", "り", "る", "れ", "ろ"]),
    ("w", ["わ", "うぃ", "う", "うぇ", "を"]),
    ("g", ["が", "ぎ", "ぐ", "げ", "ご"]),
    ("z", ["ざ", "じ", "ず", "ぜ", "ぞ"]),
    ("d", ["だ", "ぢ", "づ", "で", "ど"]),
    ("b", ["ば", "び", "ぶ", "べ", "ぼ"]),
    ("p", ["ぱ", "ぴ", "ぷ", "ぺ", "ぽ"]),
    // Hepburn / loan-word consonants
    ("j", ["じゃ", "じ", "じゅ", "じぇ", "じょ"]),
    ("f", ["ふぁ", "ふぃ", "ふ", "ふぇ", "ふぉ"]),
    // Shinonome 12x12 (JIS X 0208) has ヴ but not ゔ, so the v row yields katakana ヴ.
    ("v", ["ヴァ", "ヴィ", "ヴ", "ヴェ", "ヴォ"]),
    ("l", ["ぁ", "ぃ", "ぅ", "ぇ", "ぉ"]),
    ("x", ["ぁ", "ぃ", "ぅ", "ぇ", "ぉ"]),
    // Contracted sounds (yoon): consonant + y + vowel
    ("ky", ["きゃ", "きぃ", "きゅ", "きぇ", "きょ"]),
    ("gy", ["ぎゃ", "ぎぃ", "ぎゅ", "ぎぇ", "ぎょ"]),
    ("sy", ["しゃ", "しぃ", "しゅ", "しぇ", "しょ"]),
    ("sh", ["しゃ", "し", "しゅ", "しぇ", "しょ"]),
    ("zy", ["じゃ", "じぃ", "じゅ", "じぇ", "じょ"]),
    ("jy", ["じゃ", "じぃ", "じゅ", "じぇ", "じょ"]),
    ("ty", ["ちゃ", "ちぃ", "ちゅ", "ちぇ", "ちょ"]),
    ("cy", ["ちゃ", "ちぃ", "ちゅ", "ちぇ", "ちょ"]),
    ("ch", ["ちゃ", "ち", "ちゅ", "ちぇ", "ちょ"]),
    ("dy", ["ぢゃ", "ぢぃ", "ぢゅ", "ぢぇ", "ぢょ"]),
    ("ny", ["にゃ", "にぃ", "にゅ", "にぇ", "にょ"]),
    ("hy", ["ひゃ", "ひぃ", "ひゅ", "ひぇ", "ひょ"]),
    ("my", ["みゃ", "みぃ", "みゅ", "みぇ", "みょ"]),
    ("ry", ["りゃ", "りぃ", "りゅ", "りぇ", "りょ"]),
    ("by", ["びゃ", "びぃ", "びゅ", "びぇ", "びょ"]),
    ("py", ["ぴゃ", "ぴぃ", "ぴゅ", "ぴぇ", "ぴょ"]),
    ("fy", ["ふゃ", "ふぃ", "ふゅ", "ふぇ", "ふょ"]),
    ("vy", ["ヴャ", "ヴィ", "ヴュ", "ヴェ", "ヴョ"]),
    // Loan-word sounds
    ("th", ["てゃ", "てぃ", "てゅ", "てぇ", "てょ"]),
    ("dh", ["でゃ", "でぃ", "でゅ", "でぇ", "でょ"]),
    ("ts", ["つぁ", "つぃ", "つ", "つぇ", "つぉ"]),
    ("kw", ["くぁ", "くぃ", "くぅ", "くぇ", "くぉ"]),
    ("gw", ["ぐぁ", "ぐぃ", "ぐぅ", "ぐぇ", "ぐぉ"]),
    ("tw", ["とぁ", "とぃ", "とぅ", "とぇ", "とぉ"]),
    ("dw", ["どぁ", "どぃ", "どぅ", "どぇ", "どぉ"]),
    ("xy", ["ゃ", "ぃ", "ゅ", "ぇ", "ょ"]),
    ("ly", ["ゃ", "ぃ", "ゅ", "ぇ", "ょ"]),
];

/// Spellings that do not fit the row scheme. Applied after the rows, so they win.
const EXTRA: &[(&str, &str)] = &[
    ("n'", "ん"),
    ("xn", "ん"),
    ("n", "ん"), // only reached when flushing; see `is_prefix`
    ("ya", "や"),
    ("yu", "ゆ"),
    ("yo", "よ"),
    ("wyi", "ゐ"),
    ("wye", "ゑ"),
    ("wha", "うぁ"),
    ("whi", "うぃ"),
    ("whu", "う"),
    ("whe", "うぇ"),
    ("who", "うぉ"),
    ("xtu", "っ"),
    ("ltu", "っ"),
    ("xtsu", "っ"),
    ("ltsu", "っ"),
    ("xwa", "ゎ"),
    ("lwa", "ゎ"),
    ("fyi", "ふぃ"),
    ("zi", "じ"),
    ("ci", "し"),
    ("ca", "か"),
    ("cu", "く"),
    ("co", "こ"),
    ("ce", "せ"),
    ("qa", "くぁ"),
    ("qi", "くぃ"),
    ("qu", "く"),
    ("qe", "くぇ"),
    ("qo", "くぉ"),
    ("dzu", "づ"),
    ("hwa", "ふぁ"),
    ("hwi", "ふぃ"),
    ("hwe", "ふぇ"),
    ("hwo", "ふぉ"),
    ("-", "ー"),
];

type Table = Vec<(String, &'static str)>;

fn build() -> Table {
    let mut map: std::collections::BTreeMap<String, &'static str> = Default::default();
    for (c, kana) in ROWS {
        for (i, v) in V.iter().enumerate() {
            if kana[i].is_empty() {
                continue;
            }
            map.insert(format!("{c}{v}"), kana[i]);
        }
    }
    // The consonant-only rows must not make bare "yi" / "wu" etc. surprising:
    // `yi` is い and `ye` is いぇ (kept from ROWS), `wu` is う.
    for (k, v) in EXTRA {
        map.insert((*k).to_string(), v);
    }
    // `l`/`x` rows give plain small vowels; `xya` etc. come from the `xy`/`ly` rows. `xtsu` is in EXTRA.
    map.into_iter().collect()
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(build)
}

/// Exact lookup.
pub fn lookup(s: &str) -> Option<&'static str> {
    let t = table();
    t.binary_search_by(|(k, _)| k.as_str().cmp(s)).ok().map(|i| t[i].1)
}

/// True if some key is longer than `s` and starts with it, i.e. more keystrokes may still complete
/// a different syllable. The bare "n" entry is excluded from this test on purpose: `n` followed by
/// a consonant means ん, which the state machine handles itself.
pub fn is_prefix(s: &str) -> bool {
    let t = table();
    let start = t.partition_point(|(k, _)| k.as_str() < s);
    t[start..].iter().take_while(|(k, _)| k.starts_with(s)).any(|(k, _)| k.len() > s.len())
}

/// Consonant letters that can be doubled into a small tsu (`kka` → っか). `n` is excluded.
pub fn doubles(c: char) -> bool {
    c.is_ascii_lowercase() && !"aiueon".contains(c)
}

/// Punctuation typed in kana mode. `wide` selects full-width comma and full stop (`，．`).
pub fn punct(c: char, wide: bool) -> Option<char> {
    Some(match c {
        ',' => if wide { '，' } else { '、' },
        '.' => if wide { '．' } else { '。' },
        '[' => '「',
        ']' => '」',
        '/' => '・',
        // U+FF5E, because that is the tilde-like mark JIS X 0208 maps in the bundled font (U+301C is not covered).
        '~' => '～',
        '?' => '？',
        '!' => '！',
        '-' => 'ー',
        _ => return None,
    })
}

#[cfg(test)]
pub fn all_entries() -> &'static Table {
    table()
}
