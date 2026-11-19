//! BDF フォントの実行時ローダ。
//!
//! `include!` で Rust ソースに埋め込む方式は、1MB 超の日本語フォントでは
//! コンパイル時間が爆発するため、実行時に BDF を直接パースして使う。
//!
//! 対応: BDF 2.1、JIS X 0208 / Unicode 両方の ENCODING 値を格納。
//! M+ F12R (Latin-1, 6x13) と M+ J12R (JIS X 0208, 12x13) の両方を想定。

use std::collections::HashMap;

/// 1グリフのビットマップ。各行1つの u16（上位ビットから描画）。
#[derive(Clone)]
pub struct BdfGlyph {
    /// ピクセル幅（DWIDTH 相当）。
    pub width: u32,
    /// ピクセル高（BBX height）。
    pub height: u32,
    /// ビットマップ行データ。MSB first。
    pub rows: Vec<u16>,
}

/// BDF フォント。
pub struct BdfFont {
    /// 全体のピクセル幅（固定幅フォントの場合）。
    pub width: u32,
    /// 全体のピクセル高。
    pub height: u32,
    /// 文字コード → グリフ。
    glyphs: HashMap<u32, BdfGlyph>,
}

impl BdfFont {
    fn ascii_backslash_and_tilde(&mut self) {
        if let Some(slash) = self.glyphs.get(&('/' as u32)).cloned() {
            // Mirror within the slash's own ink columns, so the backslash sits where the slash does.
            let (lo, hi) = slash.rows.iter().fold((15u32, 0u32), |(lo, hi), r| {
                (0..16).filter(|b| r & (0x8000 >> b) != 0).fold((lo, hi), |(lo, hi), b| (lo.min(b), hi.max(b)))
            });
            if lo <= hi {
                let rows = slash.rows.iter().map(|r| (lo..=hi).filter(|b| r & (0x8000 >> b) != 0).fold(0u16, |acc, b| acc | (0x8000 >> (lo + hi - b)))).collect();
                self.glyphs.insert('\\' as u32, BdfGlyph { rows, ..slash });
            }
        }
        if let Some(hyphen) = self.glyphs.get(&('-' as u32)).cloned() {
            if let Some(row) = hyphen.rows.iter().position(|r| *r != 0) {
                let mut rows = vec![0u16; hyphen.rows.len()];
                if row >= 1 {
                    rows[row - 1] = 0x6800; // .XX.X
                    rows[row] = 0x9000; // X..X.
                    self.glyphs.insert('~' as u32, BdfGlyph { rows, ..hyphen });
                }
            }
        }
    }

    /// BDF ファイルをパースしてロード。
    pub fn load(path: &str) -> Result<Self, String> {
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {}", path, e))?;
        Self::parse(&content)
    }

    fn parse(content: &str) -> Result<Self, String> {
        let mut font = BdfFont {
            width: 0,
            height: 0,
            glyphs: HashMap::new(),
        };

        // FONTBOUNDINGBOX w h ...
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("FONTBOUNDINGBOX ") {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    font.width = parts[0].parse().unwrap_or(0);
                    font.height = parts[1].parse().unwrap_or(0);
                }
                break;
            }
        }

        // グリフを順にパース
        let lines: Vec<&str> = content.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim_end();
            if line == "STARTCHAR" || line.starts_with("STARTCHAR ") {
                // ENCODING を探す
                let mut encoding: Option<u32> = None;
                let mut bbx_w = font.width;
                let mut bbx_h = font.height;
                let mut bitmap_rows: Vec<u16> = Vec::new();
                let mut in_bitmap = false;

                i += 1;
                while i < lines.len() {
                    let l = lines[i].trim_end();
                    if let Some(rest) = l.strip_prefix("ENCODING ") {
                        encoding = rest.split_whitespace().next().and_then(|s| s.parse().ok());
                    } else if let Some(rest) = l.strip_prefix("BBX ") {
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        if parts.len() >= 2 {
                            bbx_w = parts[0].parse().unwrap_or(bbx_w);
                            bbx_h = parts[1].parse().unwrap_or(bbx_h);
                        }
                    } else if l == "BITMAP" {
                        in_bitmap = true;
                    } else if l == "ENDCHAR" {
                        break;
                    } else if in_bitmap {
                        // 16進数の行
                        let t = l.trim();
                        if let Ok(v) = u16::from_str_radix(t, 16) {
                            // One-byte rows (half-width glyphs) are left-aligned like two-byte rows.
                            bitmap_rows.push(if t.len() <= 2 { v << 8 } else { v });
                        }
                    }
                    i += 1;
                }

                if let Some(code) = encoding {
                    // ENCODING が -1 (default char) は無視
                    if code != u32::MAX {
                        font.glyphs.insert(
                            code,
                            BdfGlyph {
                                width: bbx_w,
                                height: bbx_h,
                                rows: bitmap_rows,
                            },
                        );
                    }
                }
            }
            i += 1;
        }

        // JIS X 0201 Roman (the Shinonome 6x12 Latin set) puts a yen sign at 0x5C and an overline at
        // 0x7E. Harbour shows ASCII text (paths, code, hex dumps), so those two codes get the ASCII
        // backslash and tilde: the backslash is the font's own slash mirrored, the tilde a small wave
        // on the hyphen's rows.
        if content.lines().any(|l| l.starts_with("CHARSET_REGISTRY") && l.contains("JISX0201")) {
            font.ascii_backslash_and_tilde();
        }
        Ok(font)
    }

    /// グリフ数。
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// JIS X 0208 コードでグリフ取得。
    pub fn glyph_jis(&self, jis_code: u32) -> Option<&BdfGlyph> {
        self.glyphs.get(&jis_code)
    }

    /// Unicode からグリフ取得。
    /// ※ M+ の JIS X 0208 BDF は ENCODING が JIS コードなので、
    ///   Unicode → JIS 変換テーブルが必要。これは別途用意する。
    pub fn glyph_unicode(&self, code: u32) -> Option<&BdfGlyph> {
        self.glyphs.get(&code)
    }
}

#[cfg(test)]
mod ascii_tests {
    use super::BdfFont;

    #[test]
    fn the_shinonome_latin_set_draws_ascii_backslash_and_tilde() {
        let f = BdfFont::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/shnm6x12r.bdf")).unwrap();
        let slash = f.glyph_unicode('/' as u32).unwrap();
        let back = f.glyph_unicode('\\' as u32).unwrap();
        // The backslash is the slash mirrored: the slash's top row is right of its bottom row, the
        // backslash's the other way round (a yen sign would have a crossbar instead).
        let first = |rows: &[u16]| rows.iter().copied().find(|r| *r != 0).unwrap();
        let last = |rows: &[u16]| rows.iter().copied().rev().find(|r| *r != 0).unwrap();
        assert!(first(&slash.rows) < last(&slash.rows));
        assert!(first(&back.rows) > last(&back.rows));
        let tilde = f.glyph_unicode('~' as u32).unwrap();
        assert_eq!(tilde.rows.iter().filter(|r| **r != 0).count(), 2, "a two-row wave, not a one-row overline");
        assert!(tilde.rows.iter().position(|r| *r != 0).unwrap() > 2, "in the middle of the cell, not at the top");
    }
}
