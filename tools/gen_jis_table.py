#!/usr/bin/env python3
"""
gen_jis_table.py - Unicode → JIS X 0208 変換テーブル生成器。
================================================================
JIS X 0208 BDF (mplus_j12r.bdf 等) は ENCODING に JIS 区点コードを
持つため、Unicode から引くには変換が必要。

CP932 (Shift_JIS) を経由して Unicode → JIS X 0208 を計算し、
Rust の静的マッチテーブル (jis.rs) を生成する。

使い方:
    python3 tools/gen_jis_table.py > src/generated/jis_table.rs
"""
import sys


def sjis_to_jis(s1: int, s2: int):
    """Shift_JIS 2バイト → JIS X 0208 区点コード (0x2121..0x7E7E)。"""
    if not ((0x81 <= s1 <= 0x9F or 0xE0 <= s1 <= 0xEF) and 0x40 <= s2 <= 0xFC and s2 != 0x7F):
        return None
    lead = s1 - (0x81 if s1 <= 0x9F else 0xC1)
    j1 = lead * 2 + 0x21
    if s2 >= 0x9F:
        j1 += 1
        j2 = s2 - 0x7E
    elif s2 > 0x7F:
        j2 = s2 - 0x20
    else:
        j2 = s2 - 0x1F

    # 範囲チェック (1..94 → 0x21..0x7E)
    if not (0x21 <= j1 <= 0x7E and 0x21 <= j2 <= 0x7E):
        return None
    return (j1 << 8) | j2


def build_table():
    """全 CP932 2バイト文字を舐めて Unicode → JIS マップを作る。"""
    table = {}
    # CP932 の全2バイトコード空間を走査
    for s1 in range(0x81, 0x100):
        if s1 == 0xA0:
            continue
        for s2 in range(0x40, 0x100):
            if s2 == 0x7F:
                continue
            try:
                ch = bytes([s1, s2]).decode("cp932")
            except Exception:
                continue
            if len(ch) != 1:
                continue
            u = ord(ch)
            jis = sjis_to_jis(s1, s2)
            if jis is None:
                continue
            # 複数のSJISが同じUnicodeに当たる場合があるが、最初を採用
            if u not in table:
                table[u] = jis
    return table


def emit_rust(table):
    out = []
    out.append("// Unicode → JIS X 0208 自動生成マップ。")
    out.append("// tools/gen_jis_table.py により生成。編集しないこと。")
    out.append("")
    out.append("/// (Unicode, JIS X 0208 code) のソート済みペア。")
    out.append("pub static UNICODE_TO_JIS: &[(u32, u32)] = &[")
    for u in sorted(table.keys()):
        jis = table[u]
        ch = chr(u)
        out.append("    (0x%04X, 0x%04X),  // %r" % (u, jis, ch))
    out.append("];")
    return "\n".join(out) + "\n"


def main():
    table = build_table()
    n = len(table)
    print("Generated " + str(n) + " entries.", file=sys.stderr)
    sys.stdout.write(emit_rust(table))


if __name__ == "__main__":
    main()
