//! Kana and width conversions used by the input method (all written here, no external tables).

/// Hiragana → full-width katakana (code point offset 0x60). Other characters are kept.
pub fn hira_to_kata(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' | '\u{309D}' | '\u{309E}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// Full-width katakana → hiragana. Other characters are kept.
pub fn kata_to_hira(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{30A1}'..='\u{30F6}' | '\u{30FD}' | '\u{30FE}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

const FULL: &str = "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲンァィゥェォッャュョヮー。「」、・";
const HALF: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜｦﾝｧｨｩｪｫｯｬｭｮﾜｰ｡｢｣､･";
const VOICED_FULL: &str = "ガギグゲゴザジズゼゾダヂヅデドバビブベボヴ";
const VOICED_BASE: &str = "ｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎｳ";
const SEMI_FULL: &str = "パピプペポ";
const SEMI_BASE: &str = "ﾊﾋﾌﾍﾎ";

/// Full-width katakana (hiragana is converted first) → half-width katakana; voiced sounds become
/// base + ﾞ / ﾟ. Everything else is kept.
pub fn to_half_kata(s: &str) -> String {
    let mut out = String::new();
    for c in hira_to_kata(s).chars() {
        if let Some(i) = FULL.chars().position(|f| f == c) {
            out.extend(HALF.chars().nth(i));
        } else if let Some(i) = VOICED_FULL.chars().position(|f| f == c) {
            out.extend(VOICED_BASE.chars().nth(i));
            out.push('ﾞ');
        } else if let Some(i) = SEMI_FULL.chars().position(|f| f == c) {
            out.extend(SEMI_BASE.chars().nth(i));
            out.push('ﾟ');
        } else {
            out.push(c);
        }
    }
    out
}

/// ASCII → full-width forms (space → U+3000). Other characters are kept. The two quotes become the
/// curly quotes (U+2019 / U+201D) because JIS X 0208 has no full-width apostrophe or straight quote.
pub fn to_full_alnum(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => '\u{3000}',
            '\'' => '\u{2019}',
            '"' => '\u{201D}',
            '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_line_up() {
        assert_eq!(FULL.chars().count(), HALF.chars().count());
        assert_eq!(VOICED_FULL.chars().count(), VOICED_BASE.chars().count());
        assert_eq!(SEMI_FULL.chars().count(), SEMI_BASE.chars().count());
    }

    #[test]
    fn katakana_round_trip() {
        assert_eq!(hira_to_kata("にほんご、ゔぁいおりん"), "ニホンゴ、ヴァイオリン");
        assert_eq!(kata_to_hira("ニホンゴ、ヴァイオリンー"), "にほんご、ゔぁいおりんー");
        assert_eq!(hira_to_kata("ーabc1"), "ーabc1");
    }

    #[test]
    fn half_width_kana() {
        assert_eq!(to_half_kata("かたかな"), "ｶﾀｶﾅ");
        assert_eq!(to_half_kata("がぎぐ"), "ｶﾞｷﾞｸﾞ");
        assert_eq!(to_half_kata("ぱぴぷぺぽ"), "ﾊﾟﾋﾟﾌﾟﾍﾟﾎﾟ");
        assert_eq!(to_half_kata("きゃっ、「ー」。"), "ｷｬｯ､｢ｰ｣｡");
        assert_eq!(to_half_kata("ゔ"), "ｳﾞ");
        assert_eq!(to_half_kata("Abc"), "Abc");
    }

    #[test]
    fn full_width_alnum() {
        assert_eq!(to_full_alnum("Abc 123!"), "Ａｂｃ　１２３！");
        assert_eq!(to_full_alnum("日本"), "日本");
    }
}
