# 19. テストと検証手順

## 19.1 方針

- **ユニットテスト**は、実 ROM なしで確認できる部品（CPU の CP15、IPC、DMA 以外の周辺機器、高解像度の補助関数、テンポ変換、音声キュー、チート、設定の往復）に置きます。
- **実 ROM のテスト**は `#[ignore]` にし、環境変数で ROM を渡したときだけ動かします。ROM はリポジトリに含めません。
- **描画の回帰**は、headless 例（git 管理外）で PNG を出力し、変更前のビルドとビット単位で比べます。
- **機能ごとの実 ROM テスト**は `tools/rom_test` で、手元の設定（Git 管理外）に書いたケースをまとめて実行し、基準のハッシュと比べます（[20 章](20_rom_tests.md)）。
- 実 ROM・実セーブを使うときは、**セーブを必ず別フォルダに分けます**（`SAVE_DIR`、インスタンスフォルダ、`Config::save_dir`）。元のセーブを書き換えないためです。

## 19.2 テスト一覧

| 対象 | 場所 | 内容 | 章 |
|---|---|---|---|
| CP15 | `core/cpu/src/coprocessor_15.rs` | `dtcm_window_and_readback`、`halt_and_control` | [3](03_cpu.md) |
| IPC | `core/peripherals/src/ipc.rs` | `sync_output_reaches_remote_input`、`sync_irq_requires_remote_enable`、`fifo_crosses_cpus` | [6](06_interrupt_timer_ipc.md) |
| Wi-Fi | `core/peripherals/src/wifi/tests.rs` | 擬似リンクでのコントローラ | [15](15_wifi_multiplayer.md) |
| MP（FFI） | `core/net/tests/local_mp.rs` | LocalMP の 2 台でフレームが届く | [15](15_wifi_multiplayer.md) |
| 準備ランプ | `gui/net/src/ready.rs` | ループバックで 2 つのプローブが互いを見つける | [15](15_wifi_multiplayer.md) |
| チート | `core/system/src/emulator/cheats.rs` | `parses_pairs`、`data_ops`、`runs_codes_on_main_ram` | [16](16_cheats.md) |
| 高解像度 | `core/gpu/src/display/hires.rs` | `nearest_scales_rows_and_columns`、`xbrz_keeps_flat_areas`、`xbrz_bands_match_a_whole_image_pass`、`shading_matches_native_rules` | [11](11_hires.md) |
| テンポ変換 | `gui/common/src/speed.rs` | `cycles_through_all_speeds`、`labels`、`ratio_follows_the_real_speed`、`faster_keeps_the_pitch`、`slower_keeps_the_pitch` | [12](12_sound.md) |
| 音声キュー | `gui/common/src/audio.rs` | `bursty_producer_does_not_underrun` | [12](12_sound.md) |
| チート一覧 | `gui/common/src/cheats.rs` | `only_enabled_valid_codes_run`、`round_trips_through_a_file` | [16](16_cheats.md) |
| 設定 | `gui/common/src/frontend.rs`、`gui/egui/src/config.rs` ほか | 3 つの GUI の `config.json` の往復 | [17](17_frontend_common.md) |
| エミュレータスレッド（実 ROM） | `gui/common/src/emu_thread.rs` | `thread_produces_frames`（約 60 fps でフレームが出る）、`pause_and_step`、`speed_and_resolution` | [17](17_frontend_common.md) |
| React | `gui/tauri/src/**/*.test.ts(x)` | メニュー、画面、チート画面、API | [18](18_gui.md) |

## 19.3 実行方法

| 目的 | コマンド |
|---|---|
| ユニットテスト（主要） | `cargo test --release -p ds_gpu -p ds_common` |
| 全体 | `cargo test --release --workspace` |
| 実 ROM のテスト | 環境変数 `LUNARIS_TEST_ROM` に ROM のパスを設定し、`cargo test --release -p ds_common` に ignored テストを含めるオプションを付けて実行 |
| 個別の実 ROM テスト | 上と同じで、テスト名 `pause_and_step` や `speed_and_resolution` を指定 |
| React | `gui/tauri` で npm のテストスクリプト |
| 静的解析 | `cargo clippy --workspace` |

エミュレータスレッドのテスト `thread_produces_frames`（`gui/common/src/emu_thread.rs`）は、`--release` でないと時間内に通りません。

## 19.4 headless 例（`core/system/examples/headless.rs`）

git 管理外（`.gitignore` の `core/system/examples/`）の検証用プログラムです。なくなっていたら作り直します。

ビルド：`cargo build --release -p ds_emu --example headless`（ログが必要なら `--features log`。その場合は `CARGO_TARGET_DIR=target/logbuild` で別のフォルダにすると、通常のビルドを毎回やり直さずに済みます）。

実行：`headless ROM フレーム数 出力フォルダ`

| 環境変数 | 書式 | 内容 |
|---|---|---|
| `SAVE_DIR` | フォルダ | セーブの置き場所（必ず作業用のフォルダに） |
| `PRESS` | `キー:開始:間隔[:終了]` のカンマ区切り | ボタン（a b x y l r start select up down left right）。1 回 6 フレーム押す |
| `TOUCH` | `x:y:開始:間隔[:終了]` | タッチ（6 フレーム） |
| `PNG_EVERY` | n | n フレームごとに両画面を PNG に |
| `HD` | `倍率[:x]` | 高解像度の画面も PNG に（`:x` で xBRZ） |
| `HDBENCH` | `倍率[:x]` | 毎フレーム両画面を拡大し、`run()` と高解像度処理の時間を 300 フレーム平均で表示 |
| `PCS` | n | n フレームごとに pc9 / pc7 / HALT / CPSR を表示 |
| `TRACE` | `マスク:開始:終了` | 命令ごとのトレース（1 = ARM9、2 = ARM7。1 フレーム約 15 万行なので範囲を絞る） |
| `LOGF` | `開始:終了` | stderr のロガーを有効に（`--features log` が必要）。ターゲット：`card`、`auxspi`、`ipc`、`spi`、`dma` |
| `DUMP` | `16 進アドレス:長さ` | 終了時に ARM9 から見たメモリを書き出す |
| `HIST` | 1 | 分岐の履歴（`ArmCpu::recent_pcs`） |
| `POKE` | `16 進アドレス:バイト:フレーム` | ARM9 のバイト書き込み |
| `WATCH` | 16 進アドレス | そのワードへの書き込みを表示（`Config::watch_addr`） |
| `GUI` / `WAV` | | 音声を取り出して GUI と同じ処理を通す / さらに WAV ファイルに書く |

## 19.5 描画の回帰確認

1. 変更前のコミットを別の作業ツリーに取り出す（`git worktree add --detach 作業フォルダ HEAD`）。headless 例は git 管理外なので、作業ツリーにコピーする。
2. 両方を、別々の `CARGO_TARGET_DIR` でビルドする。
3. 同じ入力で、次の場面を等倍・2 倍・3 倍 xBRZ・4 倍 xBRZ で PNG に出力する。
   - 3D のタイトル画面を持つソフト（約 620 ポリゴン、下画面 = エンジン A）：START を押してからおよそ 2100〜2300 フレーム
   - 3D のステージ紹介を持つソフト：実セーブを作業用フォルダにコピーし、START / A をおよそ 300 フレームごと。2400 フレーム以降
4. 等倍の画像を md5 で比べる（変わっていないこと）。高倍率の画像は目で確認する。

確認したい場面がない場合は、`PCS=1` で最初におかしくなるフレームを探し、そのフレームだけ `TRACE` して分岐先を集計します。SDK のゲームで `OS_Terminate` に入ると、CPSR の I ビットが立った CP15 の WFI ループになります（通常の待機は IRQ が有効なまま）。

### 19.5.1 注意点

- 一部のゲームの確認ダイアログは既定が「いいえ」なので、A の連打では先に進みません。先に左を押します。
- ビルドによってタイミングが少し変わるので、変更前後の比較にはタイミングに強い入力（間隔の広い連打など）を使います。
- 以前から知られている表示の問題（回帰ではない）：あるソフトの緑の画面など。

## 19.6 GUI の確認

- 配布用の実行ファイルは、別のフォルダにコピーして使っている場合があります。変更を確認するときは、ビルドした実行ファイルをコピーし直してください（古いコピーのままだと「変わっていない」ように見えます）。
- freya はヘッドレスの PNG スナップショットで画面を確認できます（[18 章](18_gui.md)）。
