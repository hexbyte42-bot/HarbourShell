#!/bin/sh
# Optional Japanese input dictionary for Harbour Shell.
#
# The dictionary is NOT part of this repository, the bundle or the .deb. This script asks the user,
# and only on a "yes" downloads the Mozc OSS dictionary text from a pinned commit (sha256 checked),
# converts it on the device with harbour_ime_dict into ~/.local/share/harbour/ime/harbour-ime.dic,
# and deletes the downloaded text. If anything fails the launcher simply keeps kana-only input.
#
#   ime_install.sh install [--yes]   ask (default: No), download, convert
#   ime_install.sh remove            delete the dictionary (keeps the learning history)
#   ime_install.sh forget            delete the learning history too
#   ime_install.sh status
#
# Mozc (c) Google Inc., BSD-3-Clause; the dictionary data carries the NAIST / ICOT / Okinawa licences. The full
# text is in assets/ime/Mozc-LICENSE.txt and is copied next to the generated dictionary.
set -u

COMMIT=3cf2d38553835f9241479d3b42c7d408c8f45076
BASE=https://raw.githubusercontent.com/google/mozc/$COMMIT/src/data/dictionary_oss
SUMS="9e07ce292932c4fff327ed8cb06274c09acca556bd8bdf7c7ef1f6173eae3b61  dictionary00.txt
836fd5fd8399510de0d8ef7b33be913cca6bbaee7b712cc8045fba03f4c75a89  dictionary01.txt
93943858d0cfd092b849a78def6901eaf228177e1ac5663544e865a3a1e45a9e  dictionary02.txt
cb9f1b0b4a7550d682d86244a021cd17e0b15366328819e69135b5f183a1a24b  dictionary03.txt
9fdc4e3948d03463de13ab5782adffbc11dd06e934cb80138c5920029cd3d2f6  dictionary04.txt
199e3b0f050b691823ae7ba943090d437f51af6436b27c5f3dc69cafa12000e4  dictionary05.txt
b95b14f7770ae2a824bc8e9fad8b03c1bc45c504f93a36554b8b56daa0368d2a  dictionary06.txt
7f53649b529d4c8c495d3a8dd9c4d489dfe7696080f7423107ff01331174be48  dictionary07.txt
50a31bb5fc1646721b00be371a5695dffab1292bda143e1e59cc70cbbdd8643f  dictionary08.txt
4e5fcd4cbfbf11395166a37b506f547fc54d20edc0884f5184ecd3fc705f0d9a  dictionary09.txt"

HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${HARBOUR_IME_DIR:-$HOME/.local/share/harbour/ime}
DIC=$OUT/harbour-ime.dic

find_first() { # find_first <relative path>...  (first existing file wins)
  for rel in "$@"; do
    for root in "$HERE" "$HERE/.." "$HOME/harbour-shell" /usr/share/harbour-shell; do
      [ -f "$root/$rel" ] && { echo "$root/$rel"; return 0; }
    done
  done
  return 1
}

say() { printf '%s\n' "$*"; }

fetch() { # fetch <url> <file>
  if command -v curl >/dev/null 2>&1; then curl -fsSL --retry 2 --connect-timeout 15 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then wget -q -O "$2" "$1"
  else return 127; fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

ask() {
  # Default is No; no terminal means No.
  [ -t 0 ] || return 1
  say ""
  say "日本語入力辞書をダウンロードしますか？ / Download the Japanese input dictionary?"
  say "  Mozc OSS 辞書 (Google, BSD-3 + NAIST/IPAdic ほか) を GitHub から約 63MB 取得し、"
  say "  この機器で約 33MB に変換して $OUT に置きます。取得した元ファイルは変換後に削除します。"
  say "  辞書は配布物に含まれない第三者データで、ライセンスは $OUT/Mozc-LICENSE.txt にあります。"
  say "  ダウンロードしない場合はかな入力のみ使えます。(後で: harbourctl ime-install)"
  printf '  [y/N] '
  read -r ans || return 1
  case "$ans" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

do_install() {
  case "${1:-}" in --yes|-y) ;; *) ask || { say "辞書は入れません (かな入力のみ) / Skipping the dictionary."; return 0; } ;; esac
  GEN=${HARBOUR_IME_GEN:-$(find_first bin/harbour_ime_dict)} || { say "ime: harbour_ime_dict が見つかりません。辞書なしで続けます。" >&2; return 1; }
  LIC=$(find_first assets/ime/Mozc-LICENSE.txt) || { say "ime: ライセンス文書 (assets/ime/Mozc-LICENSE.txt) が見つかりません。辞書なしで続けます。" >&2; return 1; }
  mkdir -p "$OUT" || return 1
  TMP=$(mktemp -d "$OUT/.dl.XXXXXX") || return 1
  trap 'rm -rf "$TMP"' EXIT INT TERM
  FILES=""
  say "$SUMS" | while read -r sum name; do
    say "  downloading $name ..."
    fetch "$BASE/$name" "$TMP/$name" || { say "ime: ダウンロードに失敗しました ($name)。辞書なしで続けます。" >&2; exit 1; }
    [ "$(sha256_of "$TMP/$name")" = "$sum" ] || { say "ime: $name のsha256が一致しません。破棄して辞書なしで続けます。" >&2; exit 1; }
  done || return 1
  for n in 00 01 02 03 04 05 06 07 08 09; do FILES="$FILES $TMP/dictionary$n.txt"; done
  say "  converting (しばらくかかります) ..."
  # shellcheck disable=SC2086
  "$GEN" build "$TMP/harbour-ime.dic" $FILES || { say "ime: 変換に失敗しました。辞書なしで続けます。" >&2; return 1; }
  rm -f $FILES                      # the downloaded text is not kept
  cp "$LIC" "$OUT/Mozc-LICENSE.txt"
  mv "$TMP/harbour-ime.dic" "$DIC" || return 1
  say "ime: 辞書を入れました / dictionary installed: $DIC ($(du -h "$DIC" | cut -f1))"
  say "    ランチャーを再起動すると有効になります (harbourctl try)。"
}

case "${1:-status}" in
  install) shift; do_install "${1:-}" ;;
  remove)
    rm -f "$DIC" "$OUT/Mozc-LICENSE.txt" "$OUT"/.dl.* 2>/dev/null; rm -rf "$OUT"/.dl.* 2>/dev/null
    say "ime: 辞書を削除しました / dictionary removed (learning history kept: $OUT/learn)" ;;
  forget) rm -f "$OUT/learn"; say "ime: 学習履歴を削除しました / learning history removed" ;;
  status)
    if [ -f "$DIC" ]; then say "ime: dictionary installed: $DIC ($(du -h "$DIC" | cut -f1))"; else say "ime: no dictionary (kana input only)"; fi
    [ -f "$OUT/learn" ] && say "ime: learning history: $(wc -l < "$OUT/learn") entries" ;;
  *) say "usage: ime_install.sh {install [--yes]|remove|forget|status}" >&2; exit 2 ;;
esac
