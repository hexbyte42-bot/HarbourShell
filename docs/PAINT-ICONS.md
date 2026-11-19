# Paint のツールバーアイコン

Paint の左のツールバー(2列 x 6行)に出る12個のツールアイコン。1枚のアトラスに描いて、スクリプトで1個ずつの PNG に切り出す。

## 流れ

1. `assets/icons-src/paint-tools-atlas.png`(**72 x 24 px**)に描く。12 x 12 px のマスを **横6 x 縦2** に並べ、
   左上から右へ、2段目も左から右へ、次の順で1マスにつき1つ入れる。

   | 段 | 1 | 2 | 3 | 4 | 5 | 6 |
   | --- | --- | --- | --- | --- | --- | --- |
   | 1段目 | Pencil | Brush | Eraser | Hand | Line | Rectangle |
   | 2段目 | Rounded Rect | Ellipse | Bucket | Spray | Text | Select |

2. `python3 tools/paint_icons_split.py` を実行すると、Paint のフォルダの `apps/paint/res/tools/<id>.png` が12個(`t_pencil`、`t_brush`、
   `t_eraser`、`t_hand`、`t_line`、`t_rect`、`t_round`、`t_ellipse`、`t_fill`、`t_spray`、`t_text`、`t_select`)できる。
3. ランチャーを再起動(または再ビルド)すると変わる(Paint は `app/paint/res/tools/<id>` として読む)。

何も描いていない(全部透明の)マスは、PNG を作らない。そのツールは、これまでどおりコードで描いた記号が出る。

## 大きさと色

- 1マスは **12 x 12 px**(ツールバーの1セルは 18 x 16 で、罫線を除くと 16 x 14。その中央に置かれる)。
- 色は **黒・白・透明**の3つ。黒はそのときのテーマのインク色、白はセルの地の色(選択中のツールでは反転する)、
  透明はセルの地がそのまま見える。グレーは使わない(黒か白に寄せられる)。濃淡は市松ディザで出す。
- 線は 1 px、アンチエイリアスなし。`docs/ICON-GUIDELINE.md` の線の決まりに合わせる。
- 白は、四角や楕円の中身、消しゴムの面のように、地の色と同じに見せたい所に使う。
