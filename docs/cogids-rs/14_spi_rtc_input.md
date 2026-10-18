# 14. SPI 機器・RTC・キー入力・タッチ

## 14.1 責務

- **SPI バス**（ARM7 専用）：電源管理 IC、ファームウェアのフラッシュ、タッチスクリーンコントローラ（TSC）の 3 つの機器を、SPICNT で選んで SPIDATA で 1 バイトずつやり取りする
- **ファームウェア**：ダンプまたは内蔵イメージ（[4 章](04_bios_firmware.md)）を読み込み、有効なユーザー設定を選び、パッチする
- **RTC**：ビットバング方式のシリアルで日時とアラームを返す
- **キー入力**：GUI が押したボタンを KEYINPUT / EXTKEYIN に反映する
- **タッチ**：GUI が指定したペン位置を EXTKEYIN と TSC に反映する

## 14.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/peripherals/src/spi/mod.rs` | `SPIBus`、`RegSpiCnt`、`init`、`write_spidata`、`read_spidata`、`touchscreen_press` |
| `core/peripherals/src/spi/power.rs` | `PowerMan`（SPI 機器 0） |
| `core/peripherals/src/spi/firmware.rs` | `Firmware`（SPI 機器 1）、`load`、`patch_user_settings`、`set_mac_address`、`transfer_data`、`create_crc`、`verify_crc` |
| `core/peripherals/src/spi/touchscreen.rs` | `TouchScreen`（SPI 機器 2）、`press_event`、`transfer_data` |
| `core/peripherals/src/rtc.rs` | `RealTimeClock`、`Alarm`、`set_fixed_time`、`write`、`read`、`interpret_input` |
| `core/system/src/emulator/button.rs` | `button_*_pressed` / `button_*_released` |
| `core/system/src/emulator/emu_config.rs` | `KeyInputReg`、`ExtKeyInReg`、`PowCnt2Reg` |
| `core/system/src/emulator/mod.rs` | `touchscreen_press` |

## 14.3 SPI バス

```text
 ARM7 CPU
   |  SPICNT  (0x040001C0)  bit 8-9: 機器選択、bit 11: チップ選択保持
   |  SPIDATA (0x040001C2)  1 回の転送で 1 バイト送り 1 バイト受け取る
   v
 +--------------------------- SPI バス（spi/mod.rs） ---------------------------+
 |        機器 0                   機器 1                       機器 2           |
 +-----------|-----------------------|---------------------------|--------------+
             v                       v                           v
       power.rs                firmware.rs                 touchscreen.rs
       電源管理 IC             ファームのフラッシュ        タッチスクリーン
       （電源断、バックライト、 （ユーザー設定、Wi-Fi 設定、 コントローラ（ペン位置、
        アンプ、マイクゲイン）   起動コード）                 マイク・温度の ADC）
```

3 つの機器は ARM7 のアドレス空間にマップされていません。CPU は SPICNT で機器を選び、SPIDATA でバイトを交換するしかありません。そのため、3 つの機器のモデルはバスと同じ `spi/` に置いています。

### 14.3.1 SPICNT

| ビット | フィールド | 内容 |
|---|---|---|
| 0-1 | `bandwidth` | 0 = 4 MHz、1 = 2 MHz、2 = 1 MHz、3 = 512 kHz |
| 7 | `busy` | 転送中（転送は即時なので立たない） |
| 8-9 | `device` | 0 = 電源管理、1 = ファーム、2 = タッチスクリーン、3 = 予約 |
| 10 | — | 16 ビット転送（未対応。常に 8 ビット） |
| 11 | `chipselect_hold` | 転送後もチップを選択したままにする |
| 14 | `irq_after_transfer` | 転送完了で割り込み |
| 15 | `enabled` | バス有効 |

### 14.3.2 `write_spidata(byte) -> bool`

```text
from: ds_emu arm7_write_halfword(0x040001C2)（ARM7 のみ）
write_spidata(byte)
  バス無効 → 無視
  SPICNT の機器に応じて
    0 電源管理    → powerman.transfer()
    1 ファーム    → firmware.transfer_data()
    2 タッチ      → touchscreen.transfer_data()
  チップ選択保持なし → 機器を解放（release / deselect）。現在のコマンドが終わる
  結果のバイト → output（0x040001C2 で読める）
  戻り値：SPICNT bit 14 なら true（呼び出し側が SPI 割り込みを立てる）
```

## 14.4 電源管理 IC（`PowerMan`）

最初のバイトがレジスタ番号（bit 7 = 読み出し）、2 バイト目がデータです。レジスタ：0 = 制御、1 = バッテリ（0 = 正常）、2 = アンプ、3 = マイクゲイン、4 = バックライト。`release()` で次のバイトが再びレジスタ番号になります。

## 14.5 ファームウェア（`Firmware`）

### 14.5.1 読み込み（`load(image)`、`SPIBus::init` から）

1. 2 つのユーザー設定ブロック（USER0 / USER1）のうち、新しくて有効なほうを選ぶ
2. ファームの設定項目をいくつかパッチする
3. ユーザー設定とヘッダの CRC を計算し直す

`patch_user_settings`：ユーザー設定（GBATEK "DS Firmware User Settings"）を直し、両方のスロットへ同じ内容を書きます。タッチ較正は ADC `x << 4, y << 4`（`TouchScreen` が返す値）を画素 `x, y` に対応させます（melonDS と同じ）。

### 14.5.2 フラッシュのコマンド（`transfer_data`）

チップ選択後の最初のバイトがコマンドです（melonDS `FirmwareMem::Write`）。

| コマンド | 名前 | 内容 |
|---|---|---|
| `0x03` / `0x0B` | READ / FAST READ | 続く 3 バイトがビッグエンディアンのアドレス（FAST READ はさらにダミー 1 バイト）。以降のバイトでデータを順に返す |
| `0x02` / `0x0A` | PAGE PROGRAM / PAGE WRITE | アドレス 3 バイトの後、書き込み許可ラッチが立っていればメモリ上のイメージに書く |
| `0x04` / `0x06` | WRDI / WREN | 書き込み許可ラッチ（ステータス bit 1）を下ろす / 立てる。コマンドバイトそのもので効く |
| `0x05` | RDSR | ステータスを返す |
| `0x9F` | RDID | `0x20, 0x40, 0x12` |
| その他 | | `0xFF` |

`release()` で次のバイトが新しいコマンドになり、書き込み系のコマンドの後は書き込みラッチを下ろします。

**WREN / WRDI はコマンドバイトで処理する。** 次のバイトで処理していたときは書き込みラッチが立たず、SDK の NVRAM 書き込みが永遠に再試行していました。

### 14.5.3 MAC アドレス

`set_mac_address(mac)` はヘッダ `0x36..0x3B` を書き換え、Wi-Fi 設定の CRC（`0x2A`）を計算し直します。`power_on` で `Config::mac_address` があれば呼ばれます。一緒に遊ぶ本体どうしが同じ MAC だと、ホストが子機を区別できません（[15 章](15_wifi_multiplayer.md)、[17 章](17_frontend_common.md) の `Settings::ensure_mac`）。

### 14.5.4 設計判断

- **両方のスロットにパッチする。** 内蔵イメージのユーザー設定は空です。libnds や Nitro SDK は、2 つのスロットを更新カウンタと CRC で選びます。片方だけ直すと、もう片方の空の設定が読まれ、タッチ較正がすべて 0 になって、どこを触っても画素 (0, 0) になりました。
- **CRC の実装は CorgiDS の `Firmware::create_CRC` / `verify_CRC` の忠実な移植です。**

## 14.6 タッチスクリーン（`TouchScreen`）と入力の流れ

```text
from: GUI が run() の前に（egui：input.rs Touch::apply、emu_thread：apply_touch、headless：TOUCH=）
Emulator::touchscreen_press(x, y)          y == 0xFFF は「触れていない」
  1. EXTKEYIN bit 6（ペン。0 = 触れている）
  2. spi.touchscreen_press(x, y) → TouchScreen::press_event：ADC 値として x << 4, y << 4 を保持
to:   ARM7 のタッチ処理が SPI（機器 2）で TSC を読み、結果を ARM9 へ
      （IPC FIFO のタグ 6、または自動サンプリングのバッファ）
```

TSC の `transfer_data`：制御バイトの bit 4-6 でチャネルを選び、続く 2 バイトで 12 ビットの結果を返します。チャネル 1 = タッチ Y、5 = タッチ X、6 = バッテリ/補助（`0x800`）、それ以外（圧力・マイク・温度）は `0xFFF` です。`deselect()` で転送位置をリセットします。

### 14.6.1 タップを取りこぼさない工夫（GUI 側）

GUI はペンを離しても最低 `TOUCH_MIN_FRAMES`（4 フレーム）押したままにします（`gui/egui/src/input.rs`、`gui/common/src/emu_thread.rs`）。SDK はタッチを 1 フレームに 1 回以下しかサンプリングしないため、1 回の描画の間に押して離した短いタップが届かなかったためです。

## 14.7 キー入力

| レジスタ | アドレス | CPU | ビット | 極性 |
|---|---|---|---|---|
| KEYINPUT | `0x04000130` | 両方 | 0 A、1 B、2 SELECT、3 START、4 右、5 左、6 上、7 下、8 R、9 L | 0 = 押下（未使用ビットは 1） |
| EXTKEYIN | `0x04000136` | ARM7 | 0 X、1 Y、6 ペン、7 ヒンジ | X/Y/ペンは 0 = 押下・接触、ヒンジは 1 = 閉じている。未使用ビットは 1 |

```text
from: GUI の入力（egui：input.rs apply_keyboard、ds_common の gamepad、Tauri のコマンド、headless：PRESS=）
button_*_pressed / button_*_released
  A B SELECT START 十字 R L → key_input（KEYINPUT）
  X Y                      → ext_key_in（EXTKEYIN）
to:   ゲームのキー読み取り（X/Y/ペン/ヒンジは ARM7 が共有 RAM 0x027FFFA8 経由で ARM9 へ渡す）
```

GUI のキー割り当て（全 GUI 共通）：矢印 = 十字キー、X = A、Z = B、S = X、A = Y、Q = L、W = R、Enter = START、Backspace = SELECT。

## 14.8 RTC（`RealTimeClock`、`0x04000138`）

`write(value, is_byte)`：bit 0 = データ、bit 1 = クロック（アクティブロー）、bit 2 = 選択、bit 4 = 方向（1 = 書き込み）。選択の立ち上がりで入力をリセットし、クロックに合わせて 1 ビットずつ受け取ります。コマンドバイトがそろうと `interpret_input` が処理します。

| コマンド | 読み出し | 書き込み |
|---|---|---|
| 0 | ステータス 1 | ステータス 1 |
| 1 | アラーム 1 | アラーム 1（曜日・時・分） |
| 2 | 日時（年・月・日・曜日・時・分・秒、BCD。12 時間制なら bit 6 が PM） | 何もしない |
| 3 | 時刻 | 未実装（時刻調整） |
| 4 | ステータス 2 | ステータス 2 |
| 5 | アラーム 2 | アラーム 2 |
| 6 | クロック調整 | |

日時は既定でホストのローカル時刻です。`set_fixed_time(Some((年, 月, 日, 時, 分, 秒)))` で固定でき、テストの再現に使います。

## 14.9 制約・未実装

- ゲームからの日時の書き込み（コマンド 2 / 3）は無視します（ホスト時刻に従います）。
- TSC の圧力・マイク・温度チャネルは固定値 `0xFFF` です（マイク入力なし）。
- ファームウェアへの書き込み（ゲームからのユーザー設定の保存）はメモリ上のイメージだけで、ファイルには書き戻しません。

## 14.10 関連テスト

- ハートゴールドのセーブ選択画面でタッチが効くこと（較正のパッチ、TSC、IPC の確認）
- headless 例の `TOUCH=x:y:start:every[:end]` と `PRESS=key:start:every[:end]` による操作（[19 章](19_testing.md)）
