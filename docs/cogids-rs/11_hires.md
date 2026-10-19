# 11. 高解像度出力と xBRZ

## 11.1 責務

画面を 2〜4 倍（内部的には最大 6 倍、3D は最大 8 倍）の解像度で表示します。

- 2D：等倍の画面を xBRZ（またはニアレストネイバー）で拡大
- 3D：等倍で描いたフレームの入力（ポリゴン・頂点・テクスチャ・描画レジスタ）を写し取り、同じラスタライザを N 倍で動かして、エンジン A の画面の 3D 画素だけを置き換える

**エミュレートしているハードウェアは常に等倍のままです。** BG0 の 3D、ディスプレイキャプチャなど、ゲームが読めるものはすべて 256×192 です。高解像度の画像は表示のためだけに、別に作ります。

## 11.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/gpu/src/display/hires.rs` | `HdFrame`、`HdLine`、`hd_3d_info`、`Gpu::hd_frame`、`Gpu::screen_hd`、`HdFrame::screen`、`overlay_hires_3d`、`shade_3d`、`master_bright`、`xbrz`、`nearest` |
| `core/gpu/src/engine_3d/raster/hires.rs` | `Hires`（`Gpu3D` が所有）、`Snapshot`、`Renderer`、`capture`、`scaled_bounds` |
| `core/gpu/src/engine_3d/mod.rs` | `Gpu::set_render_scale`、`Gpu::hires_3d`、`Gpu::render_3d_frame`（最後に `hires.capture`） |
| `core/gpu/src/engine_2d/compose.rs` | `composite` が `hd_3d` に 3D 画素の合成方法を記録 |
| `core/gpu/src/display/output.rs` | `draw_scanline_for_engine` が `hd_lines` にラインごとの状態を記録 |
| `gui/common/src/emu_thread.rs`, `gui/egui/src/hd.rs` | 別スレッドで `HdFrame::screen` を呼ぶ HD ワーカー（[17 章](17_frontend_common.md)、[18 章](18_gui.md)） |

## 11.3 データ構造

### 11.3.1 等倍の描画中に記録するもの

| 記録先 | 単位 | 記録する関数 | 内容 |
|---|---|---|---|
| `Gpu::hd_3d: Vec<u32>` | エンジン A の画素 | `compose.rs` の `composite` | 表示画素が 3D か、その下の層の色、合成方法（11.3.2） |
| `Gpu::hd_lines: [HdLine; 192]` | エンジン A のライン | `output.rs` の `draw_scanline_for_engine` | `graphics`（表示モード 1 か）、`master_bright`、`bg0_hofs` |
| `Hires::snapshot: Option<Arc<Snapshot>>` | 3D のフレーム | `Hires::capture`（`render_3d_frame` の最後） | 描画レジスタ、使ったポリゴン・頂点、描画順、テクスチャ画像・パレットのコピー、等倍ラスタの `generation` |

`capture` は、倍率が 1 のとき、または等倍ラスタの `generation` が前回の写しと同じとき（3D が変わっていない）は何もしません。写すポリゴンと頂点は、描画順に現れる最大の索引までに絞ります。

### 11.3.2 `hd_3d` のビット（`hd_3d_info`）

| ビット | 定数 | 意味 |
|---|---|---|
| 31 | `HD_3D` | 表示画素が 3D（エンジン A の BG0 が最前面） |
| 30 | `HD_BLEND` | 3D 画素が下の層とアルファブレンドされた |
| 16-17 | `HD_EFFECT_SHIFT` | BLDCNT の効果（2 = 明るく、3 = 暗く） |
| 18-22 | `HD_EVY_SHIFT` | EVY |
| 0-14 | | 下の層の色（BGR555） |

### 11.3.3 `HdFrame`

`Gpu::hd_frame()` が作る、1 フレーム分の**所有データだけ**の構造体です。別スレッドへ送れます。

| フィールド | 内容 |
|---|---|
| `upper`, `lower` | 両画面の等倍画像（`get_upper_frame` / `get_lower_frame`） |
| `engine_a_upper` | エンジン A（3D）が上画面か |
| `bg_3d` | エンジン A の DISPCNT bit 3 |
| `info` | `hd_3d` のコピー |
| `lines` | `hd_lines` のコピー |
| `snapshot` | `Arc<Snapshot>`（コピーではなく共有） |
| `scale_3d` | スナップショットを描く倍率（`Hires::scale`） |

`hd_frame()` のコストは、2 画面分と `info` のコピーで約 0.1 ms です。

### 11.3.4 `Hires` / `Renderer`

| 型 | 内容 |
|---|---|
| `Hires { scale, snapshot, renderer }` | `set_scale(n)`：1〜8 に制限。変わったら `renderer` を作り直し、1 ならスナップショットを捨てる |
| `Renderer` | N 倍の `Raster` と、それが描いたスナップショットの世代を持つ。`render(snap, scale)` は新しいフレームのときだけ描き直す |

`Renderer::render` は、頂点の画面位置（`final_position`）を `(hires_position × scale) >> 4` で置き換え、`scaled_bounds` でポリゴンの上下の頂点と Y の範囲を求め直し（ジオメトリエンジンのポリゴン設定と同じ規則）、等倍と同じラスタライザを `scale × 256 × 192` で動かします。テクスチャはスナップショットのコピーを使います（`render_with`）。

## 11.4 インタフェース

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `Gpu::set_render_scale(n)` | エミュレータスレッド・egui（毎フレーム） | `Hires::set_scale` |
| `Gpu::hd_frame()` | エミュレータスレッド・egui（2 倍以上で、表示するフレームだけ） | 11.3.3 |
| `HdFrame::screen(renderer, upper, scale, xbrz, out)` | HD ワーカー | 11.5。戻り値は幅（画素） |
| `Gpu::screen_hd(upper, scale, xbrz, out)` | headless 例・テスト | `hd_frame()` + `screen()`（`Hires` 内の `Renderer` を使う） |
| `Gpu::hires_3d()` | デバッグ | N 倍の 3D フレームだけ |

## 11.5 処理詳細（`HdFrame::screen`）

```text
screen(renderer, upper, scale, xbrz, out)
  scale = clamp(1, 6)
  native = upper ? self.upper : self.lower
  scale == 1 → native をそのまま返す
  1. 2D の拡大
       xbrz → xbrz(native, scale)
                192 行を 8 バンド（XBRZ_BANDS）に分けて rayon で並列。
                各バンドに上下 4 行（XBRZ_OVERLAP）の隣接行を足して xbrz::scale_rgba に渡し、
                自分の行だけを残す
       それ以外 → nearest(native, scale)
  2. 3D の重ね合わせ（次の条件をすべて満たすとき）
       この画面がエンジン A（upper == engine_a_upper）、bg_3d、scale_3d == scale、snapshot あり
       hi = renderer.render(snapshot, scale)
       overlay_hires_3d(hi, scale, out)
```

`overlay_hires_3d`：等倍の 1 行（出力の `scale` 行）を 1 タスクとして rayon で並列に処理します。表示モード 1 のラインだけを対象に、`info` の `HD_3D` が立った画素について、`render_bg_3d` と同じスクロール（BG0HOFS、9 ビット）で高解像度 3D の `scale × scale` ブロックを取り出し、`shade_3d` → `master_bright` を通して置き換えます。

`shade_3d(c, info)`：

- 3D 画素のアルファが 0 → 下の層の色
- `HD_BLEND` → `eva = alpha + 1`、`blend32(color, below, eva, 32 - eva)`
- それ以外 → BLDCNT の効果（明るく / 暗く、EVY）

等倍の合成（[8 章](08_video_2d.md) 8.5.2）と同じ計算なので、等倍で見えていた色と一致します。

## 11.6 設計判断と理由

- **表示専用にする。** ハードウェアを高解像度にするとキャプチャやゲームが読む値が変わり、互換性が下がります。表示だけを別経路にすれば、等倍の出力はビット単位で変わりません。
- **入力の写しを取り、描くのは表示するときだけ。** 早送りで表示されないフレームのコストは写しだけです。さらに、エミュレータスレッドは前回の画像を取りに来ていないフレームの `hd_frame()` を作りません（[17 章](17_frontend_common.md)）。
- **`HdFrame` は所有データだけにする。** 別スレッドの HD ワーカーで拡大でき、エミュレーション側の負担は `hd_frame()` のコピーだけになります。
- **xBRZ は 8 バンド、重なり 4 行。** xBRZ は 2 行先まで参照するので、4 行の重なりでバンドの境目が画像全体を一度に処理した場合と同じになります。
- **3D の解像度と表示倍率が一致するときだけ重ねる。** 倍率が違うと 3D のブロックが合いません。GUI は表示倍率と同じ値を `set_render_scale` に渡します。

## 11.7 計測結果（参考）

3D のタイトル画面（約 620 ポリゴン）、4 倍 + xBRZ、16 スレッド環境、両画面の拡大時間（300 フレーム平均）：変更前 23.9 ms → 変更後 11.8 ms。主な改善点は、3D ラスタの消去・コピー・final pass のバンド内並列化、バンド数の増加、3D 合成の行並列化、xBRZ のバンド数 4→8、RGBA 変換の単純化、HD 処理の別スレッド化です。

## 11.8 制約・未実装

- 拡大処理が 1 フレームの時間より長い環境では、**表示**の更新が 60 回/秒を下回ります（ゲームの速度と音は落ちません）。
- Tauri は拡大後の RGBA（4 倍で約 6 MB/フレーム）を IPC で送るため、高倍率ではこれも上限になります。
- エンジン B の 3D（キャプチャ経由で表示する 3D など）は高解像度になりません。キャプチャした画像は等倍のまま拡大されます。

## 11.9 関連テスト

- `core/gpu/src/display/hires.rs` のユニットテスト：`nearest_scales_rows_and_columns`、`xbrz_keeps_flat_areas`、`xbrz_bands_match_a_whole_image_pass`（バンド分割が全体処理と一致）、`shading_matches_native_rules`（`shade_3d` が等倍の規則と一致）
- headless 例：等倍の画像が変更前とビット一致すること、2 倍・3 倍 xBRZ・4 倍 xBRZ の画像の確認、`HDBENCH=<倍率>[:x]` による時間計測（[19 章](19_testing.md)）

## 11.10 ROM 実行テスト（機能 ID `hires`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature hires` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| hd / xbrz | 指定した倍率の画面を作り、`_hdN.png` に出力する |
| `polygons_min` | 高解像度 3D の元になる 3D がある |
| （自動）baseline | 等倍の画面が変わらない（11.6 の「表示専用」の確認）と、高解像度の画面の回帰 |

```toml
[[case]]
feature = "hires"
name = "hires-example"
rom = "my_game"            # [roms] の別名
frames = 2200
press = ["start:2000:60"]
snapshots = [2150]
hd = 3
xbrz = true
[case.expect]
polygons_min = 100
```

等倍の baseline は `hd` を付けたケースでも記録されます。高解像度の変更で等倍のハッシュが変わったら、ハードウェア側に影響が出ています。
