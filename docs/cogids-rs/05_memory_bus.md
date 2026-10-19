# 5. メモリバスとアドレスマップ

## 5.1 責務

CPU（と DMA、HLE BIOS、チート）からの読み書きを、アドレスに応じて RAM・BIOS・VRAM・各デバイスの I/O レジスタへ振り分けます。ARM9 と ARM7 はアドレスマップが異なるため、入口は共通、振り分けは CPU ごとの関数に分かれています。

I/O レジスタへの書き込みは、その場でデバイスの副作用（DMA 起動、割り込み、GX FIFO への投入、SPI の 1 バイト転送など）を起こします。

## 5.2 構成ファイル

| ファイル | 内容 |
|---|---|
| `core/system/src/emulator/read/mod.rs` | `read_word` / `read_halfword` / `read_byte`（入口）、`arm7_wram_offset` |
| `core/system/src/emulator/read/read_arm9.rs` | `arm9_read_word` / `arm9_read_halfword` / `arm9_read_byte` |
| `core/system/src/emulator/read/read_arm7.rs` | `arm7_read_word` / `arm7_read_halfword` / `arm7_read_byte` |
| `core/system/src/emulator/write/mod.rs` | `write_word` / `write_halfword` / `write_byte`（入口）、`check_watch`、`watch_key` |
| `core/system/src/emulator/write/write_arm9.rs` | `arm9_write_word` / `arm9_write_halfword` / `arm9_write_byte` |
| `core/system/src/emulator/write/write_arm7.rs` | `arm7_write_word` / `arm7_write_halfword` / `arm7_write_byte` |
| `core/system/src/emulator/gpu/io_2d.rs` | 2D エンジンのレジスタ（影コピー方式） |
| `core/mem_addr/src/lib.rs` | アドレス定数（`MAIN_RAM_START`、`SHARED_WRAM_START`、`ARM7_WRAM_START`、`IO_REGS_START`、`VRAM_*`、`OAM_START`、`MAIN_RAM_MASK` など） |

## 5.3 入口の処理

```text
read_word(address, cpu)  ／ write_word(address, value, cpu)
  write のみ：check_watch()      Config::watch_addr のデバッグ監視
  ARM9：CP15 の ITCM/DTCM に当たる？ → TCM を読み書き（Cp15::tcm_read / tcm_write）
        それ以外 → arm9_read_* / arm9_write_*
  ARM7：→ arm7_read_* / arm7_write_*
```

呼び出し元：命令フェッチ（`execute`）、ロード/ストア命令、ブロック転送、`SWP`、HLE BIOS（`Caller`）。DMA とチートは CPU ごとの関数（`arm9_*` / `arm7_*`）を直接呼びます（TCM を通らない）。

## 5.4 ARM9 のアドレスマップ

| 範囲 | 振り分け先 |
|---|---|
| 3D レジスタ（`Gpu3D::is_3d_reg`） | `gpu.engine_3d.read_reg` / `write_reg`（最初に判定） |
| 2D レジスタ（`io_2d::is_2d_reg`） | `read_io_2d_*` / `write_io_2d_*`（次に判定） |
| `0xFFFF0000..` | ARM9 BIOS |
| `0x02000000..0x02FFFFFF` | メイン RAM（`MAIN_RAM_MASK` で 4 MiB ミラー） |
| `0x03000000..0x03FFFFFF` | 共有 WRAM（5.6） |
| `0x04000000..` | DMA、タイマ、キー、IPC、カートリッジ、IE/IF/IME、VRAMCNT、WRAMCNT、除算器、平方根器、POWCNT1、GXSTAT、GXFIFO（`0x04000400..0x0400043F` → `write_gxfifo`）、コマンドポート（`0x04000440..0x040005CB` → `write_fifo_direct`） |
| `0x04100000` | IPC FIFO 受信（読むと割り込みが起きることがある） |
| `0x04100010` | カートリッジのデータポート → `cart_read_data()` |
| `0x05000000..` | パレット（エンジン A / B） |
| `0x06000000..` | VRAM（バンク割り当てに従う。[8 章](08_video_2d.md)） |
| `0x07000000..` | OAM |
| その他 | 読みは 0、書きは無視（`log` 機能でログ） |

## 5.5 ARM7 のアドレスマップ

| 範囲 | 振り分け先 |
|---|---|
| `0x00000000..0x00003FFF` | ARM7 BIOS（BIOS 外からの読みは保護） |
| `0x02000000..` | メイン RAM |
| `0x03000000..0x037FFFFF` | 共有 WRAM（WRAMCNT）または ARM7 WRAM のミラー |
| `0x03800000..0x03FFFFFF` | ARM7 専用 WRAM（64 KiB） |
| `0x04000004` / `0x04000006` | DISPSTAT / VCOUNT（ARM7 側） |
| `0x040000B0..` | DMA 4〜7 |
| `0x04000100..` | タイマ 0〜3 |
| `0x04000130` / `0x04000136` | KEYINPUT / EXTKEYIN |
| `0x04000138` | RTC |
| `0x04000180..` | IPC |
| `0x040001A0..` | カートリッジ（AUXSPI、ROMCTRL、コマンド、KEY2 シード） |
| `0x040001C0` / `0x040001C2` | SPICNT / SPIDATA（[14 章](14_spi_rtc_input.md)） |
| `0x04000208..` | IME / IE / IF |
| `0x04000241` | WRAMSTAT（バイト読み。`wram_cnt & 3`） |
| `0x04000300` / `0x04000301` | POSTFLG / HALTCNT |
| `0x04000304` | POWCNT2（→ `wifi.set_power_cnt`） |
| `0x04000308` | BIOSPROT |
| `0x04000400..0x0400051F` | SPU（`spu.read32` / `write32` ほか。[12 章](12_sound.md)） |
| `0x04100000` / `0x04100010` | IPC FIFO 受信 / カートリッジデータ |
| `0x04800000..0x0480FFFF` | Wi-Fi（`wifi.read` / `write` → `raise_wifi_irq`。[15 章](15_wifi_multiplayer.md)） |
| `0x06000000..` | ARM7 に割り当てた VRAM バンク C/D（256 KiB ミラー） |
| その他 | 読みは 0 |

## 5.6 共有 WRAM の割り当て（WRAMCNT `0x04000247`）

| WRAMCNT | ARM9 から見える範囲 | ARM7 から見える範囲 |
|---|---|---|
| 0 | 32 KiB 全体 | なし（`0x03000000..` は ARM7 WRAM のミラー） |
| 1 | 後半 16 KiB | 前半 16 KiB |
| 2 | 前半 16 KiB | 後半 16 KiB |
| 3 | なし（読みは 0） | 32 KiB 全体 |

ARM7 側の計算は `arm7_wram_offset(address) -> (is_arm7_wram, offset)` にまとめています（melonDS `NDS::MapSharedWRAM`）。ダイレクトブート後は 3 です。

## 5.7 2D レジスタの影コピー（`io_2d.rs`）

対象：`0x04000000..0x0400006F`（エンジン A）、`0x04001000..0x0400106F`（エンジン B）。DISPSTAT/VCOUNT（`0x04000004..7`）は共有レジスタなので除外します。

```text
write_io_2d_u16(address, value)      （u8 / u32 版も同じ流れ）
  1. split(address) → (エンジン, オフセット)
  2. sync_dispcapcnt()    キャプチャ終了でハードが下ろす DISPCAPCNT bit 31 を影へ反映
  3. 影 io_2d[engine] に value をマージ
  4. apply_io_2d()        その halfword を含むレジスタ全体を GPU に渡す
                          （set_dispcnt / set_bgcnt / スクロール / アフィン / ウィンドウ / 合成…）
read_io_2d_u16(address)
  DISPCAPCNT と DISP3DCNT は GPU の現在値、それ以外は影から。書き込み専用レジスタは 0
```

**理由：** ゲームはバイト・ハーフワード・ワードのどれでも書きます。DISPCNT、BGxX/BGxY、DISPCAPCNT は 32 ビット幅です。部分書き込みをそのまま GPU に渡すと残りの半分を消してしまいます。例えば DISPCNT への `STRH` で bit 16-17（表示モード）が 0 になると「表示オフ」＝真っ白な画面になりました。

## 5.8 デバッグ監視（ウォッチポイント）

`Config::watch_addr` を設定すると、CPU からそのワードへの書き込みを stderr に `WATCH armN pc~... writeN [addr] = value` の形式で出力します。`watch_key` がメイン RAM のミラーを畳むので、ミラー経由の書き込みも検出できます。

## 5.9 異常系

- 範囲外・未対応のアドレスは、読みで 0、書きで無視です。`log` 機能を有効にするとアドレスがログに出ます。
- 8 ビットの VRAM 書き込み（ARM9）は実機で無視されるため、警告を出して無視します。

## 5.10 制約・未実装

- GBA スロット（`0x08000000..`）は実装していません。
- VRAMSTAT（ARM7 `0x04000240`）は `Vram::vramstat()` があるものの、ARM7 の読み出しに接続されていません（0 が返ります）。
- 待ちサイクルは CPU 側の表（[3 章](03_cpu.md)）による近似で、バスの競合は再現しません。DMA 中は両 CPU を止めます（[7 章](07_dma.md)）。

## 5.11 ROM 実行テスト（機能 ID `memory`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature memory` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `ram` | 特定のアドレスに期待どおりの値がある（WRAMCNT の割り当て、ミラー） |
| `not_blank` | DISPCNT の部分書き込み（5.7）で真っ白にならない |
| （自動）baseline | アドレスマップの変更で画面が変わらない |

```toml
[[case]]
feature = "memory"
name = "memory-example"
rom = "my_game"            # [roms] の別名
frames = 600
[case.expect]
not_blank = ["upper", "lower"]
```

チートの書き込み（16 章）と `ram` を組み合わせると、ARM7 のバスからメイン RAM への経路も確かめられます。
