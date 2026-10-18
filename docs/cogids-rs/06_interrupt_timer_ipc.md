# 6. 割り込み・タイマ・IPC・除算器/平方根器

## 6.1 責務

- **割り込み**：各デバイスの要求を CPU ごとの IF に立て、IE・IME・CPSR.I に従って CPU に IRQ 例外を起こさせます。HALT 中の CPU を起こします。
- **タイマ**：CPU ごとに 4 本、計 8 本のタイマ（プリスケーラ、カスケード、オーバーフロー割り込み）。
- **IPC**：ARM9 と ARM7 の間の IPCSYNC と、16 段の FIFO 2 本。
- **除算器・平方根器**：ARM9 の数学ハードウェア（結果は即時）。

## 6.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/interrupt/src/lib.rs` | `Interrupt`（IE/IF のビット番号）、`InterruptRegs { ime, irq_enable, irq_flags }` |
| `core/system/src/emulator/interrupt.rs` | `request_interrupt7/9`、`raise_ipc`、`raise_wifi_irq`、`request_interrupt_gba`、`arm7_has_cart_rights` |
| `core/system/src/emulator/mod.rs` | `irq_line_raised`、`requesting_interrupt`、`start_division`、`write_div_operand`、`start_sqrt`、`write_sqrt_param` |
| `core/peripherals/src/timers.rs` | `NDSTiming`、`TimerReg`、`Divisor` |
| `core/system/src/emulator/timers.rs` | `run_timers9`、`run_timers7`、`run_timer`、`overflow` |
| `core/peripherals/src/ipc.rs` | `Ipc`、`Side`、`IpcIrq`、`IrqRequest` |

## 6.3 割り込み

### 6.3.1 割り込み番号（`Interrupt`）

| bit | 名前 | 主な発生元 |
|---|---|---|
| 0 | `VBlank` | `gpu_handle_event`（ライン 192） |
| 1 | `HBlank` | `gpu_handle_event`（ドット 256） |
| 2 | `VCountMatch` | `gpu_handle_event`（DISPSTAT の比較値） |
| 3〜6 | `Timer0`〜`Timer3` | `overflow` |
| 7 | `Rtc` | |
| 8〜11 | `Dma0`〜`Dma3` | `dma_handle_event`（完了時） |
| 12 | `Keypad` | |
| 13 | `GBASlot` | |
| 16 | `IpcSync` | IPCSYNC 書き込み |
| 17 | `IpcFifoEmpty` | 送信 FIFO が空になった |
| 18 | `IpcFifoNempty` | 受信 FIFO が空でなくなった |
| 19 | `CartTransfer` | カートリッジ転送完了（`cart_end_transfer`） |
| 20 | `CartIreqMc` | |
| 21 | `GeometryFifo` | `check_fifo_irq`（ARM9、[9 章](09_video_3d_geometry.md)） |
| 22 | `UnfoldScreen` | |
| 23 | `Spi` | SPI 転送完了（ARM7） |
| 24 | `Wifi` | `raise_wifi_irq`（ARM7） |

### 6.3.2 判定

| 関数 | 条件 | 用途 |
|---|---|---|
| `irq_line_raised(cpu)` | `IE & IF != 0`（IME は見ない） | HALT の解除 |
| `requesting_interrupt(cpu)` | `IE & IF != 0` かつ IME | IRQ 例外を起こすか（CPSR.I は呼び出し側 `execute` が見る） |

IRQ 例外の処理は `ArmCpu::handle_irq`（[3 章](03_cpu.md)）です。ゲームの割り込みハンドラが IF に 1 を書いて要求を消します（IF は「1 を書いたビットを消す」レジスタ）。

### 6.3.3 IPC・Wi-Fi の割り込み

- `raise_ipc(request)`：IPC のレジスタアクセスが返した `Option<IrqRequest>`（対象 CPU と種類）を、`request_interrupt7/9` に変換します。
- `raise_wifi_irq()`：`wifi.take_irq()` が真（`W_IF & W_IE` の立ち上がり）なら ARM7 の bit 24 を立てます。`run()` の各スライスと、Wi-Fi レジスタへの書き込み後に呼ばれます。

## 6.4 タイマ

### 6.4.1 データ構造

| 型 | フィールド | 内容 |
|---|---|---|
| `TimerReg` | `counter` | 現在値（16 ビット） |
| | `reload_value` | オーバーフロー時の再設定値 |
| | `cycles_left` | 次のカウントアップまでのサイクル |
| | `clock_div` | `Divisor`：F1 / F64 / F256 / F1024 |
| | `count_up_timing` | カスケード（前のタイマのオーバーフローで +1） |
| | `irq_on_overflow`, `enabled` | |
| `NDSTiming` | `timers[8]`, `timer_clock_divs[4]` | |

**タイマの番号：** `timers[0..4]` が **ARM7** のタイマ 0〜3、`timers[4..8]` が **ARM9** のタイマ 0〜3 です（`run_timers7` が 0..4、`run_timers9` が 4..8 を進め、`overflow` も `index < 4` を ARM7 として扱います）。ARM9 の `0x04000100` への書き込みは `timers[4]` に入ります。`core/peripherals/src/timers.rs` の「0-3 for ARM9, 4-7 for ARM7」というコメントは古い記述で、実装と逆です。DMA は逆に `0..4` が ARM9 です（[7 章](07_dma.md)）。

### 6.4.2 処理

```text
run_timers9(ran) / run_timers7(cycles_ran)       （run() の各命令の後、2 章）
  有効なタイマだけ run_timer(cycles, index)
run_timer(cycles, index)
  1. カスケード（index が 0/4 以外で count_up_timing）なら何もしない
  2. cycles_left -= cycles
     cycles_left <= 0 の間：cycles_left += 分周値
       counter == 0xFFFF → overflow(index)、それ以外 → counter += 1
overflow(index)
  1. counter = reload_value
  2. irq_on_overflow → index < 4 なら request_interrupt7(Timer0 + index)
                       それ以外 request_interrupt9(Timer0 + index - 4)
  3. 次のタイマ（index が 3/7 以外）がカスケードなら +1（0xFFFF なら再帰的に overflow）
```

分周値の単位はバスサイクル（33.513982 MHz）で、ARM9 側のタイマも同じ単位で進めます（ARM9 の経過時間は半分にしてから渡す。[2 章](02_frame_scheduler.md)）。

## 6.5 IPC

### 6.5.1 データ構造（`Ipc`）

| フィールド | 内容 |
|---|---|
| `sync9`, `sync7` | IPCSYNC（`0x04000180`）。bit 0-3 は相手の bit 8-11、bit 8-11 は自分の出力、bit 14 は相手への IRQ 許可 |
| `cnt9`, `cnt7` | IPCFIFOCNT（`0x04000184`）の保存ビット（状態ビットは計算で作る） |
| `fifo9`, `fifo7` | ARM9→ARM7、ARM7→ARM9 のキュー（`VecDeque<u32>`、最大 `FIFO_DEPTH` = 16） |
| `last9`, `last7` | 最後に取り出したワード（空・無効時の読み出し値） |

### 6.5.2 インタフェース

| 関数 | アドレス | 処理 | 割り込み |
|---|---|---|---|
| `write_sync(side, value, mask)` | `0x04000180` | `mask`（`0x00FF` / `0xFF00` / `0xFFFF`）で自分の半分だけ更新 | bit 13 書き込みかつ相手の bit 14 → 相手に `IpcSync` |
| `write_cnt(side, value)` | `0x04000184` | IRQ 許可・FIFO 有効・エラー確認・送信 FIFO クリア | 条件が既に成立している IRQ を許可した瞬間に発生 |
| `send(side, word)` | `0x04000188` | 送信 FIFO 有効なら 16 段キューへ。満杯ならエラーフラグ | 空 → 非空で、相手が受信 IRQ 許可なら相手に `IpcFifoNempty` |
| `receive(side)` | `0x04100000` | FIFO 無効なら取り出さずに先頭を返す。空なら最後の値とエラーフラグ | 相手のキューが空になり、相手が送信空 IRQ 許可なら相手に `IpcFifoEmpty` |

SDK の PXI は FIFO のワードの bit 0-4 をタグに使います（例：6 = タッチパネル、10 = 無線）。

### 6.5.3 設計判断

CorgiDS は 2 本の FIFO キューを両 CPU の FIFO オブジェクトからポインタで共有していました。ここでは `Ipc` が方向ごとに 1 本ずつキューを所有し、一方の CPU が書いたワードが必ず他方の CPU の読み出しと FIFOCNT に見えるようにしています（melonDS `NDS.cpp` と同じ構造）。

## 6.6 除算器・平方根器（ARM9）

| レジスタ | アドレス | 内容 |
|---|---|---|
| DIVCNT | `0x04000280` | bit 0-1 モード（0 = 32/32、1 = 64/32、2 = 64/64、3 = 64/32）、bit 14 = 64 ビット分母がゼロ |
| DIV_NUMER | `0x04000290..0x04000297` | 被除数 |
| DIV_DENOM | `0x04000298..0x0400029F` | 除数 |
| DIV_RESULT / DIVREM_RESULT | `0x040002A0..` / `0x040002A8..` | 商 / 余り |
| SQRTCNT | `0x040002B0` | bit 0：0 = 32 ビット入力、1 = 64 ビット入力 |
| SQRT_RESULT | `0x040002B4` | `floor(sqrt(param))` |
| SQRT_PARAM | `0x040002B8..0x040002BF` | 入力 |

- オペランドの半分を書くたびに `write_div_operand` / `write_sqrt_param` が計算し直します（結果は即時で、busy ビットは立ちません）。
- ゼロ除算：商は被除数が負なら +1、それ以外は -1、余りは被除数。32 ビットモードでは商の下位ワードだけが -1 になり、上位ワードは被除数の符号に従います。
- オーバーフロー：32 ビットで `i32::MIN / -1` は商 `0x80000000`・余り 0、64 ビットで `i64::MIN / -1` は商 `i64::MIN`・余り 0。
- GBATEK "DS Maths" と melonDS `NDS::DivDone` に従います。

## 6.7 制約・未実装

- `InterruptRegs::is_requesting_int` は CorgiDS の名残で未実装です（`unimplemented!`）。判定は `Emulator` 側で行います。
- `Rtc`、`Keypad`、`GBASlot`、`CartIreqMc`、`UnfoldScreen` の割り込みを発生させる処理はありません。
- タイマの読み出し値は、最後に `run_timer` で進めた時点の値です（読み出し時に途中経過を補間しません）。

## 6.8 関連テスト

- IPC：`core/peripherals/src/ipc.rs` のユニットテスト `sync_output_reaches_remote_input`、`sync_irq_requires_remote_enable`、`fifo_crosses_cpus`。実 ROM の起動も確認になります（SDK の PXI が動かないと起動しません）。
- 除算器：`start_division` の境界値（ゼロ除算・オーバーフロー）は GBATEK の表と照合します。
