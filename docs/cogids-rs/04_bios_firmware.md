# 4. BIOS（HLE / 代替イメージ）とファームウェア

## 4.1 責務

- **HLE BIOS**：ゲームが `SWI` で呼ぶ BIOS 関数を、CPU 上の BIOS コードではなく Rust で実行します。
- **代替 BIOS イメージ**：HLE を無効にしたときに CPU が実行する BIOS バイナリ（DraStic free BIOS、BSD-2-Clause）を提供します。
- **内蔵ファームウェア**：ファームウェアのダンプがないときに使う、SPI フラッシュの中身（ヘッダ、Wi-Fi 較正、ユーザー設定）をコンパイル時に生成します。

ファームウェアを SPI 経由で読む処理と、読み込み時のユーザー設定のパッチは [14 章](14_spi_rtc_input.md) を参照してください。

## 4.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/bios/src/swi/mod.rs` | `SwiContext` トレイト、`arm9(ctx)` / `arm7(ctx)`、`cpu_set`、`get_crc16`、`opcode` |
| `core/bios/src/swi/tables.rs` | `SINE_TABLE` / `PITCH_TABLE` / `VOLUME_TABLE`（DeSmuME `bios.cpp` 由来） |
| `core/bios/src/image/arm9.rs`, `arm7.rs` | 代替 BIOS バイナリ |
| `core/bios/src/firmware.rs` | `DSType`、`FIRMWARE_DS` / `FIRMWARE_DSI` / `FIRMWARE_DS_IQUE`、`default_firmware`、`crc16` |
| `core/system/src/bios.rs` | `Caller`（`SwiContext` の実装）、`Emulator::hle_bios` |

## 4.3 データ構造

### 4.3.1 `SwiContext`（BIOS 関数が呼び出し元 CPU に求めるもの）

| メソッド | 内容 |
|---|---|
| `reg(i)` / `set_reg(i, v)` | 呼び出し元 CPU の r0〜r15 |
| `pc()` / `is_thumb()` | SWI 番号を取り出すため |
| `read8/16/32(addr)`, `write16/32(addr, v)` | 呼び出し元 CPU のバスでの読み書き（通常のアドレスマップを通る） |
| `add_cycles(n)` | CPU を `n` サイクル止める |
| `halt()` | 割り込みまで HALT |
| `wait_for_interrupt(mask)` | IE に `mask` を足し、IME を有効にして HALT |

`Caller { emu, cpu }`（`core/system/src/bios.rs`）がこれを実装します。読み書きは `Emulator::read_*` / `write_*` を使うので、I/O の副作用も通常どおり起きます。

### 4.3.2 内蔵ファームウェア

| 定数 | 内容 |
|---|---|
| `FIRMWARE_DS` | 256 KiB（DS / DS Lite） |
| `FIRMWARE_DSI` | 128 KiB |
| `FIRMWARE_DS_IQUE` | 512 KiB |
| `MAC_ADDRESS` | `00:09:BF:11:22:33`（melonDS の既定値。`00:09:BF` は任天堂の OUI） |
| `TOUCH_CALIBRATION` | ユーザー設定 `0x58..0x63`：ADC (0x200, 0x200) = 画素 (32, 32)、ADC (0xE00, 0xA00) = 画素 (224, 160) |
| `WIFI_INIT_PORTS` | Wi-Fi 較正 `0x44..0x63`：Wi-Fi I/O ポートの初期値 |
| `WIFI_BB_INIT` | `0x64..0xCC`：ベースバンドレジスタ `0x00..0x68` の初期値 |
| `WIFI_RF_INIT` | `0xCE..0xF6`：RF チップ（種別 3）の初期値 |
| `WIFI_CHANNEL_DATA` | `0xF8..0x133`：チャネルごとの BB/RF 値 |
| `WIFI_CONFIG_LEN` | `0x138`（ヘッダ `0x2C` からの Wi-Fi 設定ブロック長） |

`DSType::model_spec()` はヘッダ `0x1D`（コンソール種別）の値です。`default_firmware::<N>(model)` は `const fn` で、CRC16（多項式 `0xA001`）もコンパイル時に計算します。

## 4.4 インタフェース

```text
from: arm_instruction::swi / THUMB の SWI（config.hle_bios が true のとき）
      false のときは ArmCpu::handle_swi() で BIOS イメージのベクタ +0x08 へ
  |
  v
Emulator::hle_bios(cpu_id)
  Caller { emu, cpu } を作り、ds_bios::swi::arm9 / arm7 を呼ぶ
    opcode(ctx)：戻り先の直前の SWI 命令（ARM は pc-4、THUMB は pc-2 の
                 さらに 2 バイト前）を read16 して下位 8 ビット
  戻り値：消費サイクル
  |
  v
to:   インタプリタに戻り、SWI の次の命令から続ける
```

## 4.5 実装済みの BIOS 関数

| 番号 | 名前 | ARM9 | ARM7 | 処理 |
|---|---|---|---|---|
| `0x03` | WaitByLoop | ○ | ○ | `r0 × 2`（ARM9）/ `r0 × 4`（ARM7）サイクル待つ |
| `0x04` | IntrWait | ○ | ○ | `wait_for_interrupt(r1)` |
| `0x05` | VBlankIntrWait | ○ | ○ | `wait_for_interrupt(1)`（IE/IF の bit 0） |
| `0x06` | Halt | ○ | ○ | `halt()` |
| `0x0B` | CpuSet | ○ | ○ | r0 = 転送元、r1 = 転送先、r2 = 長さ（bit 0-20）・bit 24 フィル・bit 26 ワード単位 |
| `0x0E` | GetCRC16 | ○ | ○ | r0 = 初期値、r1 = アドレス、r2 = バイト数 → r0。反転 CRC-16（多項式 `0xA001`） |
| `0x1A` | GetSineTable | | ○ | `SINE_TABLE[r0]` → r0 |
| `0x1B` | GetPitchTable | | ○ | `PITCH_TABLE[r0]` → r0（`(2^(i/768) - 1) × 0x10000`） |
| `0x1C` | GetVolumeTable | | ○ | `VOLUME_TABLE[r0]` → r0 |

範囲外の索引は 0 を返します（実機では不定値）。未実装の番号はログ（`log` 機能有効時）を出して何もしません。

## 4.6 設計判断と理由

- **IntrWait の近似。** 実 BIOS は共有 RAM のフラグで「新しく発生した」割り込みを待ちます。HLE では IE と IME を有効にして HALT し、通常の割り込み経路で CPU を起こします。VBlank で同期するゲームにはこれで足ります。
- **ARM7 の CpuSet は BIOS 領域を書き換えない。** 転送元と転送先がともに `0x4000` 未満なら何もしません（実機の保護に合わせる）。
- **MAC アドレスをゼロにしない。** SDK は起動時に MAC を `0x027FFCF4` へコピーします（`OS_GetMacAddress`）。後期のゲームのコピー対策は、MAC がゼロならエミュレータと判断して `OS_Terminate` を呼びます。
- **GetCRC16 はファームのユーザー設定と同じ CRC。** ゲームはこれで設定ブロックを検証します。

## 4.7 制約・未実装

- 除算（`Div`）、平方根（`Sqrt`）、LZ77 / Huffman / RLE 展開、`BitUnPack` などは HLE に実装されていません。これらは ARM9 のハードウェア除算器/平方根器（[6 章](06_interrupt_timer_ipc.md)）や、SDK 自身のソフトウェア実装で代替されている前提です。必要なゲームが見つかった場合は `swi::arm9` / `arm7` に追加します。
- `Emulator::hle_bios` の関数コメントには「Div、CpuSet、LZ77/Huffman 展開…」と書かれていますが、実装済みなのは上の表の関数だけです。
- 代替 BIOS イメージは最小限のもので、`IntrWait` などがスタブです。LLE（`hle_bios = false`）での市販ソフトの動作は期待できません。

## 4.8 関連テスト

- ファームの CRC：`core/peripherals/src/spi/firmware.rs` の `verify_crc` を読み込み時に使います（[14 章](14_spi_rtc_input.md)）。
- 内蔵ファームの書き出し例：`std::fs::write("firmware.bin", ds_bios::firmware::FIRMWARE_DS)`（`firmware.rs` の doc 例）。

## 4.9 ROM 実行テスト（機能 ID `bios`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature bios` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `no_terminate` | `IntrWait` / `VBlankIntrWait` の近似で止まらない |
| `screen_changes` | VBlank 同期で毎フレーム進む |
| `audio` | サウンドドライバ（`GetPitchTable` などの表）が動いて音が出る |

```toml
[[case]]
feature = "bios"
name = "bios-example"
rom = "my_game"            # [roms] の別名
frames = 900
[case.expect]
no_terminate = true
screen_changes = true
audio = true
```

MAC がゼロでないこと（4.6）は、コピー対策のあるゲームの `no_terminate` で間接的に確かめられます。
