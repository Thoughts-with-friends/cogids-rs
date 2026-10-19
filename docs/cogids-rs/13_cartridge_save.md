# 13. ゲームカード・暗号・セーブメモリ

## 13.1 責務

- ROM バス：8 バイトのコマンド（ROMCMD）、ROMCTRL、データポートによるカードの読み出しプロトコル
- 暗号：KEY1（Blowfish 変種、secure area とコマンド）、KEY2（データバスのストリーム暗号のシード）
- AUXSPI バス：セーブチップ（EEPROM / FRAM / FLASH、赤外線付きカートリッジ）
- セーブの永続化：`.sav` への遅延書き出しと原子的な置き換え
- セーブチップ種別の決定（ゲームデータベース）

ROM 読み込みの全体の流れは [1 章](01_boot.md) を参照してください。

## 13.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/cartridge/src/lib.rs` | `NDSCart`、`power_on`、`load_database`、`save_check`、`flush_save` |
| `core/cartridge/src/protocol.rs` | `CartCommand`、`RomCtrl`、`set_romctrl`、`receive_command`、`get_output`、`chip_size`、`chip_id`、`read_rom_b7_word`、`direct_read*` |
| `core/cartridge/src/key1.rs` | `key1_encrypt`、`key1_decrypt`、`apply_keycode` |
| `core/cartridge/src/key2.rs` | KEY2 シード（ROMSEED0/1）の設定 |
| `core/cartridge/src/auxspi.rs` | AUXSPICNT / AUXSPIDATA |
| `core/cartridge/src/backup/mod.rs` | `BackupMemory`、`SaveChip`、`SaveMemType`、`SAVE_SIZES`、`flush`、`end_frame`、`transfer` |
| `core/cartridge/src/backup/eeprom.rs`, `flash.rs` | チップ別のプロトコル |
| `core/cartridge/src/game_db.rs` | melonDS のゲームデータベース（ゲームコード → セーブ種別） |
| `core/cartridge/src/error.rs` | `CartridgeError` |
| `core/system/src/emulator/cartridge.rs` | `cartridge_load_rom`、`cartridge_setup_backup`、`cartridge_init_keycode`、`cartridge_run`、`cart_read_data`、`cart_end_transfer` |

## 13.3 データ構造

### 13.3.1 `NDSCart`

| フィールド | 内容 |
|---|---|
| `rom`, `rom_size`, `rom_name` | ROM 全体 |
| `key1_buffer` | KEY1 の鍵表（0x1048 バイト） |
| `cmd_encrypt_mode` | 0 = 平文、1 = KEY1 コマンド、2 = KEY2 |
| `command_buffer[8]` | ROMCMD（`0x040001A8..0x040001AF`） |
| `command_id` | 実行中の `CartCommand` |
| `rom_data_index`, `secure_area_index` | 読み出し位置 |
| `cycles_left`, `bytes_left` | 次のワードまでのサイクル、残りバイト |
| `data_output` | 用意できたワード |
| `romctrl` | `RegRomCtrl`（13.3.2） |
| `auxspicnt`, `spi_data`, `spi_selected` | AUXSPI（制御、最後の受信バイト、チップ選択中） |
| `encrypt_seed0`, `encrypt_seed1` | KEY2 シード |
| `keycode[3]` | KEY1 のキーコード |
| `backup` | `BackupMemory` |
| `save_database`, `database_size` | CorgiDS 形式の DB |

### 13.3.2 ROMCTRL（`0x040001A4`、`RegRomCtrl`）

| ビット | フィールド | 内容 |
|---|---|---|
| 0-12 | `key1_gap` | KEY1 のギャップ（クロック） |
| 13 | `key2_data_enabled` | |
| 15 | `key2_apply_seed` | |
| 16-21 | `key2_gap` | |
| 22 | `key2_cmd_enabled` | |
| 23 | `word_ready` | データポートにワードがある |
| 24-26 | `block_size` | 0 = 0、1〜6 = `0x100 << n`、7 = 4 バイト |
| 27 | `slow_transfer` | |
| 28 | `key1_gap_clocks` | |
| 31 | `block_busy` | 転送中 |

### 13.3.3 セーブ種別（`SaveMemType`）

| コード | チップ | サイズ | `SaveChip` |
|---|---|---|---|
| 0 | なし | — | `None`（バスは浮く） |
| 1 | EEPROM tiny | 512 B | `EepromTiny`（コマンドの bit 3 が上位 256 バイトを選ぶ） |
| 2〜4 | EEPROM / FRAM | 8 K / 64 K / 128 K | `Eeprom`（64 KiB 以下はアドレス 2 バイト、128 KiB は 3 バイト） |
| 5〜7 | FLASH | 256 K / 512 K / 1 M | `Flash`（アドレス 3 バイト、ページ/セクタ消去） |
| 8〜10 | NAND | 8 M / 16 M / 64 M | 未対応（なしとして扱う） |

### 13.3.4 `BackupMemory`

| フィールド | 内容 |
|---|---|
| `chip` | `SaveChip` |
| `data` | チップの中身（`len()` がチップサイズ、初期値 `0xFF`） |
| `path` | `.sav` のパス |
| `pos`, `cmd`, `addr` | チップ選択中のバイト番号（0 = コマンド）、コマンド、アドレス |
| `status` | ステータスレジスタ（bit 1 = `STATUS_WEL` 書き込み許可ラッチ） |
| `ir`, `ir_pos`, `ir_cmd` | 赤外線トランシーバの有無と状態（赤外線通信・歩数計付きのカード） |
| `dirty` | 前回の書き出しから変わったか |
| `idle_frames` | `dirty` のまま書き込みがなかったフレーム数 |

## 13.4 ROM バスのプロトコル

### 13.4.1 コマンドの開始（`set_romctrl`）

```text
from: ARM9/ARM7 の ROMCTRL 書き込み（コマンドは先に receive_command で 8 バイト書かれている）
set_romctrl(value)
  各フィールドを設定
  block_busy が 0→1 かつ AUXSPICNT bit 15（スロット有効）：
    word_ready = false、bytes_left = ブロックサイズ
    KEY1 モード（cmd_encrypt_mode != 0）：cycles_left += key1_gap
    cmd_encrypt_mode == 1：さらに key1_gap を足し、8 バイトを KEY1 で復号
    command_buffer[0] で解読：
      0x9F ダミー、0x00 ヘッダ、0x90 / 0xB8 チップ ID、0x3C KEY1 有効化、
      0xB7 ROM データ（アドレスは 1〜4 バイト目、ビッグエンディアン）
      KEY1 モードの上位ニブル：0x1? チップ ID、0x2? secure area ブロック、0x4? ダミー、
                              0xA? KEY2 有効化（cmd_encrypt_mode = 2）
```

### 13.4.2 データの生成（`cartridge_run`、毎スライス）

```text
cartridge_run(cycles)
  1. 転送中でワードが待っていない：cycles_left -= cycles。まだ正なら return
  2. cycles_left = 8（1 ワード 8 サイクル）
  3. コマンド別に data_output を作り word_ready = true
       Dummy → 0xFFFFFFFF
       GetHeader → ROM 先頭から（0xFFF を超えたら折り返し）
       GetChipId → chip_id()
       GetSecureAreaBlock → secure area
       ReadRom（0xB7）→ read_rom_b7_word(rom_data_index)
  4. bytes_left -= 4、gamecart_dma_request()（DMA が読めるように。7 章）
  5. 転送完了 → cart_end_transfer()（busy を下ろし、AUXSPICNT bit 14 なら割り込み）
```

### 13.4.3 データポートの読み出し（`cart_read_data`、`0x04100010`）

`cart.get_output()` で `data_output` を返し、`word_ready` を下ろして `cycles_left = 8` にします（次のワードは 8 サイクル後）。ブロックの最後のワードが**読まれた**ところで `cart_end_transfer()` を呼びます。

### 13.4.4 チップ ID と B7 読み出し

- `chip_size()`：ROM イメージのサイズを 2 の累乗に切り上げ（melonDS `PadToPowerOf2`）。
- `chip_id()`：Macronix（`0xC2`）＋サイズコード。128 MiB までは「MiB 単位のサイズ − 1」、それより大きければ `0x100 - size / 256 MiB`（melonDS `ParseROM`）。ファームが `0x027FF800` / `0x027FFC00` に置く値です。
- `read_rom_b7_word(address)`：アドレスをチップサイズで折り返し、先頭 32 KiB（secure area）は読めず `0x8000..0x81FF` を返します（melonDS `CartRetail::ReadROM_B7`）。トリムされたイメージの末尾より先は `0xFF`（消去済みの領域と同じ）を返します。

## 13.5 AUXSPI とセーブチップ

```text
from: ARM7 / ARM9 の AUXSPIDATA（0x040001A2）書き込み
set_auxspidata(byte)
  1. スロット無効または SPI モードでない → 無視
  2. チップ未選択 → backup.select()（このバイトがコマンド）
  3. reply = backup.transfer(byte) → AUXSPIDATA の読み出し値
  4. AUXSPICNT bit 6（チップ選択保持）が 0 → backup.release()
       コマンドの終わり。書き込みが終わっていれば dirty
```

- AUXSPICNT の bit 13（SPI モード）を、チップ選択中に 0 にするとチップ選択を外し、1 に戻すと再び選択します（melonDS `NDSCartSlot::Interface::WriteSPICnt`）。
- 転送は即時なので、busy（bit 7）は立ちません。
- `transfer` は、赤外線付きならまず IR コマンドバイトを受け、`0x00` のときだけ残りをセーブチップへ渡します（melonDS `CartRetailIR`）。
- `transfer_chip`：全チップ共通のステータス読み出しと書き込み許可（WREN / WRDI）を処理し、残りをチップ別のプロトコル（`eeprom_tiny`、`eeprom`、`flash`）へ渡します。書き込み系のコマンドの後は WEL を下ろします。

## 13.6 セーブの永続化

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `BackupMemory::end_frame()` | `NDSCart::save_check()` ← `run()` の最後 | `dirty` で書き込みが `FLUSH_DELAY_FRAMES`（30 フレーム）止まっていたら `flush()` |
| `BackupMemory::flush()` | 上記、`NDSCart::flush_save()` | 変更があれば、一時ファイルに書いてからリネームで置き換え |
| `NDSCart::flush_save()` | ROM の入れ替え、`Emulator::flush_save`、`Emulator` の `Drop` | 即時に書き出す |
| `BackupMemory::load_contents(bytes)` | `cartridge_setup_backup` | 短ければ残りを `0xFF`、長ければ切り詰め |

## 13.7 異常系

| 状況 | 扱い |
|---|---|
| ROM が開けない・読めない | `CartridgeError::OpenRom` / `MetadataRom` / `ReadRom` |
| `.sav` が読めない | `CartridgeError::ReadSave` |
| `.sav` の書き出し失敗 | `CartridgeError::SaveWrite`（`save_error` でパス付き）。`run()` ではログのみ |
| セーブ DB が開けない・形式不正 | `OpenSaveDatabase` / `MetadataSaveDatabase` / `InvalidSaveDatabaseFormat` / `ReadSaveDatabase` |
| B7 でブロックが 0x1000 を超える | ログ（`log` 機能） |

## 13.8 設計判断と理由

- **ROMCTRL の busy は、最後のワードが読まれるまで下ろさない。** 用意できただけで下ろすと、「word ready を見て、次に busy を見る」ポーリングループが最後のワードを取りこぼします。後期のゲームのコピー対策はこれを検出します（melonDS `NDSCartSlot::ReadROMData`）。
- **B7 はチップサイズで折り返し、先頭 32 KiB を読ませない。** コピー対策のコードは、チップサイズを超えたアドレス（例：256 MiB カードで `0x10001000`）を読み、期待どおりのデータでなければ終了します。
- **`.sav` は 30 フレーム書き込みが止まってから書く。** 複数ページのセーブを 1 回の書き出しにまとめます。
- **一時ファイルに書いてからリネームする。** 書き出し中に止まっても、途中で切れたセーブが残りません。
- **ダイレクトブートでないときは secure area を KEY1 で再暗号化する。** 実カードと同じ状態にし、BIOS の起動プロトコルが通るようにします。

## 13.9 制約・未実装

- NAND セーブ（大容量カード）は未対応です（セーブなしとして扱います）。
- 赤外線は「コマンド 0x00 で素通し」だけで、歩数計などとの通信はしません。
- 転送速度は 1 ワード 8 サイクルの近似です。`slow_transfer` や（KEY1 モード以外の）ギャップは反映していません。

## 13.10 関連テスト

- 実 ROM：コピー対策のある後期の市販ソフト（B7・ROMCTRL）、赤外線付きカードのソフトでセーブと起動を確認します。
- ログ：ターゲット `card` に、コマンドバイトと長さが出ます。
- **実ファイルで試すときは、セーブを別フォルダに分けてください**（インスタンスフォルダの `saves/`、または `Config::save_dir`）。

## 13.11 ROM 実行テスト（機能 ID `cartridge`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature cartridge` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `save_from` | 実セーブをコピーして起動する（元のファイルは書き換えない） |
| `save_written` | セーブチップの判定と `.sav` の書き出しが動く |
| `no_terminate` | B7 読み出し・ROMCTRL の busy・MAC のコピー対策を通る（13.8） |

```toml
[[case]]
feature = "cartridge"
name = "cartridge-example"
rom = "my_game"            # [roms] の別名
frames = 600
save_from = "D:/path/to/game.sav"
[case.expect]
save_written = true
no_terminate = true
```

セーブを書かせるには、`press` でゲーム内のセーブ操作まで進めるケースを作ります。書いたセーブはケースのフォルダに残るので、中身を比べられます。
