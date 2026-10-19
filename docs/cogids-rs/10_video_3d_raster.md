# 10. 3D ソフトウェアラスタライザ

## 10.1 責務

VBlank で確定した描画リスト（[9 章](09_video_3d_geometry.md)）を、256×192 のカラーバッファに描きます。エンジン A は、この結果を BG0 として読みます（[8 章](08_video_2d.md)）。

melonDS のソフトウェアレンダラ（`GPU3D_Soft.cpp`、GPL-3.0-or-later）の移植で、ハードウェアと同じ次の処理を再現します。

- 辺の傾きと丸め（左右の辺で規則が違う）
- パースペクティブ補正の補間（辺は 9 ビット、スパンは 8 ビット精度）
- 深度テスト（Z / W バッファ、less / equal）
- 半透明・影ポリゴンのポリゴン ID 規則
- テクスチャ 7 形式、ラップ / ミラー / クランプ
- シェーディング（モジュレート、デカル、トゥーン、ハイライト）
- エッジマーキング、フォグ、アンチエイリアス（final pass）

同じラスタを N 倍の解像度で動かす機能（[11 章](11_hires.md) が表示専用に使う）も持ちます。

## 10.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/gpu/src/engine_3d/raster/mod.rs` | `Raster`、`Dims`、`Frame`、`render`、`render_with`、`render_frame`、`clear_borders`、`clear_rows`、`band_rows`、`alpha_blend`、`scanline_final_pass`、`Pixels`（`final_pass`、`apply_fog`、`fog_density`） |
| `core/gpu/src/engine_3d/raster/band.rs` | `Band`（`load`、`render`、`final_pass`、`store`）、`Span`、辺の設定（`setup_polygon`、`setup_left_edge`、`setup_right_edge`）、`setup_span`、`for_span`、`render_scanline`、`render_shadow_mask_scanline`、ポリゴン行の描画、`plot_translucent_pixel` |
| `core/gpu/src/engine_3d/raster/slope.rs` | `Interpolator<DIR>`、`Slope<RIGHT>` |
| `core/gpu/src/engine_3d/raster/pixel.rs` | 深度テスト（`for_poly`）、`render_pixel` |
| `core/gpu/src/engine_3d/raster/texture.rs` | `texture_lookup` |
| `core/gpu/src/engine_3d/raster/hires.rs` | 高解像度（11 章） |

## 10.3 データ構造

### 10.3.1 `Raster`

| フィールド | 内容 |
|---|---|
| `dims: Dims` | 解像度倍率に応じたバッファの形。表示領域 `width × height` の周りに 1 画素の枠（エッジマーキングが参照する） |
| `color`, `depth`, `attr` | 2 層（`layer 0` = 最前面、`layer 1` = 辺の下の画素。AA とエッジマーキング用） |
| `bands: Vec<Band>` | スレッドごとのバンド（フレーム間で再利用） |
| `tex`, `pal` / `tex_next`, `pal_next` | テクスチャ画像 VRAM とパレット VRAM の平坦なコピー（現在 / 次のフレーム用） |
| `valid` | 描画済みのフレームを持っているか |
| `generation` | 実際に描いたフレームの数（変わったら新しい画素） |

`Dims`：`pitch = width + 2`（行のピッチ）、`layer`（1 層の大きさ）、`origin`（表示画素 (0, 0) の位置）。`width = 256 × scale`、`height = 192 × scale`。

色の形式：`r | g << 8 | b << 16 | a << 24`（RGB 各 6 ビット、アルファ 5 ビット）。BGR555 からは `expand6` で伸長します（0 以外の成分は `v + 1`）。

### 10.3.2 `Frame`（1 フレームの入力）

描画レジスタ `st: &RenderState`、ポリゴン、描画順 `order`、頂点、テクスチャ画像・パレットのコピー、表示幅。

`RenderState`（VBlank でラッチ）：`disp_cnt`、`alpha_ref`、`clear_attr1`/`clear_attr2`（クリア色・深度・ポリゴン ID・フォグ）、`fog_color`、`fog_offset`、`fog_shift`、`fog_density[34]`、`edge_table[8]`、`toon_table[32]`。

### 10.3.3 `Band`

自分の担当行 `y0..y1` について、両層の行のコピー（各行は全体バッファと同じく 1 画素の枠を含む）と、自分専用の影ステンシルを持ちます。

## 10.4 インタフェース

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `Raster::with_scale(scale)` | `Gpu3D`（等倍）、`Renderer`（高解像度） | N 倍のラスタを作る |
| `render(st, polys, order, verts, vram, changed)` | `Gpu::render_3d_frame`（`core/gpu/src/engine_3d/mod.rs`、ライン 215） | 10.5.1 |
| `render_with(..., tex, pal)` | 高解像度の `Renderer` | 他のラスタ（等倍）のテクスチャのコピーを借りて描く |
| `line(y)` | `render_bg_3d`（BG0 の合成） | 描画済みの 1 行（`256 × scale` 画素、layer 0） |
| `generation()` / `scale()` | 高解像度 | |

## 10.5 処理詳細

### 10.5.1 `render`

```text
render(state, polys, order, verts, vram, changed)
  1. テクスチャ画像 / パレット VRAM を tex_next / pal_next にコピー（Vram::copy_region）。前回と違うか？
  2. changed が false（同じポリゴンとレジスタ）かつテクスチャも同じ → 前のフレームをそのまま使う
  3. render_frame()
  4. generation += 1、valid = true
```

### 10.5.2 `render_frame`（バンド並列）

```text
render_frame(f)
  1. clear_borders()           表示領域の上下 1 行（枠）だけを消去。表示行はバンドが消す
  2. count = 等倍：rayon のスレッド数（1〜8）／ 高解像度：スレッド数 × 2（1〜32）
     lines = height を count で割った切り上げ
  3. バンドごとに並列（rayon の par_iter_mut）
       Band::load()        layer 0 の担当行を clear_rows()（CLEAR_COLOR/DEPTH、または背景ビットマップ）
                           layer 1 は全体バッファからコピー
       Band::render()      バンドに掛かるポリゴンを集め、先頭行で辺を設定し、各行を render_scanline()
       Band::final_pass()  上下の行がともにバンド内にある行の、エッジマーキング・フォグ・AA
       Band::store()       自分の担当範囲（他のバンドと重ならない）だけを書き戻す
  4. 各バンドの先頭行と最終行の scanline_final_pass()（上下のバンドの結果が必要なので最後に逐次）
```

### 10.5.3 1 行の描画（`render_scanline`）

```text
render_scanline(f, y)
  描画順に各ポリゴンについて：
    setup_span(f, rp, y)       辺を行 y まで進め、スパン（左辺・内部・右辺）を計算。
                               右辺が左辺より左なら swapped
    影マスクのポリゴン → render_shadow_mask_scanline()   深度テストに失敗した画素のステンシルを立てる
    それ以外 → ポリゴン行の描画：
       for_span(span, wireframe, f)   3 区間（0 = 左辺、1 = 内部、2 = 右辺）を塗りの規則どおりに
       画素ごと：深度テスト → render_pixel（テクスチャ・シェーディング）→ アルファテスト
                → 不透明なら書き込み、半透明なら plot_translucent_pixel
                → 辺の画素は edge フラグと AA の被覆率
```

### 10.5.4 半透明画素（`plot_translucent_pixel`）

- 同じポリゴン ID の半透明が既に描かれていれば描かない（ポリゴン ID 規則）
- POLYGON_ATTR の設定があれば深度を更新
- DISP3DCNT bit 3 が有効ならアルファブレンド（`alpha_blend`）

### 10.5.5 final pass（`Pixels::final_pass`）

1 行ずつ、上下の行も参照して次を行います。

| 処理 | 条件 | 内容 |
|---|---|---|
| エッジマーキング | DISP3DCNT bit 5 | 上下左右の画素とポリゴン ID が違い、深度が手前の辺の画素を `edge_table[ID >> 3]` の色に |
| フォグ | DISP3DCNT bit 7、fog 属性ビットのある画素 | `fog_density(addr)`：深度と `fog_offset`/`fog_shift` から密度表を補間、`fog_color` を混ぜる |
| アンチエイリアス | DISP3DCNT bit 4 | 辺の画素を、layer 1（辺の下の画素）と被覆率で混ぜる |

final pass が隣の行から読むのは、ポリゴン ID（attr の上位ビット）と深度だけです。どちらも final pass では変わらないので、行をどの順番で処理しても結果は同じです。これが 10.5.2 の並列化の前提です。

### 10.5.6 補間と辺（`slope.rs`）

| 型 | 内容 |
|---|---|
| `Interpolator<DIR>` | `DIR = true` は辺に沿った補間（9 ビット精度）、`false` はスパンに沿った補間（8 ビット精度）。`setup(x0, x1, w0, w1, wbuffer)` で W からパースペクティブの重みを計算、`interpolate(y0, y1)` で属性、`interpolate_z(z0, z1)` で深度（Z バッファは線形、W バッファはパースペクティブ） |
| `Slope<RIGHT>` | 1 本の辺。`RIGHT` で右辺の丸め規則。`setup(...)` が行 y での開始 X を返し、`step()` で 1 行進む。`edge_params(swapped)` は辺の長さと AA の被覆率（bit 31 = x 主体、bit 12-21 = 開始被覆率、bit 0-9 = 増分） |

### 10.5.7 テクスチャ（`texture_lookup`）

`(texparam, texpal, s, t) -> (BGR555, alpha 0〜31)`。形式：A3I5、4 色、16 色、256 色、4×4 圧縮、A5I3、ダイレクトカラー。ラップ / ミラー / クランプは TEXIMAGE_PARAM に従います。テクスチャは `Frame` のコピー（`tex` / `pal`）から読みます。

## 10.6 設計判断と理由

- **バンドに行をコピーして並列に描く。** 4 倍解像度では、逐次の消去・コピー・書き戻し・final pass が約 6 ms、8 バンドでの描画が約 11 ms でした。これらをバンド内に移し、高解像度ではスレッド数の 2 倍のバンドにしたことで、ポリゴンが一部に集中しても空いたスレッドが次のバンドを処理できるようになり、高解像度処理の時間が約半分になりました。出力は以前の逐次処理とビット単位で一致します。
- **辺は各バンドの先頭行で直接設定する。** 上のバンドから辺の状態を引き継がないので、バンドを独立に描けます。
- **補間の `(x * w) << 9` は 64 ビットで計算する。** 高解像度では辺が長くなり、32 ビットであふれていました（2 倍以上で、何もない 3D に、ずれたブロックが出た）。等倍の出力は変わりません。
- **テクスチャ VRAM はフレームごとにコピーする。** 描画中に CPU が VRAM を書き換えても影響を受けません。また、高解像度の `Renderer` が別スレッドで同じテクスチャを使えます。
- **何も変わっていないフレームは描き直さない。** `changed` とテクスチャの比較で、静止画面のコストを下げます。

## 10.7 制約・未実装

- 1 ドットごとのタイミング（ラインバッファの容量不足でポリゴンが欠ける、などのハードの制約）は再現しません。
- ワイヤーフレームは `for_span` の `wireframe` で辺だけを描きます。

## 10.8 関連テスト

- 回帰：headless 例で等倍・2 倍・3 倍 xBRZ・4 倍 xBRZ の PNG を出力し、変更前のビルドと md5 で比較します（[19 章](19_testing.md)）。
- 速度：`HDBENCH` で 3D 描画と高解像度処理の時間を計測します。

## 10.9 ROM 実行テスト（機能 ID `raster3d`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature raster3d` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `polygons_min` | 3D が描かれている |
| `not_blank` | 描画結果が画面に出ている |
| snapshots + baseline | ラスタの変更で画素が 1 つも変わらない（並列化の確認） |

```toml
[[case]]
feature = "raster3d"
name = "raster3d-example"
rom = "my_game"            # [roms] の別名
frames = 2200
press = ["start:2000:60"]
snapshots = [2150]
[case.expect]
polygons_min = 100
not_blank = ["upper", "lower"]
```

`--jobs` を変えて 2 回実行し、baseline が一致すれば、バンド並列がスレッド数に依存しないことも確かめられます。
