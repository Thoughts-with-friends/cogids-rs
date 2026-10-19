# 8. 2D 映像（VRAM・2D エンジン・画面出力・キャプチャ）

## 8.1 責務

- VRAM バンク A〜I を、BG・OBJ・テクスチャ・拡張パレット・LCDC・ARM7 の各領域へ割り当てる
- 2D エンジン A（メイン、BG0 に 3D を表示できる）と B（サブ）で、ラインごとに BG と OBJ を描き、ウィンドウと色効果で合成する
- 表示モード（オフ / 通常 / VRAM 表示）、マスター輝度、上下画面の割り当て（POWCNT1 bit 15）
- ディスプレイキャプチャ（エンジン A の出力や 3D を VRAM へ書き戻す）

ゲームから読めるものは、すべて等倍 256×192 のまま扱います。高解像度表示は [11 章](11_hires.md) の別経路です。

## 8.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/gpu/src/gpu.rs` | `Gpu`（両エンジン・3D・VRAM・パレット・OAM を所有）、`power_on`、`start_frame`、`is_frame_complete`、`get_upper_frame`/`get_lower_frame` |
| `core/gpu/src/vram.rs` | `Vram`、`Region`、`BANK_SIZES`、`remap`、`copy_region`、`arm9_region` |
| `core/gpu/src/memory.rs` | パレット・OAM・VRAM の CPU 側／エンジン側アクセサ |
| `core/gpu/src/io.rs` | GPU のレジスタ取得・設定（`get_dispcnt_a` など） |
| `core/gpu/src/engine_2d/mod.rs` | `Gpu2DEngine`、レジスタの設定・取得、`vblank_start` |
| `core/gpu/src/engine_2d/regs.rs` | `DispcntReg`、`DispcapcntReg`、`WininReg`、`WinoutReg`、`BldcntReg` |
| `core/gpu/src/engine_2d/bg.rs` | BG0〜3（テキスト・アフィン・拡張・大型ビットマップ・3D） |
| `core/gpu/src/engine_2d/obj.rs` | OBJ（通常・アフィン・ビットマップ・OBJ ウィンドウ） |
| `core/gpu/src/engine_2d/window.rs` | WIN0 / WIN1 / OBJWIN / 外側 |
| `core/gpu/src/engine_2d/compose.rs` | `compose_scanline`、`composite`、色演算 |
| `core/gpu/src/display/output.rs` | `draw_scanline`、`draw_scanline_for_engine`（表示モード・キャプチャ・マスター輝度） |
| `core/gpu/src/display/regs.rs` | `SchedulerEvent`、`DispStatReg`、`PowerCtrlReg` |
| `core/system/src/emulator/gpu/io_2d.rs` | 2D レジスタの影コピー（[5 章](05_memory_bus.md) 5.7） |

## 8.3 データ構造

### 8.3.1 `Gpu`

| フィールド | 内容 |
|---|---|
| `engine_upper`, `engine_lower` | エンジン A、B（名前は CorgiDS のまま。実際の上下は POWCNT1 bit 15 で決まる） |
| `engine_3d` | `Gpu3D`（9〜11 章） |
| `vram` | `Vram` |
| `palette_upper`, `palette_lower` | エンジン A・B 用、各 1 KiB（BG 512 バイト + OBJ 512 バイト） |
| `oam` | 2 KiB（エンジン A 1 KiB + B 1 KiB） |
| `display_status_arm7`, `display_status_arm9` | CPU ごとの DISPSTAT |
| `vertical_count` | VCOUNT |
| `power_control_reg` | POWCNT1（LCD、エンジン A、3D 描画、3D ジオメトリ、エンジン B、bit 15 = A を上画面へ） |
| `frame_complete`, `frames_skipped` | フレーム完了フラグ、スキップしたフレーム数 |
| `hd_3d`, `hd_lines` | 高解像度用の記録（[11 章](11_hires.md)） |

`power_on` 後の POWCNT1 は `0x820F`（両エンジンと 3D が ON、エンジン A が上画面）です。

### 8.3.2 VRAM のページ割り当て（`Vram`）

各領域を 16 KiB のページに分け、ページごとに「割り当てられたバンクのビットマスク」（bit 0 = A … bit 8 = I）を持ちます（melonDS `GPU::VRAMMap_*` と同じ方式）。読み出しは重なったバンクの OR、書き込みは全バンクへ行います。バンクは常に自分のサイズに揃った位置に割り当てられるので、バンク内の位置は `offset & (bank_size - 1)` で求まります。

| バンク | サイズ | MST=0 | MST=1 | MST=2 | MST=3 | MST=4 | MST=5 |
|---|---|---|---|---|---|---|---|
| A, B | 128 KiB | LCDC | BG-A（OFS×128K） | OBJ-A（(OFS&1)×128K） | テクスチャ（OFS×128K） | | |
| C | 128 KiB | LCDC | BG-A | ARM7（(OFS&1)×128K） | テクスチャ | BG-B | |
| D | 128 KiB | LCDC | BG-A | ARM7 | テクスチャ | OBJ-B | |
| E | 64 KiB | LCDC | BG-A | OBJ-A | テクスチャパレット | BG 拡張パレット A（32 KiB） | |
| F, G | 16 KiB | LCDC | BG-A（(OFS&1)×16K + (OFS>>1)×64K） | OBJ-A（同） | テクスチャパレット（スロット (OFS&1)+(OFS>>1)×4） | BG 拡張パレット A（(OFS&1)×16K） | OBJ 拡張パレット A |
| H | 32 KiB | LCDC | BG-B | BG 拡張パレット B | | | |
| I | 16 KiB | LCDC | BG-B（+32K） | OBJ-B | OBJ 拡張パレット B | | |

VRAMCNT の bit 7 が 0 のバンクは割り当てません。`set_cnt` のたびに `remap()` で全ページ表を作り直します。

| 領域（`Region`） | 大きさ | CPU アドレス |
|---|---|---|
| `BgA` | 512 KiB | `0x06000000` |
| `BgB` | 128 KiB | `0x06200000` |
| `ObjA` | 256 KiB | `0x06400000` |
| `ObjB` | 128 KiB | `0x06600000` |
| `Lcdc` | 656 KiB | `0x06800000` |
| `Arm7` | 256 KiB | ARM7 の `0x06000000` |
| `TexImage` | 512 KiB（スロット 0〜3） | 3D エンジンのみ |
| `TexPal` | 96 KiB（128 KiB 空間、スロット 0〜5） | 3D エンジンのみ |
| `BgExtPalA/B` | 32 KiB（スロット 0〜3） | エンジンのみ |
| `ObjExtPalA/B` | 8 KiB | エンジンのみ |

### 8.3.3 合成用のラインバッファ（`compose.rs`）

各レイヤをまず自分のラインバッファに描き、画素ごとに優先度順で合成します（melonDS `GPU2D_Soft` と同じ）。画素値は下位 15 ビットが BGR555、上位ビットがフラグです。

| フラグ | ビット | 意味 |
|---|---|---|
| `OPAQUE` | 31 | 不透明画素あり |
| `OBJ_SEMI` | 30 | 半透明 OBJ（OBJ モード 1） |
| `OBJ_BITMAP` | 29 | ビットマップ OBJ（bit 20-23 にアルファ） |
| `BG_3D` | 28 | BG0 の 3D 画素（bit 16-20 に 5 ビットアルファ） |

`LineLayers` の OBJ 画素は bit 16-17 に OBJ の優先度を持ちます。

## 8.4 インタフェース

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `Gpu::draw_scanline()` | `gpu_handle_event`（表示ラインの HBlank 開始、スキップ時を除く） | POWCNT1 で有効なエンジンそれぞれに `draw_scanline_for_engine` |
| `Gpu2DEngine::vblank_start()` | `gpu_handle_event`（ライン 192） | BG2/BG3 のアフィン参照点を BGxX/BGxY から再読み込み、キャプチャの終了 |
| `Gpu::get_upper_frame(buf)` / `get_lower_frame(buf)` | `Emulator::get_upper_frame` ほか（GUI） | POWCNT1 bit 15 に従い、上/下画面のエンジンの `front_framebuffer` を `0x00RRGGBB` でコピー |
| `Vram::set_cnt(bank, value)` | VRAMCNT_A〜I の書き込み | 割り当てを作り直す |
| `Vram::copy_region(region, out)` | 3D ラスタ（テクスチャのスナップショット） | 領域全体を利用者から見た配置でコピー |

## 8.5 処理詳細

### 8.5.1 1 ラインの描画

```text
draw_scanline_for_engine(engine)
  1. compose_scanline(engine)
       a. 有効な BG ごとに、BG モード（DISPCNT bit 0-2）に従って描く
            エンジン A の BG0 で 3D が有効 → render_bg_3d（3D ラスタの 1 行、BG0HOFS でスクロール）
            テキスト / アフィン / 拡張（256 色・ダイレクトカラー）/ 大型ビットマップ（BG モード 6）
       b. OBJ 有効 → render_objs()       OBJ と OBJ ウィンドウのマスク
       c. composite()                   ウィンドウのマスク（window_masks）、優先度の上位 2 レイヤ、
                                        BLDCNT（アルファ / 明るく / 暗く）
       d. advance_affine_refs()         BG2/BG3 の参照点に PB/PD を加算
  2. DISPCNT の表示モード
       0 オフ    → 白
       1 通常    → 合成結果
       2 VRAM    → LCDC バンクの行（BGR555 → RGB888）
       3 メインメモリ FIFO → 未実装
     → front_framebuffer の行
  3. エンジン A でキャプチャ有効（DISPCAPCNT）
       ソース A（2D 出力または 3D）とソース B（VRAM）を EVA/EVB で混ぜて、
       書き込みバンク・オフセットへ。サイズ 128×128〜256×192
  4. MASTER_BRIGHT（bit 14-15 モード、bit 0-4 係数）で明るく / 暗く
```

### 8.5.2 色の計算

| 演算 | 式 | 用途 |
|---|---|---|
| BGR555 → 0xFFRRGGBB | 5 ビットを 8 ビットへ伸長 | 出力 |
| アルファブレンド | `(a × eva + b × evb) / 16`、成分ごとに飽和 | BLDALPHA |
| 3D のブレンド | 5 ビットの重み（`eva + evb == 32`） | 3D 画素と下のレイヤ |
| 明るく | `c + (31 - c) × evy / 16` | BLDY |
| 暗く | `c - c × evy / 16` | BLDY |

### 8.5.3 ウィンドウ

`window_masks(engine, line, layers)` が、画素ごとの許可マスク（bit 0-3 = BG0〜3、bit 4 = OBJ、bit 5 = 色効果）を作ります。範囲 `[start, end)` は `start > end` のとき折り返します（実機と同じ）。優先度は WIN0 > WIN1 > OBJWIN > 外側です。

### 8.5.4 OBJ

`render_objs` は、ラインに掛かるすべての OBJ を描きます。優先度の値が小さいほうが勝ち、同じなら OAM の番号が小さいほうが勝ちます。サイズ表 `OBJ_SIZES[shape][size]` は GBATEK のとおりです。

## 8.6 設計判断と理由

- **両エンジンを毎ライン描く。** 以前は `if engine_upper { .. } else if engine_lower { .. }` の分岐で、1 ラインに 1 エンジンしか描いていませんでした。先に有効だったエンジンが常に勝ち、もう一方の画面はずっと初期色のままでした。CorgiDS の `GPU::draw_scanline()` と同じく、各エンジンは自分の POWCNT1 ビットだけで判定します。
- **VRAMCNT は 16 ビット書き込みでも 2 つのバイトレジスタとして扱う。** あるソフトは `0x04000240` と `0x04000242` をハーフワードで書きます。これを捨てていたため、VRAM が全く割り当てられませんでした（`arm9_write_halfword`）。
- **重なったバンクは OR / 全書き込み。** 実機の挙動で、一部のゲームはバンクを重ねて割り当てます。
- **DISPCAPCNT bit 31 はハードが下ろす。** 部分書き込みの前に影コピーを GPU の値で更新します（[5 章](05_memory_bus.md) 5.7）。

## 8.7 制約・未実装

- 表示モード 3（メインメモリ表示 FIFO）は未実装です（DMA の timing 4 も未実装。[7 章](07_dma.md)）。
- モザイク（MOSAIC）は値を保存するだけで、描画に反映していません。
- `Config::bg_enable`（BG ごとの表示切り替え）は CorgiDS 由来で、`power_on` で全 `true` にしています。

## 8.8 関連テスト

- headless 例で、3D のタイトル画面と 3D のステージ紹介を PNG に出力し、変更前のビルドと md5 で比較します（[19 章](19_testing.md)）。

## 8.9 ROM 実行テスト（機能 ID `video2d`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature video2d` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `not_blank` | 両エンジンが描かれている（8.6 の「片方だけ描かれる」不具合の再発検知） |
| snapshots + baseline | BG・OBJ・ウィンドウ・合成の画面が変わらない |

```toml
[[case]]
feature = "video2d"
name = "video2d-example"
rom = "my_game"            # [roms] の別名
frames = 900
snapshots = [300, 600]
[case.expect]
not_blank = ["upper", "lower"]
```

2D の変更は、見た目の確認に `local/results/run-*/ケース名/*.png` を、回帰の確認に baseline を使います。
