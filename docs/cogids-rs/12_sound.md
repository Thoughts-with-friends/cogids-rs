# 12. 音声（SPU・ホスト出力・テンポ変換）

## 12.1 責務

| 層 | 場所 | 受け持ち |
|---|---|---|
| SPU | `core/spu`（`ds_spu`） | 16 チャンネルの音源、ミキサ、キャプチャ 2 系統。ステレオ `i16` を約 32728 Hz で出力 |
| SPU の駆動 | `core/system/src/emulator/sound.rs` | ARM7 の時間で SPU を進め、メモリ（サンプルデータ）へのアクセスを仲介 |
| テンポ変換 | `gui/common/src/speed.rs` | 倍速・0.5 倍で、音程を変えずにテンポだけを変える（WSOLA） |
| ホスト出力 | `gui/common/src/audio.rs` | `cpal` で再生。デバイスのレートへのリサンプリング、バッファ量の制御 |

## 12.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/spu/src/lib.rs` | `Spu`（別名 `SPU`）、`SoundBus` トレイト、`SAMPLE_RATE`、`CYCLES_PER_SAMPLE`、`OUTPUT_RATE_HZ`、レジスタの読み書き |
| `core/spu/src/channel.rs` | `Channel`（PCM8 / PCM16 / ADPCM / PSG / ノイズ、32 バイト FIFO） |
| `core/spu/src/adpcm.rs` | `decode_nibble`（IMA-ADPCM） |
| `core/spu/src/mixer.rs` | `Spu::mix`、`take_samples`、`clear_samples` |
| `core/spu/src/capture.rs` | `Capture`（SNDCAPx） |
| `core/system/src/emulator/sound.rs` | `SpuBus`、`run_spu`、`take_audio_samples` |
| `gui/common/src/speed.rs` | `SPEEDS`、`SpeedMeter`、`stretch_ratio`、`TimeStretch` |
| `gui/common/src/audio.rs` | `AudioOutput`、`rate_step` |

## 12.3 SPU

### 12.3.1 定数

| 定数 | 値 | 意味 |
|---|---|---|
| `SAMPLE_RATE` | 32768 | 名目上の出力レート（Hz） |
| `CYCLES_PER_SAMPLE` | 1024 | 1 サンプルあたりの ARM7 バスサイクル |
| `OUTPUT_RATE_HZ` | 33513982 / 1024 ≈ **32728.5** | 実際の出力レート。ホスト側のリサンプリングはこちらを使う |

`run_spu` の関数コメントには「32768 Hz」とありますが、実際に 1 秒間に出るサンプル数は `OUTPUT_RATE_HZ` です。

### 12.3.2 レジスタ（ARM7、`0x04000400..0x0400051F`）

| アドレス | レジスタ | 内容 |
|---|---|---|
| `0x04000400 + x*0x10` | SOUNDxCNT | 音量（0〜127）、音量シフト、パン、PSG デューティ、リピート（1 = ループ、2 = 1 回）、形式（0 = PCM8、1 = PCM16、2 = ADPCM、3 = PSG/ノイズ）、bit 31 = 開始 |
| `+0x04` | SOUNDxSAD | 開始アドレス（`& 0x07FFFFFC`） |
| `+0x08` | SOUNDxTMR / PNT | タイマの再設定値（下位 16 ビット）、ループ開始位置（上位、ワード単位） |
| `+0x0C` | SOUNDxLEN | ループ後の長さ（ワード単位、21 ビット） |
| `0x04000500` | SOUNDCNT | bit 0-6 マスター音量、bit 8-9 / 10-11 左右の出力元、bit 12/13 チャンネル 1/3 をミキサに入れない、bit 15 有効 |
| `0x04000504` | SOUNDBIAS | 10 ビット |
| `0x04000508` / `0x04000509` | SNDCAP0CNT / SNDCAP1CNT | キャプチャ制御 |
| `0x04000510` / `0x04000518` | SNDCAPxDAD | 書き込み先 |
| `0x04000514` / `0x0400051C` | SNDCAPxLEN | 長さ（ワード単位、最小 1 ワード） |

チャンネル 1 / 3 のタイマ設定は、キャプチャ 0 / 1 のタイマにも書き込みます（キャプチャはそのタイマで動く）。

### 12.3.3 チャンネル（`Channel::run`）

```text
run(cycles, bus)                      cycles = 512（SPU クロック 16.76 MHz で 1 サンプル分）
  1. 開始ビットが 0 → 0
  2. 種類：形式 0〜2 はそのまま、形式 3 はチャンネル 8〜13 で PSG、14〜15 でノイズ、それ以外は無音
  3. PCM/ADPCM で loop_pos + length < 16 → 0（データが短すぎる）
  4. key_on（開始ビットの 0→1）なら start()：タイマ再設定、位置リセット、FIFO の先読み
  5. timer += cycles。16 ビットを超えるたびに timer = 再設定値 + 超過分 で次のサンプルへ
       PCM8 / PCM16 / ADPCM：FIFO（32 バイト。半分空いたら fifo_fill で 16 バイト補充）から読む
       ADPCM：先頭 4 バイトのヘッダ（初期値・索引）、以降 4 ビットずつ decode_nibble
       PSG：PSG_TABLE[デューティ] の 8 段階
       ノイズ：15 ビット LFSR
       データの終わり：wrap_or_stop（ループならループ位置へ、1 回なら停止）
  6. 戻り値 = (cur_sample << volume_shift) * volume
```

### 12.3.4 ミキサ（`Spu::mix`）

```text
from: run_spu()、ARM7 の 1024 サイクルごと
mix(bus)
  1. SOUNDCNT bit 15 が 0 → 無音
  2. 16 チャンネルを run(512)
  3. pan_output で左右に加算（SOUNDCNT bit 12/13 でチャンネル 1/3 を除外）
  4. キャプチャ 0/1 が有効なら、左 / 右のミキサ出力（>> 8 して i16 に制限）を capture.run → メモリ
  5. 出力元（SOUNDCNT bit 8-11）：0 = ミキサ、1 = チャンネル 1、2 = チャンネル 3、3 = 1 + 3
  6. マスター音量（127 は 128 として扱う）を掛け、>> 7 >> 8、i16 に制限して 1 組追加
     （市販ソフトが使う SOUNDBIAS 0x200 は符号なし DAC の直流成分なので、ここでは加えない）
  7. 取り出されないまま約 2 秒分（SAMPLE_RATE × 4 個）たまったら古いものから捨てる（headless 用）
```

### 12.3.5 キャプチャ（`Capture`）

キャプチャ 0 は左ミキサ、1 は右ミキサを記録します。チャンネル 1 / 3 のタイマで動き、オーバーフローごとに PCM8 または PCM16 として書き込みます。16 バイトたまるごとに `flush` でメモリへ書き、長さの終わりでループまたは停止します。

### 12.3.6 SPU とメモリ（`SpuBus`）

SPU はメモリを所有せず、呼び出し側の `SoundBus` を通して読み書きします。`SpuBus` はメイン RAM、共有 WRAM、ARM7 WRAM だけを見せ（サンプルデータはここにある）、それ以外は 0 を読み、書き込みを捨てます。

## 12.4 SPU の駆動（`sound.rs`）

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `run_spu(cycles)` | `run()`、ARM7 の 1 命令ごと | `spu_cycles += cycles`。1024 を超えるごとに `spu.mix(bus)` |
| `take_audio_samples(out)` | GUI（`run()` の後） | たまったインターリーブのステレオ `i16` を `out` へ移す |

## 12.5 テンポ変換（`speed.rs`）

### 12.5.1 速度の選択肢と計測

| 項目 | 内容 |
|---|---|
| `SPEEDS` | `[50, 100, 150, 200, 300, 400]`（%）。整数の百分率で持つので正確に比較できる |
| `valid(p)` / `next(p)` / `label(p)` / `fps(p)` | 不正値は 100、次の速度（4 倍の次は 0.5 倍）、`"0.5x"` などの表示、`DS_FPS × p / 100` |
| `SpeedMeter` | 実際の速度（エミュレート秒 / 実秒）を約 0.5 秒（`METER_SECS`）で平滑化。`tick(frames)` は前回からのフレーム数を受け取る |
| `stretch_ratio(percent, measured)` | テンポ比 = 実測値を `[0.25, 設定速度]` に制限 |

### 12.5.2 `TimeStretch`（WSOLA）

```text
 入力  |--グレイン k--|               名目上の開始位置は HOP × ratio ずつ進む。
               |--グレイン k+1--|     実際の開始は ±SEEK の範囲で、前の続き（tail）に
                                      最も似た位置（モノラル和の正規化相互相関）を選ぶ
 出力  |--k--|xx|--k+1--|xx|          xx = OVERLAP 分をクロスフェード
```

| 定数 | 値 | 時間（32728 Hz） |
|---|---|---|
| `GRAIN` | 1200 | 約 37 ms |
| `OVERLAP` | 240 | 約 7 ms |
| `SEEK` | 300 | 約 9 ms |
| `HOP` | `GRAIN - OVERLAP` = 960 | 出力側の 1 グレイン |

| 関数 | 内容 |
|---|---|
| `process(samples, ratio, out)` | `samples` を追加し、ここまでに変換できた分を `out` に書く（`out` は先にクリア）。`ratio` は 0.25〜8 |
| `best_start(nominal)` | `nominal ± SEEK` のうち、最初の `OVERLAP` が `tail` に最も似ている開始位置 |
| `reset()` | バッファを捨てる（一時停止・1 倍・停止時） |

## 12.6 ホスト出力（`audio.rs`）

### 12.6.1 定数

| 定数 | 値 | 意味 |
|---|---|---|
| `SOURCE_RATE` | `OUTPUT_RATE_HZ` | 入力レート |
| `TARGET_FRAMES` | 2300 | 目標のキュー量（約 70 ms、エミュレートの 4 フレーム強） |
| `MAX_FRAMES` | `TARGET_FRAMES × 3` | これを超えたら目標まで古い音を捨てる |
| `MAX_RATE_ADJUST` | 0.005 | リサンプリング比の最大補正（±0.5%） |

### 12.6.2 処理

```text
AudioOutput::new(volume)
  cpal のストリームを専用スレッドで作成・保持。デバイスがなければ無音で続ける（ログ）
push(samples)                 GUI / エミュレータスレッドから、1 フレーム分ずつ
  キューに追加。MAX_FRAMES を超えたら目標まで捨てる
デバイスのコールバック
  再生中でなければ、目標量がたまるまで待つ（バッファリング中は最後のサンプルを fade() で無音へ）
  step = rate_step(SOURCE_RATE / デバイスレート, キュー量)
         目標より多ければ少し速く、少なければ少し遅く消費（最大 ±0.5%）
  next(step)：frames[0] と frames[1] の間を線形補間
  キューが空になったら（アンダーラン）再びバッファリングへ
clear()                       停止・一時停止でキューを捨てる（再開時に再バッファリング）
stats() → (アンダーラン回数, 捨てたフレーム数)
```

### 12.6.3 フロントエンドでの流れ

```text
1 フレームのサンプル（take_audio_samples）
   │
   ├ 速度が 1 倍 → そのまま AudioOutput::push
   └ それ以外    → TimeStretch::process(stretch_ratio(設定, SpeedMeter::speed())) → push
リミッタ解除（早送り）中 → 音を出さない
```

egui は 1 回の描画で複数フレームを実行するので、実行したフレーム数を `SpeedMeter::tick(ran)` に渡して、実時間あたりの速度を測ります。

## 12.7 設計判断と理由

- **以前は 1 フレームごとにサンプルを間引いて（水増しして）いた。** この方式では倍速で音が高くなり、エミュレーションが目標速度に届かないときは「高いだけで速くない」音になっていました。WSOLA で音程を保ち、melonDS と同じ聞こえ方にしています。
- **フレームをまたいで状態を持つ。** 1 フレーム（約 547 サンプル、17 ms）はグレインより短いので、フレームごとに独立に処理すると 60 Hz のブザー音が出ます。
- **テンポ比は設定値ではなく実測値。** 高解像度などで重く 2 倍に届かないとき、設定値の 2 倍で縮めると音が足りなくなって途切れます。実測値を使えば、出る音の長さが実時間と一致します。
- **1 倍は変換しない。** 遅延と音質の変化を避けるためです。速度変更・一時停止・停止のときは状態をリセットします。
- **再生開始前に約 70 ms ためる。** エミュレータはフレーム単位でまとめて音を作り、デバイスは連続して消費します。少しずつ流すと、フレームのまとめ実行やスケジューリングの揺れでキューが空になります。
- **動的レート制御。** エミュレータの時計とサウンドカードの時計は完全には一致しないので、変換比を最大 ±0.5% 調整して目標量を保ちます。大きく超えたとき（停止の後など）だけ捨てます。
- **`cpal` のストリームは専用スレッドで持つ。** Windows の WASAPI は COM をマルチスレッドモードで初期化します。UI スレッドで作ると、ウィンドウのシングルスレッド COM とぶつかり、起動直後にウィンドウが閉じました。
- **フレームの長さを正確にする。** ライン処理の予約を前回の予定時刻から数えるようにしたことで、1 秒あたりのサンプル数が 33.4 kHz から 32.7 kHz に正しく戻りました（[2 章](02_frame_scheduler.md)）。

## 12.8 制約・未実装

- マイク入力（TSC のマイクチャネル）は実装していません。
- 極端な速度（4 倍）では、グレインのつなぎ目が聞こえることがあります。
- SOUNDBIAS は保存するだけで、出力に加えません。

## 12.9 関連テスト

- `gui/common/src/speed.rs`：440 Hz の正弦波を 1 フレームずつ入れて 2 倍・0.5 倍で変換し、音程が ±3% 以内、出力長が入力 ÷ 比率になることを確認（`faster_keeps_the_pitch`、`slower_keeps_the_pitch`）。ほかに `cycles_through_all_speeds`、`labels`、`ratio_follows_the_real_speed`
- `gui/common/src/audio.rs` の `bursty_producer_does_not_underrun`：揺れのあるフレーム単位の投入とデバイスレートでの消費で、起動後にアンダーランが起きないこと
- 実行：`cargo test --release -p ds_gpu -p ds_common`

## 12.10 ROM 実行テスト（機能 ID `sound`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature sound` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `audio` | SPU から音が出る |
| `audio_rate` | 出力レートが約 32728.5 Hz（1 フレーム約 547 サンプル） |

```toml
[[case]]
feature = "sound"
name = "sound-example"
rom = "my_game"            # [roms] の別名
frames = 900
[case.expect]
audio = true
audio_rate = true
```

テンポ変換とホスト出力（12.5、12.6）は GUI 側の処理なので、このツールでは扱いません（`ds_common` のユニットテストで確かめます）。
