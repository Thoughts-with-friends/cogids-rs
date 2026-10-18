# 3. CPU（ARM9 / ARM7・CP15・インタプリタ）

## 3.1 責務

2 つの ARM CPU の状態（レジスタ、モード、PSR、例外）と、ARM / THUMB 命令の解釈実行を受け持ちます。ARM9 については CP15（TCM、例外ベクタ位置、WFI）も扱います。

- CPU の状態とALU・シフタ・例外処理は `ds_cpu`（`core/cpu`）にあり、メモリに触れません。
- 命令のデコードと実行（メモリアクセスを含む）は `ds_emu`（`core/system/src/cpu`）にあります。命令ハンドラは `fn(&mut Emulator, CpuType, u32)` の形で、`Emulator` の `read_*` / `write_*` を通してバスにアクセスします。

この分け方は「emu と cpu の相互参照を避ける」ためです（`ArmCpu` は `Emulator` を知らない）。

## 3.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/cpu/src/arm_cpu.rs` | `ArmCpu`、`PsrFlags`、`PsrMode`、`CpuType`、ALU（`add`/`sub`/`adc`/`sbc`/`mov`…）、シフタ（`lsl`/`lsr`/`asr`/`rrx`/`rotr32`）、`msr`/`mrs`、`handle_irq`/`handle_swi`/`handle_undefined`、待ちサイクル加算 |
| `core/cpu/src/coprocessor_15.rs` | `Cp15`、`Cp15Effect`、TCM の読み書き |
| `core/cpu/src/instruction_table.rs` | 命令テーブルの補助 |
| `core/cpu/src/disassemble.rs` | 逆アセンブラ（`disasm_arm` など。トレース用） |
| `core/system/src/cpu/arm_table.rs` | `ARM_TABLE: [InterpreterFunc; 4096]`（CorgiDS `armtable.cpp`） |
| `core/system/src/cpu/interpreter/mod.rs` | `arm_interpret` |
| `core/system/src/cpu/interpreter/arm_instruction.rs` | ARM 命令ハンドラ（データ処理、乗算、ロード/ストア、ブロック転送、分岐、コプロセッサ、SWI） |
| `core/system/src/cpu/interpreter/thumb_instruction.rs` | `thumb_interpret`、`thumb_decode`、THUMB 命令ハンドラ |

## 3.3 データ構造

### 3.3.1 `ArmCpu`

| フィールド | 内容 |
|---|---|
| `cpu_id` | 0 = ARM9、1 = ARM7 |
| `halted` | HALT 中（割り込みで解除。[2 章](02_frame_scheduler.md) 2.5.3） |
| `regs[16]` | 現在のモードから見える r0〜r15 |
| `sp_svc/irq/fiq/abt/und`, `lr_*` | モードごとのバンクレジスタ |
| `fiq_regs` | FIQ の r8〜r12 |
| `cpsr`, `spsr[mode]` | `PsrFlags`（N/Z/C/V/Q、I/F、T、mode） |
| `exception_base` | 例外ベクタのベース（ARM9 は CP15 の制御ビット 13 で `0xFFFF0000`） |
| `timestamp`, `last_timestamp` | 実行済み時刻（ARM9 は 2 倍クロック単位） |
| `current_instr` | フェッチ済みで実行中の命令 |
| `pc_history`（長さ `PC_HISTORY_LEN` = 32） | 分岐先のリングバッファ（デバッグ用、`recent_pcs()`） |
| `code_waitstates[16][4]`, `data_waitstates[16][4]` | アドレス上位 4 ビット（領域）× アクセス種別（0 = N32、1 = S32、2 = N16、3 = S16）の待ちサイクル |

ARM7 のコード待ちサイクルの例（GBATEK に基づく値）：メイン RAM（領域 `0x2`）= 1、WRAM/I/O（`0x3`/`0x4`）= 32 ビット 8・16 ビット 4、パレット/VRAM（`0x5`/`0x6`）= 32 ビット 10・16 ビット 5。

### 3.3.2 `Cp15`

レジスタ番号は melonDS `CP15.cpp` と同じく `id = (CRn << 8) | (CRm << 4) | opcode2` です。

| id | レジスタ | 読み出し | 書き込み |
|---|---|---|---|
| `0x000`, `0x003..=0x007` | Main ID | `0x41059461` | |
| `0x001` | キャッシュ種別 | `0x0F0D2112` | |
| `0x002` | TCM サイズ | `(6 << 6) \| (5 << 18)` | |
| `0x100` | 制御 | `control` | 書き込み可能ビット `0x000FF085`、常に 1 のビット `0x00000078`。変更時 `Cp15Effect::ControlChanged`（ベクタ位置の更新） |
| `0x200`/`0x201`/`0x300` | PU キャッシュ設定 | 保存値 | 保存のみ |
| `0x500`〜`0x503` | PU アクセス権 | 2 ビット形式と 4 ビット形式を相互変換（`compress_rw`/`expand_rw`） | 保存のみ |
| `0x600`〜`0x670` | PU 領域 | 保存値 | 保存のみ |
| `0x704` / `0x782` | WFI | | `Cp15Effect::Halt`（ARM9 を HALT） |
| `0x910` | DTCM 領域 | `dtcm_setting` | `update_tcm()` |
| `0x911` | ITCM 領域 | `itcm_setting` | `update_tcm()` |

TCM：ITCM は物理 32 KiB（アドレス 0 から、設定サイズ内でミラー）、DTCM は物理 16 KiB（`dtcm_base` から、設定サイズ内でミラー）。無効時は `itcm_size = 0`、`dtcm_base = 0xFFFFFFFF`、`dtcm_mask = 0` です。

## 3.4 インタフェース

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `arm_interpret(emu, cpu)` | `Emulator::execute` | ARM 命令を条件判定してテーブルで実行 |
| `thumb_interpret(emu, cpu)` | `Emulator::execute` | THUMB 命令をデコードして実行 |
| `ArmCpu::handle_irq()` | `execute`（IE&IF≠0、IME、CPSR.I=0） | SPSR_irq = CPSR、LR_irq = 戻り先 + 4、IRQ モード、CPSR.I = 1、ARM 状態、ベクタ + `0x18` へ |
| `ArmCpu::handle_swi()` | SWI ハンドラ（HLE 無効時） | SPSR_svc = CPSR、LR_svc = SWI の次、SVC モード、CPSR.I = 1、ベクタ + `0x08` へ |
| `ArmCpu::jp(addr, change_thumb)` | 分岐命令 | 分岐（必要なら THUMB 切り替え） |
| `ArmCpu::update_reg_mode(mode)` | `msr`、例外、`spsr_to_cpsr` | レジスタバンクの切り替え |
| `ArmCpu::add_n32_code` ほか | フェッチ・ロード/ストア | 待ちサイクルを `timestamp` に加算 |
| `Cp15::read(id)` / `write(id, v)` | `coprocessor_reg_transfer`（MRC/MCR） | 3.3.2 |
| `Cp15::tcm_read::<N>(addr)` / `tcm_write(addr, bytes)` | `Emulator::read_*` / `write_*`（ARM9 のみ） | TCM に当たれば読み書きし `Some`/`true` |

## 3.5 処理詳細

### 3.5.1 ARM 命令

```text
arm_interpret(emu, cpu)
  1. instruction = current_instr、condition = bit 31-28
  2. op = (instruction >> 4 & 0xF) | (instruction >> 16 & 0xFF0)   12 ビットの索引
  3. ARM9 かつ cond = 15 かつ bit 27-25 = 101 → blx（即値 BLX、条件なし）
     それ以外：check_condition(cond) が真なら ARM_TABLE[op](emu, cpu, instruction)
```

`ARM_TABLE` は、bit 27-20 と bit 7-4 の組み合わせ（4096 通り）をハンドラに対応づけます。

### 3.5.2 THUMB 命令

`thumb_decode(instruction)` が `ThumbInstr` 列挙子を返し、`match` でハンドラを呼びます。BL は 2 命令（`thumb_long_branch_prep` → `thumb_long_branch` / `thumb_long_blx`）に分かれます。

### 3.5.3 ALU の書き戻し（`alu_writeback`）

- 結果の書き込み先が PC で S ビットありの場合は「例外からの復帰」です。CPSR を SPSR から戻し（レジスタバンクと THUMB 状態を含む）、フラグも SPSR の値にします。
- S ビットなしの PC 書き込みは単純な分岐です。ALU による PC 書き込みでは ARM/THUMB の切り替え（interworking）はしません。

### 3.5.4 シフタの特殊ケース

| 符号化 | 意味 |
|---|---|
| `LSL #0` | シフトなし、C は変化しない |
| `LSR #0` / `ASR #0` | 32 ビットシフト（`lsr_32` / `asr_32`） |
| `ROR #0` | RRX（キャリー経由で 1 ビット右回転） |
| レジスタ指定で 32 | LSL：結果 0・C = bit 0、LSR：結果 0・C = bit 31 |
| レジスタ指定で 32 超 | 結果 0・C = 0（ASR は bit 31 で埋める） |
| `rotr32(n, c)` | `c == 0` は変化なし。それ以外は `c % 32` 回転、C = 結果の bit 31 |

### 3.5.5 コプロセッサ転送（MRC / MCR）

ARM9 で `coprocessor_id == 15` かつ opcode1 = 0 のときだけ CP15 を操作します。`id = (CRn << 8) | (CRm << 4) | opcode2`。MCR の戻り値 `Cp15Effect` に応じて、`Halt` なら `halt()`、`ControlChanged` なら `set_high_vectors(cp15.high_vectors())` を呼びます。それ以外のコプロセッサはログを出して無視します。

## 3.6 設計判断と理由

- **`SWI` は `config.hle_bios` で分岐。** 以前はこの設定がどこにも効かず、常に BIOS イメージへ例外ジャンプしていました。HLE のルーチン（`Emulator::hle_bios`）は使われていないコードでした（[4 章](04_bios_firmware.md)）。
- **ARM9 の即値 BLX（cond = 15）はテーブルの前に判定。** 条件コード 15（NV）は通常「実行しない」ですが、ARMv5 ではこの符号化が BLX です。ARM7（ARMv4）では判定しません。
- **WFI は CP15 の書き込みで HALT。** SDK は ARM9 の待機に CP15 の c7 を使うため、これがないと CPU が空回りします。

## 3.7 制約・未実装

- 保護ユニット（PU）とキャッシュは値を保存するだけで、アクセス保護もキャッシュの時間も再現しません。
- 待ちサイクルは領域ごとの表による近似です。ARM9 のキャッシュヒット/ミスは考慮しません。
- 未定義命令は `undefined` ハンドラ（未定義命令例外）に入ります。

## 3.8 関連テスト

- `core/cpu/src/coprocessor_15.rs` のユニットテスト：`dtcm_window_and_readback`（DTCM の位置とミラー、読み戻し）、`halt_and_control`（WFI と制御レジスタ）
- トレース：`Config::trace_cpus`（bit 0 = ARM9、bit 1 = ARM7）で、実行命令を stderr に `<9|7><A|T> addr opcode cpsr r0..r14` の形式で出力します。`Config::trace_trigger = (mask, pc)` で、指定 PC に達したときからトレースを始められます。
