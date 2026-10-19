# 2. 1 フレームの実行とスケジューラ

## 2.1 責務

`Emulator::run()` 1 回で DS の 1 フレーム（263 ライン、560190 バスサイクル）を進めます。2 つの CPU と、タイマ・3D ジオメトリ・SPU・Wi-Fi・GPU のライン処理・DMA・カートリッジ転送を、短い時間単位（スライス）で交互に進める「キャッチアップ方式」です。

受け持たないもの：実時間との同期（何フレームをいつ実行するか）は GUI 側の責務です（[17 章](17_frontend_common.md)、[18 章](18_gui.md)）。

## 2.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/system/src/emulator/runner.rs` | `Emulator::run`、`Emulator::execute`、トレース出力 |
| `core/system/src/emulator/mod.rs` | `calculate_system_timestamp`、`add_gpu_event`、`reschedule_gpu_event`、`add_dma_event`、`irq_line_raised`、`requesting_interrupt`、`get_timestamp` |
| `core/system/src/emulator/gpu/gpu_3d.rs` | `gpu_handle_event`（ライン処理） |
| `core/gpu/src/display/regs.rs` | `SchedulerEvent`、`DispStatReg` |

## 2.3 データ構造

| フィールド | 型 | 単位 | 内容 |
|---|---|---|---|
| `system_timestamp` | `u64` | バスサイクル（33.51 MHz） | 現在のスライスの終わりの時刻 |
| `next_event_time` | `u64` | 同上 | 次に来るイベントの時刻（`calculate_system_timestamp` が毎回計算し直す） |
| `gpu_event` | `SchedulerEvent { id, activation_time }` | 同上 | ライン処理のイベント。`id` 0 = HBlank 開始、1 = HBlank 終了 |
| `dma_event` | `SchedulerEvent { id, activation_time, processing }` | 同上 | 保留中の DMA。`id` はチャンネル番号。同時に 1 件だけ |
| `arm9.timestamp` | `u64` | ARM9 クロック（バスの 2 倍） | ARM9 が実行済みの時刻 |
| `arm7.timestamp` | `u64` | バスサイクル | ARM7 が実行済みの時刻 |
| `arm*.last_timestamp` | `u64` | 各 CPU の単位 | 直前の `execute` 開始時の時刻（`cycles_ran()` の基準） |

1 フレームの長さ：1 ライン = 355 ドット × 6 サイクル = 2130 サイクル。263 ライン × 2130 = 560190 サイクル。フレームレートは 33.513982 MHz ÷ 560190 ≈ 59.8261 fps（`ds_common::DS_FPS`）です。

## 2.4 インタフェース

| 関数 | 呼び出し元 | 内容 |
|---|---|---|
| `run()` | GUI（毎フレーム）、headless 例 | 1 フレーム実行。入力は呼ぶ前に反映済みであること |
| `execute(cpu)` | `run()` | 1 命令実行、または HALT/DMA 中の待機 |
| `calculate_system_timestamp()` | `run()` | 次のスライスの終わりを決める |
| `add_gpu_event(id, rel)` | `power_on()` のみ | 現在時刻 + `rel` にライン処理を予約 |
| `reschedule_gpu_event(id, rel)` | `gpu_handle_event()` の最後 | **前回の予定時刻** + `rel` に次のライン処理を予約 |
| `add_dma_event(ch, rel)` | DMA の起動要因（[7 章](07_dma.md)） | 現在時刻 + `rel` に DMA を予約（後勝ち） |

## 2.5 処理詳細

### 2.5.1 `run()`

```text
run()
 1. gpu.start_frame()          frame_complete を下ろす
    wifi.new_frame()           通信リンクの送受信処理（ds_net）
    run_cheats()               Action Replay コード（16 章）
 2. gpu.is_frame_complete() になるまで繰り返す
    a. before = system_timestamp
       calculate_system_timestamp()          slice = 増えた分
    b. ARM9：arm9.timestamp < system_timestamp * 2 の間
         execute(Arm9)
         ran = (arm9.timestamp >> 1) - (arm9.last_timestamp >> 1)
         run_timers9(ran)
         run_3d(ran)                         GX コマンド実行（9 章）
    c. ARM7：arm7.timestamp < system_timestamp の間
         execute(Arm7)
         run_timers7(arm7.cycles_ran())
         run_spu(arm7.cycles_ran())          1024 サイクルごとに 1 サンプル（12 章）
    d. wifi.run(slice) → raise_wifi_irq()
    e. system_timestamp >= gpu_event.activation_time → gpu_handle_event()
    f. dma_event.processing かつ期限到来 → dma_handle_event()
    g. cartridge_run(slice)                  ROM データ転送（13 章）
 3. cart.save_check()          書き込みが止んでいれば .sav を書き出す
```

`frame_complete` はライン 192（VBlank 開始）で立ちます。したがって `run()` が返る時点は「VBlank の始まり」で、画面バッファにはライン 0〜191 が描き終わっています。

### 2.5.2 `calculate_system_timestamp()`

```text
next = gpu_event.activation_time
dma_event.processing なら next = min(next, dma_event.activation_time)
next_event_time = next
cycles = next - system_timestamp（wrapping、i32 として解釈）
system_timestamp += cycles が 1..=64 なら cycles、それ以外（期限切れ・遠い）は 64
```

`SLICE_CYCLES = 64`。スライスが大きいほどループ回数が減り速くなりますが、CPU 間の IPC や割り込みの遅延が粗くなります（CorgiDS は 20）。

### 2.5.3 `execute(cpu)`

```text
execute(cpu)
 1. last_timestamp = timestamp
 2. HALT 中 または DMA 転送中（dma_active）
      timestamp = スライスの終わり（ARM9 は system_timestamp * 2）
      IE & IF != 0 なら HALT 解除（irq_line_raised、IME は見ない）
      IME かつ CPSR.I が 0 なら handle_irq()
      return
 3. トレース（Config::trace_trigger / trace_cpus）
 4. フェッチ：Thumb は pc-2 を read_halfword、ARM は pc-4 を read_word
    record_pc、コードの待ちサイクル加算、r15 を進める
 5. thumb_interpret() / arm_interpret()   デコードしてハンドラ実行（3 章）
 6. 割り込み要求あり（requesting_interrupt）かつ CPSR.I が 0 → handle_irq()
```

### 2.5.4 ライン処理 `gpu_handle_event()`

1 ラインに 2 回呼ばれます。

```text
id 0 = HBlank 開始（ドット 256）
  1. 表示ライン（0..191）かつフレームスキップ対象外 → gpu.draw_scanline()（8 章）
  2. DISPSTAT の HBlank フラグ ON、HBlank IRQ（ARM9/ARM7 それぞれ許可されていれば）
  3. 表示ライン → hblank_dma_request()
  4. reschedule_gpu_event(1, 99 * 6)
id 1 = HBlank 終了（次のライン）
  1. HBlank フラグ OFF、V カウント一致フラグと IRQ（CPU ごとの設定値で判定）
  2. vertical_count += 1
  3. ライン 215：gpu.render_3d_frame()（次のフレームで表示される 3D を描く）
  4. ライン 192：VBlank 開始
       engine_3d.end_of_frame()（ポリゴンバッファの入れ替え、9 章）
       frame_complete = true、VBlank IRQ、2D の vblank_start()
  5. ライン 263：0 に戻す、VBlank フラグ OFF、フレームスキップのカウンタ
  6. reschedule_gpu_event(0, 256 * 6)
```

## 2.6 定数

| 定数 | 値 | 意味 |
|---|---|---|
| `SLICE_CYCLES` | 64 | スライスの最大長（バスサイクル） |
| ドット | 6 サイクル | |
| 描画期間 | 256 ドット = 1536 サイクル | |
| HBlank | 99 ドット = 594 サイクル | |
| ライン | 355 ドット = 2130 サイクル | |
| フレーム | 263 ライン = 560190 サイクル | 表示 192、VBlank 71 |
| 3D 描画ライン | 215 | `render_3d_frame` |
| `DS_FPS` | 59.826098 | `gui/common/src/lib.rs` |

## 2.7 設計判断と理由

- **ARM9 の経過時間は「両方を半分にしてから引く」。** `cycles_ran() >> 1` だと、1 サイクルの命令が 0 に丸められます。GXSTAT やタイマを待つ短いループで 3D とタイマが永遠に進まなくなりました（ある市販ソフトで GXSTAT の busy 待ちが終わらなかった）。
- **次のライン処理は「前回の予定時刻」から数える。** 現在時刻から数えると処理の遅れが積み重なり、1 フレームが約 2% 長く（572k サイクル）なっていました。その結果 SPU が実時間 1 秒あたり約 33.4 kHz の音を作り、音声キューがあふれていました。
- **`get_timestamp()` は `system_timestamp` を返す。** CorgiDS の構造体に残る未使用の `total_timestamp` を返していたときは、HALT 中の CPU が自分の時刻を巻き戻してしまい、最初の HALT や DMA 待ちで実行ループが止まりました。
- **`next_event_time` は毎回計算し直す。** 以前は小さくなる方向にしか更新されず、最初のイベントの後はずっと過去の値のままでした。
- **期限切れのイベントでもスライスは 64。** CorgiDS の仕様に合わせています。期限切れのイベントは同じループ内の e/f で処理されます。

## 2.8 制約・未実装

- DMA イベントは同時に 1 件だけです。後から予約した DMA が前の予約を上書きします（実用上は即時起動で直後に処理されるため問題になっていません）。
- サイクル精度は命令単位です。メモリの待ちサイクルは `ArmCpu` の表で近似しています（[3 章](03_cpu.md)）。

## 2.9 関連テスト

- 実 ROM でのフレームレート確認：`ds_common` の `#[ignore]` テスト（`emu_thread.rs`、約 60 fps で出ること）
- headless 例の `HDBENCH`（毎フレームの `run()` の時間）（[19 章](19_testing.md)）

## 2.10 ROM 実行テスト（機能 ID `scheduler`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature scheduler` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `audio_rate` | 1 フレームの長さが 560190 サイクルからずれていない（ずれると 1 フレームあたりのサンプル数が変わる。2.7 の 2% の不具合の再発検知） |
| `screen_changes` | 時間待ちのループで止まらない |
| `max_ms_per_frame` | `--jobs 1` で実行したときの速度の目安（参考値） |

```toml
[[case]]
feature = "scheduler"
name = "scheduler-example"
rom = "my_game"            # [roms] の別名
frames = 900
[case.expect]
audio_rate = true
screen_changes = true
max_ms_per_frame = 8.0
```

GXSTAT やタイマを短いループで待つゲームを入れると、ARM9 の時間の丸め（2.7）の回帰を検知できます。
