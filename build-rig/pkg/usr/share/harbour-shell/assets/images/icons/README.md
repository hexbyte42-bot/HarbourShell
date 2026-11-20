> The drawing sources (`assets/icons-src/`, `assets/statusbar-src/`) are kept private; only the finished icons are in the repository.

# Home アイコンの描き方

描き方の決まり(余白・線・ディザ・使い回す部品)は **`docs/ICON-GUIDELINE.md`**。

**標準は1色(塗りなし)**: 32 x 32 px の PNG を、線とディザだけで描く。描く色は白か黒のどちらか1色で、透明以外は全部テーマのインク色で描かれる(白と透明だけの PNG、または黒と透明だけの PNG は、塗り分けのない1色として扱う)。ディザは市松で描く。

以下は、必要なアイコンだけ使えるオプションの4色形式(白を塗りに使うなど)。標準ではないので、使わなくてよい。

使える色は4つ(ステータスバーと同じ考え方):

| 描く色 | 判定 | 画面での色 |
| --- | --- | --- |
| 透明 | alpha < 128 | 何も描かない |
| **黒** | 輝度 < 64 | `foreground`(インク) |
| **グレー** | 輝度 64〜192 | `muted`(控えめな色) |
| **白** | 輝度 > 192 | `icon_white`(テーマごと。明るいテーマは最も明るい色、暗いテーマは背景より少し明るい色) |

- 中間の色はグレー扱い。アンチエイリアスは使わず、4色だけで塗る。
- 選択中のタイル(`select_bg` の板の上): 黒 → `select_fg`、グレー → 板と文字色の中間、白 → 板の色(=塗りなし)。
  そのため `*_selected.png` は不要(ファイルが残っていても使われない)。
- **旧形式との互換**: 白と透明だけで描かれた PNG(旧1ビット)は「全部インク色」で描く。黒かグレーを1ピクセルでも使うと新形式になり、白は `icon_white` で塗られる。
- 外部アプリの自動生成アイコン(`tile/<id>`)はこの形式ではなく、元アイコンのグレー濃淡マスクをインク色で描く。

実装: `src/assets.rs` の `split_layers` / `blit_icon`、パレットは `src/launcher/home.rs`。

## アイコン一覧

ここにあるのは共有のアイコン: Controlpanel, Dictionary(24px), File(24px), Folder, Mail, NoIcon, Notes, Trash
(試作用の Test / Test2 / Test3 は非公開)。

組み込みアプリのアイコンは、それぞれのアプリのフォルダ `apps/<id>/icon.png` にある(`.desktop` の `Icon=`)。
外部アプリとの対応は `src/launcher/icons.rs` の `known_icon_for`(パッケージの名前と別名、それ以外は Notes /
Dictionary / Mail)。Fune などの公式アプリは、そのアプリの `.desktop` の `Icon=` を、ほかの外部アプリと同じように変換して使う。
1色のまま: すべてのアイコン(Controlpanel / NoIcon / Preferences / Timer は手描きの黒を1色にして塗りを抜いた)。
