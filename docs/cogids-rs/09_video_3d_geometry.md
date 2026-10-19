# 9. 3D ジオメトリエンジンと GX FIFO

## 9.1 責務

ARM9 が送る GX コマンド（行列・頂点・ポリゴン属性・ライト・テストなど）を、FIFO を経由して CPU 時間に合わせて実行し、ポリゴン RAM / 頂点 RAM を作ります。`SWAP_BUFFERS` の後、VBlank でバンクを入れ替えて描画リストを確定し、ライン 215 でラスタライザ（[10 章](10_video_3d_raster.md)）に渡します。

GX FIFO の割り込みと GX FIFO DMA の起動条件も、ここで扱います。

## 9.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/system/src/emulator/gpu/gpu_3d.rs` | `write_gxfifo`、`write_fifo_direct`、`write_command`、`read_command`、`check_fifo_irq`、`check_fifo_dma`、`request_fifo_dma`、`set_gxstat`、`gpu_handle_event` |
| `core/system/src/emulator/gpu/run.rs` | `run_3d`、`gpu3d_run` |
| `core/system/src/emulator/gpu/execute.rs` | `exec_command`、`geometry_pipeline_cycles` |
| `core/gpu/src/engine_3d/gxfifo.rs` | `CMD_PARAM_AMOUNTS[256]`、`CMD_CYCLE_AMOUNTS[256]` |
| `core/gpu/src/engine_3d/state.rs` | `Gpu3D`、`Vertex`、`Polygon`、`GxCommand`、`RenderState`、`MAX_VERTICES`、`MAX_POLYGONS`、`Mtx` |
| `core/gpu/src/engine_3d/regs.rs` | `is_3d_reg`、`write_reg`、`read_reg`、`get_gxstat`、`write_gxstat`、`end_of_frame` |
| `core/gpu/src/engine_3d/geometry/mod.rs` | `execute_command`、`update_clip_matrix`、`set_viewport` |
| `core/gpu/src/engine_3d/geometry/matrix.rs` | 行列演算（20.12 固定小数点）、`matrix_command` |
| `core/gpu/src/engine_3d/geometry/lighting.rs` | `calculate_lighting` |
| `core/gpu/src/engine_3d/geometry/polygon.rs` | `submit_vertex`、`submit_polygon` |
| `core/gpu/src/engine_3d/geometry/clip.rs` | `clip_segment`、`clip_against_plane`、`clip_polygon`、`box_test` |
| `core/gpu/src/engine_3d/mod.rs` | `render_3d_frame`、`set_render_scale`、`hires_3d` |

ジオメトリエンジンは melonDS `GPU3D.cpp`（GPL-3.0-or-later）の移植です。

## 9.3 データ構造（`Gpu3D` の主な部分）

| 分類 | フィールド | 内容 |
|---|---|---|
| FIFO | `gxfifo` | 256 段の `GxCommand { command, param }` |
| | `gxpipe` | 4 段のパイプ（FIFO の前段） |
| | `cmd_params`, パック解読状態 | 実行中コマンドの引数、`GXFIFO` へのパックド書き込みの解読状態 |
| | `cycles` | ジオメトリエンジンがまだ忙しいサイクル数 |
| 行列 | `proj_matrix`, `pos_matrix`, `vec_matrix`, `tex_matrix`, `clip_matrix`（+ `clip_matrix_dirty`） | 4×4、20.12 固定小数点、行優先 `m[row * 4 + col]` |
| | `proj_stack`（1 段）, `tex_stack`（1 段）, `pos_stack[32]`, `vec_stack[32]` と各ポインタ | 行列スタック |
| 頂点状態 | `cur_vertex`, `vertex_color`, `normal`, `raw_texcoords`, `texcoords`, `polygon_attr`, `cur_polygon_attr`, `tex_param`, `tex_palette`, `polygon_mode` | |
| ライト | `light_direction[4]`, `light_color[4]`, `spec_recip[4]`, `mat_diffuse/ambient/specular/emission`, `shininess_table[128]`, `use_shininess_table` | |
| テスト | `pos_test_result[4]`, `vec_test_result[3]` | POS_TEST / VEC_TEST の結果 |
| 組み立て | `temp_vertices[4]`, `vertex_num_in_poly`, `num_consecutive_polygons`, `last_strip_polygon` | ストリップの共有頂点 |
| RAM | `vertex_ram[2]`, `polygon_ram[2]`, `cur_bank` | 2 バンク（ダブルバッファ）。上限 `MAX_VERTICES` = 6144、`MAX_POLYGONS` = 2048 |
| | `num_vertices`, `num_polygons`, `num_opaque_polygons` | 現在のバンクの数 |
| 入れ替え | `swap_buffers`, `flush_attributes` | `SWAP_BUFFERS` 受信済み（VBlank までコマンドを止める）、その引数 |
| 描画 | `render_polygons`, `render_bank`, `render`（`RenderState`）, `render_dirty` | 確定した描画リストと、VBlank でラッチした描画レジスタ |
| | `raster`, `hires` | ラスタライザ（10 章）、高解像度コピー（11 章） |

`Vertex`：クリップ座標 `position[4]`、色（5 ビット成分を bit 12-16 に）と 9 ビットに伸長した色、画面座標（x 0〜511、y 0〜255）、1/16 画素単位の `hires_position`、テクスチャ座標など。`Polygon`：頂点 RAM の索引、属性、半透明か、Y の範囲と並べ替えキー（`sort_key`）、W/Z バッファ、上下の頂点など。

## 9.4 インタフェースと処理詳細

### 9.4.1 コマンドの投入（CPU / DMA → FIFO）

```text
write_gxfifo(word)            0x04000400..0x0400043F（パックド形式）／ GX FIFO DMA
  1. パックド命令の途中でない：word を最大 4 つのコマンドバイトとしてラッチ
     先頭が引数を必要とするなら引数を待つ
  2. 途中なら word を次の引数として数える
  3. (コマンド, word) を write_command() へ。引数がそろったら次のコマンドバイトへ
     （引数なしのコマンドは続けて出す）
write_fifo_direct(address, word)   0x04000440..0x040005CB（アドレスでコマンドを選ぶ）
  → write_command()
write_command(cmd)
  GXFIFO が空で GXPIPE が 4 未満 → GXPIPE へ直接
  それ以外：
    GXFIFO が満杯（256）→ 空きができるまで exec_command()（CPU が止まるのに相当）
    GXFIFO へ入れて check_fifo_irq()
```

### 9.4.2 実行（CPU 時間に合わせる）

```text
run_3d(ran)  ← run() で ARM9 の 1 命令ごと（2 章）
  → gpu3d_run(cycles)
      1. SWAP_BUFFERS 受信済み → 時間だけ進めて何もしない（VBlank 後に再開）
      2. 何もない（借りの時間なし・GXPIPE 空）→ return
      3. エンジンの cycles -= cycles
      4. エンジンが空いていて GXPIPE にコマンドがある間 exec_command()
exec_command()
  1. read_command()        GXPIPE の先頭を取り出す。GXPIPE が 3 未満なら GXFIFO から最大 2 個補充し、
                           check_fifo_dma() と check_fifo_irq()
  2. 引数を保存。CMD_PARAM_AMOUNTS[cmd] 個そろうまで return
  3. cycles += CMD_CYCLE_AMOUNTS[cmd]
  4. engine_3d.execute_command(cmd)
  5. cycles += geometry_pipeline_cycles(cmd, 実行前のポリゴン数)
```

`geometry_pipeline_cycles`（コマンド表にない時間）：

| コマンド | 追加サイクル |
|---|---|
| `0x23..=0x28`（VTX_*） | 3 + 7、ポリゴンが 1 つ増えたら + 26（`polygon_mode` bit 0 が 1 なら + 35） |
| `0x21`（NORMAL） | 有効なライト数（最低 1）+ 7 |

### 9.4.3 コマンド一覧（`execute_command`）

| コマンド | 内容 |
|---|---|
| `0x10..0x1C` | MTX_MODE、PUSH、POP、STORE、RESTORE、IDENTITY、LOAD_4x4 / 4x3、MULT_4x4 / 4x3 / 3x3、SCALE、TRANS |
| `0x20..0x2B` | COLOR、NORMAL（ライティング）、TEXCOORD、VTX_16 / 10 / XY / XZ / YZ / DIFF → `submit_vertex()`、POLYGON_ATTR、TEXIMAGE_PARAM、PLTT_BASE |
| `0x30..0x34` | DIF_AMB、SPE_EMI、LIGHT_VECTOR、LIGHT_COLOR、SHININESS |
| `0x40..0x41` | BEGIN_VTXS / END_VTXS |
| `0x50` | SWAP_BUFFERS（VBlank まで停止） |
| `0x60` | VIEWPORT（Y を画面座標へ反転） |
| `0x70..0x72` | BOX_TEST、POS_TEST、VEC_TEST |

### 9.4.4 頂点とポリゴン

```text
submit_vertex()
  1. クリップ行列（射影 × 位置）で変換
  2. 現在の色・テクスチャ座標と共に頂点スロットへ
  3. プリミティブ（三角形・四角形・ストリップ）に必要な数がそろったら submit_polygon()
submit_polygon()
  1. POLYGON_ATTR に従い表面 / 裏面カリング
  2. clip_polygon()：Z → Y → X の面でクリップ
  3. ビューポート変換で画面 x/y、深度（Z または W バッファ）
  4. ポリゴン RAM（+ 頂点 RAM）へ追加。不透明ポリゴンを数え、Y の並べ替えキーを計算
```

### 9.4.5 VBlank での入れ替え（`end_of_frame`）

```text
from: gpu_handle_event のライン 192
1. swap_buffers が立っていれば
     描画リスト = 不透明ポリゴン → 半透明ポリゴンの順
     安定ソートで Y 並べ替え：全ポリゴン。ただし SWAP_BUFFERS bit 0（半透明は手動ソート）なら
                              不透明だけ
     render_bank = cur_bank、cur_bank を反転、数をリセット、swap_buffers = false、render_dirty = true
2. DISP3DCNT、アルファテスト値、クリア色・深度、フォグ、エッジ色、トゥーン表などを render にラッチ
   （前回と違えば render_dirty）
to:   ライン 215 の render_3d_frame()
```

`render_3d_frame()`：DISPCNT bit 3（3D 表示）が 0 なら描かずに dirty のままにし、そうでなければ `raster.render(...)`（描画レジスタ、描画バンクのポリゴン/頂点、並べ替え済みリスト、VRAM のテクスチャ、dirty）を呼びます。

## 9.5 GX FIFO の割り込みと DMA

| 関数 | 条件 |
|---|---|
| `check_fifo_irq()` | GXSTAT bit 30-31 のモード 1：FIFO が 128 未満（半分未満）の間、モード 2：空の間、IF9 bit 21 を立てる。条件が成り立たなければ **下ろす** |
| `check_fifo_dma()` / `request_fifo_dma()` | FIFO が半分未満なら `gxfifo_dma_request()`（[7 章](07_dma.md)） |
| `set_gxstat(word, mask)` | GXSTAT の書き込み。bit 15 で行列スタックエラーを確認、bit 30-31 で IRQ モード。モードが変わるので `check_fifo_irq()` |

`check_fifo_irq` は、FIFO の段数や IRQ モードが変わるたびに呼びます（`read_command`、`write_command`、`set_gxstat`）。

## 9.6 設計判断と理由

- **GX FIFO 割り込みはレベルトリガー。** IF bit 21 は条件に従い、FIFO が補充されたら下ろします（melonDS `GPU3D::CheckFIFOIRQ`）。Nitro SDK は「FIFO 半分未満」割り込みでディスプレイリストを DMA で流し、最後のチャンクの DMA 完了割り込みでこれを止めます。DMA で FIFO が埋まった後も bit 21 を立てたままにしていたため、優先度で DMA の割り込みに常に勝ち（割り込みストーム）、3D のオープニングを持つソフトの 3D が止まりました。
- **頂点・法線・ポリゴン設定のパイプライン時間を足す。** コマンド表の時間だけでは、FIFO が実機の数倍の速さで空になりました。SDK は最後のチャンクの DMA 割り込みが FIFO が半分以上あるうちに来ることを前提にしています。速すぎると GX FIFO 割り込みが立ちっぱなしになり、DMA の割り込みが処理されませんでした（3D のオープニングを持つソフト）。
- **SWAP_BUFFERS の後は VBlank までコマンドを止める。** 実機と同じで、次のフレームのコマンドが現在のバンクを壊さないようにします。
- **描画はライン 215。** VBlank 中のテクスチャ転送（ライン 192 以降）が終わってから描くためです。描いたフレームは次のフレームで BG0 に表示されます。

## 9.7 制約・未実装

- ポリゴン RAM（`MAX_POLYGONS`）/ 頂点 RAM（`MAX_VERTICES`）があふれた場合は、そのポリゴンを捨て、DISP3DCNT の bit 13（overflow）を立てます（`submit_polygon`）。
- コマンドの時間は表と上記の近似です。ジオメトリエンジンの内部パイプラインを完全には再現していません。

## 9.8 関連テスト

- 3D のオープニングとタイトル画面を持つソフトを headless 例で描き、止まらないことと画像の一致を確認します（[19 章](19_testing.md)）。
- ログ：ターゲット `dma` の完了ログに、GX FIFO と GXPIPE の段数、IF9 が出ます。

## 9.9 ROM 実行テスト（機能 ID `geometry3d`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature geometry3d` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `polygons_min` | GX コマンドが実行され、ポリゴンが描画リストに入る |
| `no_terminate` | GXSTAT の busy 待ちや FIFO 割り込みで止まらない（9.6 の割り込みストームの再発検知） |

```toml
[[case]]
feature = "geometry3d"
name = "geometry3d-example"
rom = "my_game"            # [roms] の別名
frames = 2200
press = ["start:2000:60"]
[case.expect]
polygons_min = 100
no_terminate = true
```

3D のオープニングがあるゲームは、FIFO 割り込みと DMA の割り込みの順序（9.6）の回帰検知に向きます。
