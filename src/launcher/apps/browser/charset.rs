//! Byte -> text decoding for fetched pages: UTF-8, Latin-1/Windows-1252, Shift_JIS, EUC-JP and
//! ISO-2022-JP (rare, skipped). Japanese decoding reuses the JIS X 0208 -> Unicode table the
//! font already compiles in (reversed once), so no crate is needed.

use std::collections::HashMap;
use std::sync::OnceLock;

fn jis_to_unicode() -> &'static HashMap<u32, char> {
    static MAP: OnceLock<HashMap<u32, char>> = OnceLock::new();
    MAP.get_or_init(|| {
        // Reverse of crate::jis::unicode_to_jis over the BMP kanji / kana / symbol ranges.
        let mut m = HashMap::new();
        for u in 0x80u32..0xFFFF {
            if let Some(ch) = char::from_u32(u) {
                if (0xFF61..=0xFF9F).contains(&u) {
                    continue; // half-width kana normalise onto full-width ones
                }
                if let Some(j) = crate::jis::unicode_to_jis(ch) {
                    m.entry(j).or_insert(ch);
                }
            }
        }
        m
    })
}

fn jis(row: u8, col: u8) -> char {
    jis_to_unicode().get(&(((row as u32) << 8) | col as u32)).copied().unwrap_or('\u{FFFD}')
}

pub fn decode_sjis(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            0..=0x7F => {
                out.push(c as char);
                i += 1;
            }
            0xA1..=0xDF => {
                out.push(char::from_u32(0xFF61 + (c as u32 - 0xA1)).unwrap_or('?'));
                i += 1;
            }
            0x81..=0x9F | 0xE0..=0xFC if i + 1 < b.len() => {
                let t = b[i + 1];
                if !(0x40..=0xFC).contains(&t) || t == 0x7F {
                    out.push('\u{FFFD}');
                    i += 1;
                    continue;
                }
                let lead = if c >= 0xE0 { c - 0xC1 } else { c - 0x81 } as u32; // 0..
                let (row, col) = if t >= 0x9F {
                    (lead * 2 + 2, t as u32 - 0x9F + 1)
                } else {
                    (lead * 2 + 1, if t > 0x7F { t as u32 - 0x40 - 1 + 1 } else { t as u32 - 0x40 + 1 })
                };
                out.push(jis((row + 0x20) as u8, (col + 0x20) as u8));
                i += 2;
            }
            _ => {
                out.push('\u{FFFD}');
                i += 1;
            }
        }
    }
    out
}

pub fn decode_eucjp(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            out.push(c as char);
            i += 1;
        } else if c == 0x8E && i + 1 < b.len() {
            out.push(char::from_u32(0xFF61 + (b[i + 1] as u32).saturating_sub(0xA1)).unwrap_or('?'));
            i += 2;
        } else if (0xA1..=0xFE).contains(&c) && i + 1 < b.len() {
            out.push(jis(c - 0x80, b[i + 1].saturating_sub(0x80)));
            i += 2;
        } else {
            out.push('\u{FFFD}');
            i += 1;
        }
    }
    out
}

fn decode_latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Canonical charset name from a label, if we can decode it.
pub fn canonical(label: &str) -> Option<&'static str> {
    let l = label.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase();
    Some(match l.as_str() {
        "utf-8" | "utf8" => "utf-8",
        "shift_jis" | "shift-jis" | "sjis" | "x-sjis" | "windows-31j" | "ms932" | "cp932" | "ms_kanji" => "sjis",
        "euc-jp" | "eucjp" | "x-euc-jp" => "euc-jp",
        "iso-8859-1" | "latin1" | "windows-1252" | "us-ascii" | "ascii" | "iso-8859-15" => "latin1",
        _ => return None,
    })
}

pub fn decode(bytes: &[u8], charset: Option<&str>) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match charset.and_then(canonical) {
        Some("sjis") => decode_sjis(bytes),
        Some("euc-jp") => decode_eucjp(bytes),
        Some("latin1") => decode_latin1(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `charset=` from a Content-Type header value or a `<meta>` content / charset attribute.
pub fn charset_from(text: &str) -> Option<String> {
    let l = text.to_ascii_lowercase();
    let i = l.find("charset=")? + 8;
    let rest = &text[i..];
    let rest = rest.trim_start_matches(|c| c == '"' || c == '\'');
    let end = rest.find(|c: char| c == ';' || c == '"' || c == '\'' || c == ' ' || c == '>' || c == '/').unwrap_or(rest.len());
    let v = &rest[..end];
    if v.is_empty() { None } else { Some(v.to_string()) }
}

/// Sniff `<meta charset>` / `<meta http-equiv content="...charset=">` in the first 2 KB.
pub fn sniff_meta(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(2048)];
    let s: String = head.iter().map(|&c| if c < 0x80 { c as char } else { ' ' }).collect();
    let l = s.to_ascii_lowercase();
    let m = l.find("<meta").and_then(|_| {
        let mut from = 0;
        while let Some(p) = l[from..].find("<meta") {
            let start = from + p;
            let end = l[start..].find('>').map(|e| start + e).unwrap_or(l.len());
            if let Some(cs) = charset_from(&s[start..end]) {
                return Some(cs);
            }
            from = end;
        }
        None
    });
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_bom() {
        assert_eq!(decode(b"\xEF\xBB\xBFabc \xE3\x81\x82", None), "abc あ");
    }

    #[test]
    fn shift_jis() {
        // "日本語" in Shift_JIS: 93FA 967B 8CEA; "ｱ" half-width: B1
        assert_eq!(decode_sjis(&[0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA, b'!', 0xB1]), "日本語!ｱ");
        // "あいう": 82A0 82A2 82A4
        assert_eq!(decode_sjis(&[0x82, 0xA0, 0x82, 0xA2, 0x82, 0xA4]), "あいう");
    }

    #[test]
    fn euc_jp() {
        assert_eq!(decode_eucjp(&[0xC6, 0xFC, 0xCB, 0xDC, 0xB8, 0xEC]), "日本語");
    }

    #[test]
    fn labels() {
        assert_eq!(charset_from("text/html; charset=Shift_JIS"), Some("Shift_JIS".into()));
        assert_eq!(canonical("Shift_JIS"), Some("sjis"));
        assert_eq!(canonical("EUC-JP"), Some("euc-jp"));
        assert_eq!(canonical("klingon"), None);
        assert_eq!(sniff_meta(b"<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=EUC-JP\">"), Some("EUC-JP".into()));
        assert_eq!(sniff_meta(b"<meta charset=\"utf-8\">"), Some("utf-8".into()));
    }
}
