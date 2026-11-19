> 注: ここで触れる作図用の元データ(`assets/icons-src/`、`assets/statusbar-src/`)は作者が非公開で管理していて、公開リポジトリには含まれない。ガイドラインは作者の作業メモとして残してある。

# アプリアイコンのガイドライン

Home 画面のアプリアイコンを描くときの決まり。形式の細かい話(色の扱い)は `assets/images/icons/README.md`、
道具は `tools/icons_atlas.py`。

置き場所: 組み込みアプリのアイコンは、そのアプリのフォルダ `apps/<id>/icon.png`(`.desktop` の `Icon=icon.png`)。
共有のアイコン(Folder、File、Trash、NoIcon、Controlpanel と、Harbour に同名のアプリがない公式アプリ用の Notes、
Dictionary、Mail)は `assets/images/icons/`。元データは1枚のアトラスのままで、`unpack` がそれぞれの置き場所へ書く。

## 1. キャンバスと余白

- キャンバスは **32 x 32 px**。絵は **30 x 30 の安全領域**(外側1pxを空ける)の中に収める。
- 本体の大きさの目安は **28 x 28**(外側2pxを空ける)。正方形に近いものはこの大きさ、横長は幅28 x 高さ20〜26、
  縦長は幅20〜26 x 高さ28。迷ったらこの3つのどれかに寄せる。
- 絵の中心をキャンバスの中心に置く。影の分(右と下に2px出る)は絵の一部として数え、本体だけでなく影を含めた
  かたまりで中央に置く。
- 24 px 以下の小さい絵(旧 Dictionary / File など)も、32 px のキャンバスの中央に置く。拡大縮小はしない。

## 2. 線と形

- 線は **1 px**。アンチエイリアスは使わない。
- 角は **2 px の階段**で丸める(1px 欠けて、次の行でもう1px 欠ける)。角を丸くしない形(紙、画面)は直角でもよい。
- 窓・パネル・紙の系統は、**右と下に2pxの市松ディザの影**を付ける(Controlpanel / NoIcon / Preferences が見本)。

## 3. 色(標準は1色)

- 描く色は **1色で、黒**(エディタの白い背景で見やすい)。透明以外は全部テーマのインク色で描かれる。白で描いた古いアイコンも同じように描かれる(アトラスの正本は黒で描いた `builtin-icons-black.png`。`builtin-icons.png` は古い白の版。`python3 tools/icons_atlas.py blacken` は白の版から黒の版を `builtin-icons-black.new.png` に作る。正本は上書きしない)。
- 濃淡はグレーではなく**市松ディザ**(`(x + y)` が偶数のピクセルだけ打つ)で出す。ディザの面は、格子の向きをそろえる。
- 選択中のタイルの絵は、自動で色が変わる(塗り直しは要らない)。`*_selected.png` は作らない。
- 白を塗りに使う4色形式は標準ではない(使う場合は README を見る)。

## 4. 使い回す部品

新しいアイコンは、次の4つのどれかを土台にして、その上に記号を足すと、揃いやすい。

| 部品 | 元 | 使い方 |
| --- | --- | --- |
| フォルダ | `Folder` | 中身がまとまり(Files、Recent など)。タブは左上。 |
| 書類 | `File`(Document) | 文書・データ系。右上の折れ角はそのまま、真ん中に記号を足す。 |
| アイコンのないアプリ | `NoIcon` | 外部アプリで絵がないとき、新アプリの仮アイコン。 |
| 汎用コントロールパネル | `Controlpanel` | 設定・モニター系。上が画面、下がつまみの帯。 |

- 記号を足すときは、**土台の内側に 12 x 12 以内**で、土台の中心に置く(土台の線にぶつけない)。
- 4つの部品は `assets/icons-src/parts.png` に横に並べてある(`python3 tools/icons_atlas.py parts` で作り直せる)。
  これをコピーして使う。

## 5. 描き方の手順

1. `assets/icons-src/builtin-icons-black.png`(アトラスの正本、1行に8個、順番は `builtin-icons.txt`)を開く。
2. `assets/icons-src/guide-overlay.png` を、アトラスの上のレイヤーに重ねる(Photoshop なら「ファイルを配置」)。
   水色=32px の枠、ピンク=30px の安全領域。**書き出す前に、このレイヤーは隠す**。
   書き出しは PNG(透明あり)、拡大縮小なし。キャンバスは 256 x 96(32 の倍数)のまま。鉛筆ツール(1px、アンチエイリアスなし)で描く。
3. 描いたら、切り出して、はみ出しを確認する。

```bash
cd <repo root> && ~/.pyenv/shims/python3 tools/icons_atlas.py unpack && ~/.pyenv/shims/python3 tools/icons_atlas.py check
```

- `check` は、各アイコンの描いた大きさを出し、安全領域にはみ出たものに印を付ける。
- 使わなくなったマスは、`builtin-icons.txt` のその行を `-` にする(書き出さない。後のマスの位置はずれないので、
  そのマスはあとで別の絵に使える)。Fune(公式アプリは自分のアイコンを使う)の行は `-`。
- 新しいアイコンを足すときは、`builtin-icons.txt` に「名前 書き出し先」の行を足す(例: `Paint apps/paint/icon.png`)。
  書き出し先を省いた行は、`icons_atlas.py` の `DEFAULT_DEST`(組み込みアプリ)か `assets/images/icons/<名前>.png`(共有)へ書く。
  `python3 tools/icons_atlas.py names` で、全部の行を書き出し先つきで出せる(`.txt` の書き直しに使う)。
  アプリとの対応は、そのアプリの `.desktop` の `Icon=` で決まる(コードに名前の表はない)。
  アトラスの行が増えるので、PNG の高さも32の倍数に広げる(`icons_atlas.py extend`)。
