# 20. ROM 実行テスト（`tools/rom_test`）

## 20.1 目的

各機能（1〜16 章）を、**自分の持っている ROM** で実際に動かして確かめます。ROM の名前・パス・内容・画面の画像・ROM ごとの期待値は秘匿情報なので、`docs/cogids-rs/local/`（`.gitignore` で除外）の下にだけ書きます。公開されているホームブリューも、ROM として同じ扱いにします。

Git にあるのは、ツール本体（`tools/rom_test`）、架空の名前だけのひな形（`roms.example.toml`）、VS Code のタスク（`.vscode/tasks.json`）だけです。

## 20.2 すぐに試す

```text
mkdir docs/cogids-rs/local
cp docs/cogids-rs/roms.example.toml docs/cogids-rs/local/roms.toml   # ROM のパスとケースを書く
cargo run --release -p ds_rom_test
```

設定ファイルを作らずに、ROM を 1 本だけ引数で動かすこともできます（20.4.2）。

```text
cargo run --release -p ds_rom_test -- --rom ROM のパス --feature boot --frames 120 --expect no_terminate
```

結果は `docs/cogids-rs/local/results/latest.md` です。

### 20.2.1 ケースの作り方の目安

| 機能 | ROM の選び方 | よく使うチェック |
|---|---|---|
| `boot`、`cpu`、`memory`、`video2d` | 小さなホームブリューで十分。画面の決まった位置の色や、決まったアドレスの値を確かめる | `pixel`、`ram`、`no_terminate` |
| `scheduler`、`sound` | どの ROM でもよい（音を出さない ROM でも SPU はサンプルを出す） | `audio_rate`、`audio` |
| `input` | どの ROM でもよい。押している間の KEYINPUT（16 ビット）を確かめる | `ram`（`0x04000130:16`）、`screen_changes` |
| `cheats` | どの ROM でもよい。メイン RAM の空き領域への書き込みを確かめる | `ram` |
| `hires` | 2D だけの ROM でも等倍の不変を確かめられる。3D があれば高解像度 3D も | `hd`、`pixel`、baseline |
| `bios`、`interrupt`、`dma`、`geometry3d`、`raster3d`、`cartridge`、`wifi` | 割り込み・DMA・3D・セーブ・無線を使う市販ソフトが必要 | `no_terminate`、`polygons_min`、`save_written` |

## 20.3 GitHub で md を見ながら実行するには

GitHub の画面からは、手元のコマンドを実行できません（GitHub はページを表示するだけです）。次のどれかを使います。

| 方法 | 手順 |
|---|---|
| コマンドをコピー | GitHub のコードブロック右上のコピーボタンでコピーし、リポジトリのフォルダで開いたターミナルに貼る |
| VS Code のタスク | コマンドパレット → **Tasks: Run Task** → `rom_test: …` を選ぶ。ターミナルパネルで実行されるので、その間も md を見たり編集したりできる |
| VS Code のターミナル | md のプレビューを横に開き、統合ターミナルに貼る |

`.vscode/tasks.json` のタスク：

| タスク | 内容 |
|---|---|
| `rom_test: 自分の ROM（全ケース）` | `local/roms.toml` |
| `rom_test: 自分の ROM（機能を選ぶ）` | 機能 ID を一覧から選ぶ |
| `rom_test: 基準を更新（自分の ROM）` | `--update-baseline` |
| `rom_test: ROM を 1 本指定` | ROM のパス・機能・フレーム数を入力して `--rom` で実行（パスは Git に残らない） |

## 20.4 実行ファイルと引数

`cargo build --release -p ds_rom_test` で `target/release/rom_test`（Windows は `rom_test.exe`）ができます。ソースは Git 管理、ビルドした exe は `target/` の下なので管理外です。`cargo run --release -p ds_rom_test -- 引数` でも、exe を直接呼んでもかまいません。

### 20.4.1 設定ファイルのケースを実行する

| 引数 | 内容 |
|---|---|
| `--config PATH` | 設定ファイル（既定 `docs/cogids-rs/local/roms.toml`） |
| `--feature ID[,ID]` | その機能のケースだけ |
| `--case TEXT` | 名前に TEXT を含むケースだけ |
| `--jobs N` | 同時に動かすケース数（既定は設定の `jobs`、なければ 2） |
| `--results DIR` | 結果フォルダ（既定は設定の `results`） |
| `--update-baseline` | 今回のハッシュを基準として保存 |
| `--no-png` | 画面をハッシュだけにし、PNG を書かない |
| `--list` | 機能 ID と、設定にあるケースの一覧 |
| `--help` | 使い方 |

### 20.4.2 ROM を 1 本、引数だけで実行する（設定ファイル不要）

```text
rom_test --rom PATH [--feature ID] [--name NAME] [--frames N]
         [--press key:start:every[:end]] [--touch x:y:start:every[:end]]
         [--cheat "XXXXXXXX YYYYYYYY"] [--save-from FILE.sav]
         [--hd N] [--xbrz] [--snapshot FRAME] [--expect KEY[=VALUE]] ...
```

`--press`、`--touch`、`--cheat`、`--snapshot`、`--expect` は何回でも指定できます。`--expect` のキーは `[case.expect]` と同じです（20.6）。例：

```text
cargo run --release -p ds_rom_test -- --rom ROM のパス --feature video2d --frames 30 --expect pixel=lower:0:0=0xF80000 --expect pixel=upper:0:0=0
```

結果は既定で `docs/cogids-rs/local/results/` に書きます。ケース名は `--name`（既定 `cli`）で、ROM のファイル名は使いません。

終了コード：0 = すべて成功（実行するものがない場合も 0）、1 = 失敗したケースあり、2 = 引数や設定の誤り。

## 20.5 ファイル構成

```text
.vscode/tasks.json           VS Code のタスク（Git 管理）
tools/rom_test/              ツール本体（Git 管理）
docs/cogids-rs/
├── roms.example.toml        自分の ROM 用のひな形（Git 管理。名前は架空）
└── local/                   .gitignore で除外
    ├── roms.toml            自分の ROM とケース（ほかの設定ファイルを置いてもよい）
    └── results/
        ├── latest.md        最新の結果（自分の ROM）
        ├── baseline.json    基準のハッシュ
        └── run-<秒>/        report.md、results.json、ケースごとのフォルダ（save/、PNG）
```

基準（`baseline.json`）は結果フォルダの中に置きます。設定ファイルごとに別の基準になります。

## 20.6 設定ファイルの書式

### 20.6.1 `[settings]`

| キー | 既定値 | 内容 |
|---|---|---|
| `results` | `"results"` | 結果フォルダ（設定ファイルからの相対パス）。**`local/` の下にすること** |
| `jobs` | 2 | 同時に動かすケース数 |
| `fixed_time` | `[2026, 1, 1, 12, 0, 0]` | RTC の日時。毎回同じにして結果を再現できるようにする |

### 20.6.2 `[roms]`

`別名 = "ROM のパス"`。相対パスは設定ファイルのフォルダから数えます（`local/roms.toml` に `"game.nds"` と書けば `docs/cogids-rs/local/game.nds`）。ケースの `rom` には別名かパスを書きます。

### 20.6.3 `[[case]]`

| キー | 必須 | 内容 |
|---|---|---|
| `feature` | ○ | 機能 ID（20.7） |
| `name` | ○ | ケース名（設定内で一意。結果フォルダの名前にもなる） |
| `rom` | ○ | `[roms]` の別名、またはパス |
| `frames` | ○ | 実行するフレーム数 |
| `press` | | `"キー:開始:間隔[:終了]"` のリスト。キーは a b x y l r start select up down left right。6 フレーム押す。間隔 0 は 1 回だけ |
| `touch` | | `"x:y:開始:間隔[:終了]"`（x < 256、y < 192）。6 フレーム触れる |
| `cheats` | | Action Replay のコード（1 行に 2 ワード）。毎フレーム実行 |
| `save_from` | | 既存の `.sav`。ケースのセーブフォルダにコピーしてから起動（元のファイルには書かない） |
| `hd` | | スナップショットの内部解像度（2〜4） |
| `xbrz` | | 高解像度のスナップショットで xBRZ を使う |
| `snapshots` | | 画面をハッシュ・PNG にするフレーム（0 起点）。最後のフレームは常に含む |
| `memo` | | メモ（レポートに表示） |
| `[case.expect]` | | 確かめること（20.8） |

## 20.7 機能 ID

| ID | 章 | ID | 章 |
|---|---|---|---|
| `boot` | [1](01_boot.md) | `raster3d` | [10](10_video_3d_raster.md) |
| `scheduler` | [2](02_frame_scheduler.md) | `hires` | [11](11_hires.md) |
| `cpu` | [3](03_cpu.md) | `sound` | [12](12_sound.md) |
| `bios` | [4](04_bios_firmware.md) | `cartridge` | [13](13_cartridge_save.md) |
| `memory` | [5](05_memory_bus.md) | `input` | [14](14_spi_rtc_input.md) |
| `interrupt` | [6](06_interrupt_timer_ipc.md) | `wifi` | [15](15_wifi_multiplayer.md) |
| `dma` | [7](07_dma.md) | `cheats` | [16](16_cheats.md) |
| `video2d` | [8](08_video_2d.md) | `frontend` | [17](17_frontend_common.md) |
| `geometry3d` | [9](09_video_3d_geometry.md) | `gui` | [18](18_gui.md) |

## 20.8 チェック（`[case.expect]` / `--expect`）

| キー | 合格の条件 | 計測のしかた |
|---|---|---|
| `not_blank = ["upper", "lower"]` | 最後のフレームで、指定した画面が単色でない | `get_upper_frame` / `get_lower_frame` |
| `screen_changes = true` | 実行中に両画面の組み合わせが 2 種類以上ある | 毎フレームのハッシュ |
| `pixel = ["lower:X:Y=0xRRGGBB"]` | 最後のフレームの画素の色が一致 | フロントエンドが受け取る値（BGR555 を 8 ビットに広げたもの。赤 `0x1F` は `0xF80000`） |
| `polygons_min = N` | どこかのフレームで 3D ポリゴンが N 個以上 | `gpu.engine_3d.rend_poly_count` の最大値 |
| `audio = true` | 最初の 10 フレームより後に、0 でないサンプルがある | `take_audio_samples` |
| `audio_rate = true` | 1 フレームあたりのステレオサンプル数が約 547.06（±3%） | 32728.5 Hz ÷ 59.826 fps。最初の 10 フレームを除く |
| `save_written = true` | 終了後、ケースのセーブフォルダに 0 バイトより大きい `.sav` がある | `flush_save` の後 |
| `no_terminate = true` | 最後の 60 フレームの終わりのすべてで ARM9 の CPSR.I が立っている、ということがない | SDK の `OS_Terminate` は IRQ を止めて WFI を繰り返す |
| `ram = ["0xADDR[:WIDTH]=0xVALUE"]` | 終了時に ARM9 から見た値が一致。WIDTH は 8 / 16 / 32（既定 32） | `arm9_read_byte` / `_halfword` / `_word` |
| `max_ms_per_frame = 数値` | 平均時間が上限以下（**参考値**。失敗にしない） | 同時実行中は他のケースの影響を受ける |
| （自動）baseline | `baseline.json` にあるスナップショットのハッシュと一致 | 20.9 |

レポートでは ✓ = 合格、✗ = 不合格、△ = 参考値が上限を超えた、を表示します。

I/O レジスタは、ゲームが使う幅で読んでください。例えば KEYINPUT は 16 ビットのレジスタで、ARM9 のワード読み出しには対応していないため `0x04000130:16` と書きます。

## 20.9 再現性と回帰確認

- RTC を `fixed_time` で固定し、MAC はファームの既定値のままにするので、同じビルド・同じ設定なら毎回同じ画面になります（3D ラスタのバンド並列もスレッド数によらず同じ。[10 章](10_video_3d_raster.md)）。手元の ROM で、`--jobs` を変えた 2 回の実行が基準と一致することを確認しています。
- スナップショットのフレームごとに、両画面の等倍と（`hd` があれば）高解像度の画像の FNV-1a ハッシュを `results.json` に記録します。
- `--update-baseline` で保存し、以後の実行は自動的に比較します（不一致は `baseline` の失敗）。描画を変える前に基準を取り、変えた後に実行すれば、[19 章](19_testing.md) 19.5 の手作業の比較を置き換えられます。意図した変更なら、確認後に基準を更新します。

## 20.10 安全のための決まり（コードで強制）

- **セーブは必ずケース専用のフォルダ**（`run-*/ケース名/save/`）に書きます。ROM の隣（本物のセーブ）には書きません。セーブフォルダが ROM のフォルダと同じになる場合はエラーで止めます。
- `save_from` はコピーするだけで、元のファイルには書き戻しません。
- 結果・PNG・セーブ・基準は結果フォルダの下にだけ書きます。

## 20.11 ツールの構成（`tools/rom_test/src`）

```text
main.rs    設定の読み込み（または --rom から 1 ケース）、ケースの選択、ワーカースレッド（スタック 64 MiB）で並列実行、
           基準の読み書き、終了コード
cli.rs     引数の解析、--rom のケースと --expect の組み立て
config.rs  設定の型（serde + toml。未知のキーはエラー）、機能 ID と章の表、検証、相対パスの解決
input.rs   press / touch の解析と、フレームごとの適用（6 フレーム押す）
run.rs     1 ケースの実行：
             prepare   ROM の確認、セーブフォルダ（+ save_from）、RTC 固定、load_rom、チート、解像度
             毎フレーム script.apply → run() → observe（ポリゴン、音声、画面ハッシュ、CPSR.I）
             snapshot  指定フレームと最後のフレームで、両画面（と高解像度）をハッシュ・PNG に
             終了      RAM と画素の読み出し、flush_save、.sav の確認
check.rs   expect と基準から合否を作る
report.rs  report.md / results.json / latest.md、baseline.json
help.txt   --help の文面
```

ツール自身のユニットテスト：`cargo test --release -p ds_rom_test`（入力の書式、`--rom` の組み立て、RAM・画素の指定の解析）。

## 20.12 制約

- 通信対戦（2 台）は扱いません。`core/net/examples/mp_pair.rs` を使います（[15 章](15_wifi_multiplayer.md)）。
- GUI（17・18 章）の操作は扱いません。エミュレータ本体だけを headless で動かします。
- 速度の検査は参考値です。正確に測るときは `--jobs 1` で実行します。
- `no_terminate` は経験則による判定です。IRQ を長く止めて動くソフトでは、誤って失敗することがあります。