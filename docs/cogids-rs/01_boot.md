# 1. 起動・ROM 読み込み・ダイレクトブート

## 1.1 責務

エミュレータを生成してから、最初の `Emulator::run()` で ROM のエントリポイントから命令を実行できる状態になるまでを受け持ちます。

- `Emulator` の生成（全メモリをゼロで確保）
- 設定（`Config`）の反映、BIOS・ファームウェアの読み込み
- ROM ファイルの読み込み、セーブチップ種別の決定と `.sav` の読み込み
- 全デバイスの電源投入リセット（`power_on`）
- BIOS/ファームウェアの起動コードを実行せずに、起動直後の状態を作る（ダイレクトブート）

フレームの実行そのものは [2 章](02_frame_scheduler.md)、セーブの読み書きの詳細は [13 章](13_cartridge_save.md) を参照してください。

## 1.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/system/src/emulator/mod.rs` | `Emulator` 構造体、`new` / `power_on` / `direct_boot` / `firmware_direct_boot` / `flush_save` / `set_mp_link`、`Drop` |
| `core/system/src/emulator/load.rs` | `load_rom` / `load_firmware` / `load_bios7` / `load_bios9` / `load_firmware_image` / `load_save_database`、`read_firmware_image` |
| `core/system/src/emulator/cartridge.rs` | `cartridge_load_rom`、`cartridge_setup_backup`、`corgids_database_type`、`cartridge_init_keycode` |
| `core/system/src/emulator/emu_config.rs` | `Config`、`BiosMem` |
| `core/cpu/src/arm_cpu.rs` | `ArmCpu::direct_boot`（起動直後のレジスタ） |
| `core/cpu/src/coprocessor_15.rs` | `Cp15::direct_boot`（起動直後の CP15） |
| `gui/common/src/instance.rs` | `Settings::apply`（GUI の設定を `Config` へ反映） |

## 1.3 データ構造

### 1.3.1 `Emulator`（`core/system/src/emulator/mod.rs`）

DS 本体のすべての状態を 1 つの構造体が所有します。デバイス同士は互いを参照せず、`Emulator` のメソッドが仲介します（例：DMA の完了割り込みは `Emulator` が `int9_reg` に立てる）。

| 分類 | フィールド | 型 | 内容 |
|---|---|---|---|
| 設定 | `config` | `Config` | 1.3.2 参照 |
| CPU | `arm9`, `arm7` | `ArmCpu` | 2 つの CPU（[3 章](03_cpu.md)） |
| | `arm9_cp15` | `Cp15` | ARM9 のシステム制御コプロセッサ（TCM 含む） |
| デバイス | `cart` | `NDSCart` | ゲームカード（[13 章](13_cartridge_save.md)） |
| | `dma` | `NDSDma` | DMA 8 チャンネル（[7 章](07_dma.md)） |
| | `gpu` | `Gpu` | 2D/3D 映像（8〜11 章） |
| | `rtc`, `spi` | `RealTimeClock`, `SPIBus` | RTC・SPI 機器（[14 章](14_spi_rtc_input.md)） |
| | `spu` | `SPU` | 音源（[12 章](12_sound.md)） |
| | `nds_timing` | `NDSTiming` | タイマ 8 本（[6 章](06_interrupt_timer_ipc.md)） |
| | `wifi` | `WiFi` | 無線（[15 章](15_wifi_multiplayer.md)） |
| | `ipc` | `Ipc` | CPU 間通信 |
| メモリ | `main_ram` | `Vec<u8>` | 4 MiB |
| | `shared_wram` | `Vec<u8>` | 32 KiB（`wram_cnt` で ARM9/ARM7 に割り当て） |
| | `arm7_wram` | `Vec<u8>` | 64 KiB |
| | `arm9_bios`, `arm7_bios` | `BiosMem<N>` | BIOS ダンプ（読み込んだ場合のみ） |
| スケジューラ | `system_timestamp`, `next_event_time`, `gpu_event`, `dma_event` | | [2 章](02_frame_scheduler.md) |
| I/O | `io_2d` | `[[u8; 0x70]; 2]` | 2D レジスタの生の写し（[5 章](05_memory_bus.md)） |
| | `int7_reg`, `int9_reg` | `InterruptRegs` | IE/IF/IME |
| | `key_input`, `ext_key_in` | | KEYINPUT / EXTKEYIN |
| | `pow_cnt2` | `PowCnt2Reg` | POWCNT2（スピーカ、Wi-Fi 電源） |
| | `divcnt` ほか, `sqrtcnt` ほか | | 除算器・平方根器 |
| | `postflg7/9`, `bios_prot`, `wram_cnt`, `ex_mem_cnt`, `r_cnt`, `sio_cnt`, `aux_spi_cnt` | | 起動フラグ、メモリ制御 |
| チート | `cheats` | `Vec<Vec<u32>>` | 毎フレーム実行する AR コード（[16 章](16_cheats.md)） |
| 音 | `spu_cycles` | `u32` | まだサンプルにしていない ARM7 サイクル |

`Drop` で `cart.flush_save()` を呼び、未保存のセーブを書き出します。

### 1.3.2 `Config`（`emu_config.rs`）

| フィールド | 既定値 | 意味 |
|---|---|---|
| `arm7_bios_path`, `arm9_bios_path`, `firmware_path` | 空 | ダンプのパス。空なら内蔵の代替イメージ |
| `savelist_path` | 空 | CorgiDS 形式のセーブデータベース |
| `save_dir` | `None` | `.sav` の置き場所。`None` は ROM の隣（`<rom>.sav`） |
| `direct_boot_enabled` | `true` | ダイレクトブート |
| `hle_bios` | `true` | HLE BIOS（[4 章](04_bios_firmware.md)） |
| `frameskip` | 0 | 描画を省くフレーム数 |
| `mac_address` | `None` | 電源投入時にファームへ書き込む MAC |
| `bg_enable`, `enable_framelimiter`, `pause_when_unfocused` | | CorgiDS 由来（`bg_enable` は `power_on` で全 `true`） |
| `test`, `trace_cpus`, `trace_trigger`, `watch_addr` | 無効 | デバッグ補助（[19 章](19_testing.md)） |

GUI は `ds_common::instance::Settings::apply` で `settings.json` の内容（`hle_bios`、MAC、`frameskip`、ファームのパス、BIOS ダンプ）を `load_rom` より前に反映します。

## 1.4 インタフェース

| 関数 | 呼び出し元 | 処理 | 次の処理 |
|---|---|---|---|
| `Emulator::new()` | 各 GUI、headless 例 | 電源オフ・ゼロ初期化の本体を作る | `config` 設定 → `load_rom` |
| `load_firmware()` | 任意 | `config` の BIOS/ファームを読む。ファームが開けなければ内蔵イメージ | `spi.init` |
| `load_bios7(&[u8])` / `load_bios9(&[u8])` | `Settings::apply` | BIOS ダンプを `BiosMem` に入れる | |
| `load_save_database(path)` | 任意 | CorgiDS のセーブ DB を読む | `cartridge_setup_backup` が参照 |
| `load_rom(path)` | 各 GUI | 1.5.1 の手順 | GUI が毎フレーム `run()` |
| `power_on()` | `load_rom` | 1.5.3 | `direct_boot()` |
| `flush_save()` | GUI（停止時・セーブ取り込み時） | 未保存セーブを即時書き出す | |
| `set_mp_link(link)` | GUI（`new` の後） | Wi-Fi の通信相手を接続。`load_rom` / `power_on` をまたいで保持 | [15 章](15_wifi_multiplayer.md) |

## 1.5 処理詳細

### 1.5.1 `load_rom(path)`

```text
from: gui/egui start / ds_common Frontend::start / headless main
  |
  v
load_rom(path)
  1. cartridge_load_rom(path)
       a. 前のゲームのセーブを flush_save()（失敗はログのみ）
       b. ROM を丸ごと cart.rom に読み込み、rom_size と名前を記録
       c. cartridge_setup_backup()     セーブチップの種別決定と .sav 読み込み
       d. ダイレクトブートなら終了
       e. それ以外：KEY1 で secure area を再暗号化（実カードと同じ状態）し、
          起動プロトコル用に KEY1 を初期化したままにする
  2. ファーム未読み込みなら read_firmware_image(config.firmware_path)
       開けない → 内蔵ファーム（ds_bios::firmware::FIRMWARE_DS）
       → spi.init(image)    ユーザー設定の選択とパッチ（14 章）
  3. power_on()
  |
  v
to:   GUI が毎フレーム run()
```

### 1.5.2 セーブチップの決定（`cartridge_setup_backup`）

次の順で最初に見つかった種別を使います。

1. melonDS のゲームデータベース（`ds_cartridge::game_db`、ゲームコードで検索）
2. `load_save_database` で読んだ CorgiDS のデータベース（19 バイト/件：ヘッダ 16 バイト + オフセット 18 にサイズコード）
3. 既存の `.sav` のサイズ（`BackupMemory::type_from_size`）
4. melonDS の既定値（ホームブリューはなし、それ以外は 64 KiB EEPROM）

既存の `.sav` がチップより短ければ残りを `0xFF` で埋め、長ければ切り詰めます。置き場所は `config.save_dir` があればその下、なければ ROM の隣です（GUI はインスタンスフォルダの `saves/` を指定します。[17 章](17_frontend_common.md)）。

### 1.5.3 `power_on()`

```text
power_on()
  1. CPU×2・CP15・DMA・カートリッジ・GPU・SPU・タイマ・RTC をリセット
  2. add_gpu_event(0, 256 * 6)        ライン 0 の最初の HBlank 開始
  3. total_timestamp = 20
     POWCNT2：スピーカ ON・Wi-Fi OFF
     config.mac_address があればファームの MAC を書き換え
     wifi.reset(firmware)              コンソール種別・RF チップ・チャネル表
     wifi.set_power_cnt(false)
  4. スケジューラ（system_timestamp=0, next_event_time=0xFFFFFFFF, イベント初期化）
     POSTFLG / AUXSPICNT / SIOCNT / BIOSPROT / SQRTCNT / DIVCNT = 0
     キーをすべて離した状態、IPC リセット、io_2d をゼロ
     IE/IF/IME（両 CPU）= 0
  5. config.direct_boot_enabled なら direct_boot()
```

メイン RAM と WRAM は `power_on` では消去しません（`new` でゼロ確保済み）。

### 1.5.4 `direct_boot()`（melonDS `NDS::SetupDirectBoot` 相当）

| 手順 | 内容 |
|---|---|
| 1 | カートリッジヘッダ `0x000..0x16F` をメイン RAM `0x027FFE00` へコピー |
| 2 | コピーしたヘッダ `0x027FFE20..` から 8 ワードを読む：ARM9 の ROM オフセット・エントリ・RAM アドレス・サイズ、ARM7 の同じ 4 項目 |
| 3 | `arm9.direct_boot(entry9)`、`arm7.direct_boot(entry7)`、`arm9_cp15.direct_boot()`、`arm9.set_high_vectors(cp15.high_vectors())` |
| 4 | チップ ID を `0x027FF800/804`、`0x027FFC00/C04` に書く。ヘッダ CRC（ROM `0x15E`）と secure area CRC（ROM `0x6C`）を `0x027FF808/80A`、`0x027FFC08/C0A` に書く。さらに `0x5835` を `0x027FF850` と `0x027FFC10` に、`0xFFFF` を `0x027FFC30` に、`0x0001` を `0x027FFC40` に書く |
| 5 | `POSTFLG7/9 = 1`、`RCNT = 0x8000`、`EXMEMCNT = 0xE880`、`AUXSPICNT = 0x8000`、`BIOSPROT = 0x1204`、`WRAMCNT = 3` |
| 6 | ARM9 バイナリを ROM から `arm9_write_word` で、ARM7 バイナリを `arm7_write_word` でコピー（4 バイトずつ） |
| 7 | `firmware_direct_boot()`：ファームサイズ、ヘッダの一部、ユーザー設定（名前・言語・タッチ較正）をメイン RAM（`0x027FFC80` 付近）へ |

起動直後の CPU レジスタ（`ArmCpu::direct_boot`）：

| | ARM9 | ARM7 |
|---|---|---|
| PC / R12 / R14 | エントリポイント | エントリポイント |
| R13（System） | `0x03002F7C` | `0x0380FD80` |
| SP_irq | `0x03003F80` | `0x0380FF80` |
| SP_svc | `0x03003FC0` | `0x0380FFC0` |
| モード | System | System |

起動直後の CP15（`Cp15::direct_boot`）：制御レジスタ `0x00052078`、DTCM 設定 `0x0300000A`（`0x03000000` に 16 KiB）、ITCM 設定 `0x00000020`、保護ユニットの領域設定（`0x600..0x670`）など、melonDS と同じ値を書きます。

## 1.6 異常系

| 状況 | 扱い |
|---|---|
| ROM が開けない / サイズが取れない / 読めない | `CartridgeError::OpenRom` / `MetadataRom` / `ReadRom`（パス付き）を返す。`power_on` は実行しない |
| `.sav` が読めない | `CartridgeError` の `ReadSave` |
| ファームのパスが開けない | 内蔵ファームを使う（エラーにしない） |
| ファームのダンプがフラッシュより短い | `EmuError` を返す（`read_firmware_image`） |
| 前のゲームのセーブ書き出し失敗 | ログのみ。読み込みは続ける |

## 1.7 設計判断と理由

- **ヘッダのコピーは `0x170` バイトまで。** `0x027FFF70..0x027FFFFF` には SDK の共有ロックワード（例：カートリッジロック `0x027FFFE8`）があります。DSi 拡張ヘッダを持つハイブリッド ROM でここまで埋めると、ロックが「取得済み」のまま残り、起動が止まりました。
- **HLE BIOS とダイレクトブートが既定。** 内蔵の代替 BIOS イメージでは `IntrWait`/`VBlankIntrWait` などが無限ループのスタブです。市販ソフトは毎フレーム `SWI 5` で垂直同期を取るため、LLE では止まります。
- **`set_mp_link` は `power_on` をまたいで保持。** GUI は通信リンクを一度だけ作り、ゲームを入れ替えても付け直さずに済みます。
- **ROM を入れ替える前にセーブを書き出す。** 新しい ROM の `.sav` を読む前に、前のゲームの未保存データをファイルへ書いておく必要があります。GUI 側でも「停止してから開始」の順にしています（[17 章](17_frontend_common.md)）。

## 1.8 制約・未実装

- BIOS / ファームの起動コードを実行する LLE 起動は、代替 BIOS がスタブのため実用になりません。
- `Emulator::frame_complete()` / `Emulator::display_swapped()` は CorgiDS API の名残で、未実装です。代わりに `gpu.is_frame_complete()` を使います。
- `power_on` ではメイン RAM を消去しません。

## 1.9 関連テスト

- `ds_common` の `#[ignore]` テスト（実 ROM が必要）：環境変数 `LUNARIS_TEST_ROM` に ROM のパスを入れて `cargo test -p ds_common -- --ignored` を実行し、起動してフレームが出ることを確認します。
- headless 例（`core/system/examples/headless.rs`、git 管理外）で起動と描画を確認します（[19 章](19_testing.md)）。
