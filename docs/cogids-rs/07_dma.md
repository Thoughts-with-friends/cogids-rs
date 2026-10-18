# 7. DMA

## 7.1 責務

8 チャンネル（ARM9 用 4、ARM7 用 4）の DMA 転送を受け持ちます。起動条件（即時・HBlank・カートリッジ・GX FIFO）の判定、1 単位ずつのコピー、アドレスの増減、完了割り込み、リピートを扱います。転送中は両 CPU を止めます。

## 7.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/system/src/dma.rs` | `DmaCnt`、`Dma`、`NDSDma`（レジスタとチャンネル状態） |
| `core/system/src/emulator/dma.rs` | `dma_handle_event`、`dma_write_cnt`、`dma_write_len_cnt`、`hblank_request`、`gamecart_request`、`gxfifo_request` |
| `core/system/src/emulator/mod.rs` | `dma_active`、`hblank_dma_request`、`gamecart_dma_request`、`gxfifo_dma_request`、`check_gxfifo_dma`、`add_dma_event` |

## 7.3 データ構造

### 7.3.1 チャンネル番号

`dmas[0..4]` が ARM9 の DMA0〜3、`dmas[4..8]` が ARM7 の DMA0〜3 です（`is_arm9 = index < 4`）。タイマとは番号の付け方が逆なので注意してください（[6 章](06_interrupt_timer_ipc.md)）。

### 7.3.2 `DmaCnt`（DMAxCNT の上位 16 ビット）

| フィールド | ビット | 内容 |
|---|---|---|
| `dest_control` | 5-6 | 0 = 増加、1 = 減少、2 = 固定、3 = 増加してリピート時に再設定 |
| `source_control` | 7-8 | 0 = 増加、1 = 減少、2 = 固定 |
| `repeat` | 9 | 完了後に繰り返す |
| `word_transfer` | 10 | 32 ビット単位（0 なら 16 ビット） |
| `timing` | 11-13（ARM9）/ 11-12（ARM7） | 起動条件（7.3.4）。ARM7 のビット位置は 7.7 参照 |
| `irq_after_transfer` | 14 | 完了で割り込み |
| `enabled` | 15 | 有効 |

`get(is_arm9)` / `set(value, is_arm9)` は、CPU によって timing の幅を変えます。

### 7.3.3 `Dma`

| フィールド | 内容 |
|---|---|
| `source`, `destination`, `length` | ゲームが書いた値 |
| `internal_source`, `internal_dest`, `internal_len` | 転送中の作業用コピー |
| `cnt` | `DmaCnt` |
| `index` | 0〜7 |
| `is_arm9` | |

`NDSDma { dmas[8], active_dmas: u8 }`。`active_dmas` は転送中のチャンネルのビットマスクで、0 でなければ `dma_active()` が真になり、`execute` は両 CPU を止めます。

### 7.3.4 起動条件（timing）

| 値 | ARM9 | ARM7 | 起動する処理 |
|---|---|---|---|
| 0 | 即時 | 即時 | `dma_write_cnt` |
| 1 | VBlank | VBlank | **未実装**（7.7） |
| 2 | HBlank | DS カード | ARM9：`hblank_request`。ARM7：未接続（7.7） |
| 3 | 表示同期 | Wi-Fi / GBA | 未実装 |
| 4 | メインメモリ表示 | — | 未実装（ARM9） |
| 5 | DS カード | — | `gamecart_request` |
| 6 | GBA カード | — | 未実装 |
| 7 | GX FIFO | — | `gxfifo_request` |

### 7.3.5 転送長（`NDSDma::write_len`）

| CPU | 書いた値 | `length` |
|---|---|---|
| ARM9 | 0 | `0x200000` |
| ARM9 | それ以外 | `len & 0x1FFFFF` |
| ARM7 ch 7 | 0 | `0x10000` |
| ARM7 ch 7 | それ以外 | `len & 0xFFFF` |
| ARM7 ch 4〜6 | 0 | `0x4000` |
| ARM7 ch 4〜6 | それ以外 | `0x3FFF`（7.7 参照） |

## 7.4 インタフェース

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `dma_write_cnt(index, cnt)` | DMAxCNT_H の書き込み | 7.5.1 |
| `dma_write_len_cnt(index, word)` | DMAxCNT の 32 ビット書き込み | 下位を `write_len`、上位を `dma_write_cnt` |
| `hblank_request()` | `gpu_handle_event`（表示ラインの HBlank 開始） | 最初の有効な ARM9 timing 2 を起動（他の DMA が転送中でなければ） |
| `gamecart_request()` | `cartridge_run`（ROM データのワードが用意できた） | EXMEMCNT bit 11 でスロットを持つ CPU 側：ARM7 は timing 4、ARM9 は timing 5 |
| `gxfifo_request()` | `check_fifo_dma` / `request_fifo_dma` / `check_gxfifo_dma`（GX FIFO が半分未満） | 最初の有効な ARM9 timing 7 を起動 |
| `dma_handle_event()` | `run()`（`dma_event` の期限到来） | 7.5.2 |

どの起動処理も `active_dmas |= 1 << i` と `add_dma_event(i, 0)` を行います。

## 7.5 処理詳細

### 7.5.1 `dma_write_cnt(index, cnt)`

```text
1. cnt.set(cnt, is_arm9)
2. 無効 → 有効（bit 15 の 0→1）になったときだけ：
     internal_source = source、internal_dest = destination、internal_len = 0
     timing 0 → active_dmas に追加、add_dma_event(index, 0)
     timing 7 → check_gxfifo_dma()（FIFO が半分未満ならすぐ起動）
     その他   → 各起動要因を待つ
```

### 7.5.2 `dma_handle_event()`

```text
dma_event.processing = false、ch = dma_event.id
ループ：
  1. internal_len += 1
  2. internal_len > length（全単位完了）
       irq_after_transfer → ARM9：IF9 bit 8 + index
                             ARM7：IF7 bit 8 + (index & 3)
       internal_len = 0
       repeat なし → enabled = false、active_dmas から外す
       repeat あり → dest_control 3 なら internal_dest = destination
                     timing 0 → add_dma_event(ch, length)（自分で再予約）
                     それ以外 → active_dmas から外し、次の起動要因を待つ
       return
  3. 1 単位コピー：所有 CPU のバスで read → write
       ARM9：arm9_read_* → arm9_write_*   ARM7：arm7_read_* → arm7_write_*
       （I/O の副作用が起きる。例：GX FIFO への書き込み）
  4. dest / source を ±サイズ、または固定（dest 0 と 3 は増加）
  5. ARM9 timing 7 で internal_len >= 112：
       internal_len = 0、length -= 112、active_dmas から外して return
       （残りは次の FIFO 要求で続ける）
```

1 回のイベントで全単位を一度に転送します。転送にかかる時間は再現していません（CPU は `active_dmas` が 0 になるまで止まるだけで、DMA は同じスライス内で終わります）。

## 7.6 設計判断と理由

- **`internal_len += 1` をループの先頭で行う。** CorgiDS の C++ と同じ順序です。過去のリファクタでこの加算が消え、完了判定が永遠に真にならず、転送元・転送先のアドレスがメモリ全体を回り続けました。
- **`NDSDma` の `Default` は derive しない。** derive すると全チャンネルの `index` が 0 になり、完了割り込みがすべて DMA0 として（ARM7 側はタイマ割り込みとして）上がっていました。DMA1〜3 の割り込みを待つゲームが止まりました。
- **timing の幅を CPU で変える。** 以前は常に 2 ビットでマスクしていたため、ARM9 専用の timing 4〜7 を表現できず、カード DMA と GX FIFO DMA が一度も起動しませんでした。
- **GX FIFO DMA は 112 ワードで区切る。** 実機と同じく、FIFO が半分未満になるたびに 112 ワードずつ流し込みます。区切らないと FIFO があふれ、割り込みのタイミングもずれます（[9 章](09_video_3d_geometry.md)）。
- **DMA は所有 CPU のバス関数を直接使う。** TCM を通らないこと、I/O の副作用（GXFIFO、カードのデータポート）が通常の書き込みと同じに起きることが目的です。

## 7.7 制約・未実装・要確認

- **VBlank DMA（timing 1）を起動する処理がありません。** `gpu_handle_event` の VBlank 開始から呼ぶ要求関数が未実装です。
- ARM9 の表示同期（3）、メインメモリ表示（4）、GBA（6）と、ARM7 の Wi-Fi / GBA（3）も未実装です。
- **ARM7 のカード DMA は起動しません。** `gamecart_request` は ARM7 のチャンネルを timing 4 で探しますが、ARM7 の timing は 2 ビット幅（0〜3）なので 4 になりません。GBATEK では ARM7 の DS カードは timing 2 です。通常のゲームは ARM9 でカードを読むため、今のところ問題になっていません。
- **ARM7 の timing のビット位置。** `DmaCnt::set` は ARM7 でも bit 11 から 2 ビットを読みます（`(value >> 11) & 0x3`）。GBATEK では ARM7 の起動タイミングは bit 12-13 です。ARM7 で即時以外の DMA を使うゲームで問題が出たら、ここを確認してください。
- `NDSDma::write_len` で、ARM7 のチャンネル 4〜6 に 0 以外を書くと、書いた値によらず `0x3FFF` になります（`len & 0x3FFF` の誤りの可能性があります）。
- DMA の転送時間（バスの占有サイクル）は再現していません。
- `dma_event` は同時に 1 件だけです（[2 章](02_frame_scheduler.md)）。
- `Emulator::dma_event(index)` は何もしないプレースホルダです。

## 7.8 関連テスト

- GX FIFO DMA の流れは、ハートゴールドのオープニング（3D のディスプレイリストを DMA で送る）で確認します（[19 章](19_testing.md)）。
- ログ：`log` 機能を有効にすると、ターゲット `dma` に起動・完了（転送元・転送先・FIFO の段数・IF9）が出ます。
