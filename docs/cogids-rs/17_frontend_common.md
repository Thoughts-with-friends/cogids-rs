# 17. フロントエンド共通部品（`ds_common`）

## 17.1 責務

3 つの GUI（egui / freya / Tauri）が共有する、ウィンドウ以外の部品をまとめます。

- インスタンスフォルダ（プロセスごとのセーブ・設定・ログ）と `settings.json`
- `config.json`（GUI の状態）と、ゲームの開始・停止・メニュー操作（`Frontend`）
- エミュレータ専用スレッドと HD ワーカー（`EmuThread`。freya / Tauri が使う）
- メニューの定義（melonDS 風。3 つの GUI で同じ木）
- 画面の配置（回転・レイアウト・間隔・入れ替え・整数倍）とタッチ座標の変換
- 音声出力、倍速とテンポ変換（[12 章](12_sound.md)）、ゲームパッド、チート一覧（[16 章](16_cheats.md)）、ROM 情報、ログ

`gui/common/src/lib.rs` の先頭コメントは「egui と Tauri の共通部品」となっていますが、freya も使います。

## 17.2 構成ファイル

| モジュール | 主な型・関数 |
|---|---|
| `instance.rs` | `Instance`（`open`、`open_in`、`number`、`root`、`saves_dir`、`save_path`、`import_save`、`import_legacy_save`）、`Settings`（`load`、`ensure_mac`、`mac`、`apply`）、`format_mac`、`parse_mac`、`reveal`、`launch_instance` |
| `frontend.rs` | `GuiConfig`、`Frontend`（`start`、`stop`、`apply`、`apply_view`、`load_cheats`、`set_config`）、`lock`、`file_name`、`rom_paths` |
| `emu_thread.rs` | `Shared`、`EmuThread`、`key`、`publish_frame`、`hd_worker`、`emulate` |
| `menu.rs` | `Action`、`Window`、`Item`、`Menu`、`MenuState`、`menu_bar`、`remember`、`reason`、`RECENT_LIMIT` |
| `view.rs` | `View`、`ScreenLayout`、`ScreenSizing`、`Rect`、`ScreenRect`、`Placement`、`place`、`ROTATIONS`、`SCREEN_GAPS`、`MAX_RENDER_SCALE` |
| `gamepad.rs` | `Gamepads`（`poll`、`apply`、`key_bits`、`take_speed_presses`、`names`） |
| `cheats.rs` | [16 章](16_cheats.md) |
| `rom_info.rs` | `RomInfo`、`ICON_SIZE` |
| `audio.rs`, `speed.rs` | [12 章](12_sound.md) |
| `log.rs` | `setup_logging`、`setup_logging_in` |
| `lib.rs` | `DS_FPS` = 59.826098 |

## 17.3 インスタンスフォルダ（`instance.rs`）

```text
instances/instance1/
├── cheats/         ゲームごとのチート一覧（ROM 名.json）
├── logs/           lunaris.log（ローテーション、4 ファイル保持）
├── saves/          ROM 名.sav
├── states/         （予約。セーブステート用）
├── config.json     GUI の状態（ROM フォルダ、選択中の ROM、表示設定、最近の ROM など）
├── settings.json   エミュレータの設定（ログレベル、BIOS/ファーム、MAC など）
└── instance.lock   プロセスが使っている間ロックする
```

`instance.rs` の図では `cheats/` が「予約」となっていますが、実際にはチート一覧の置き場所です。

- `Instance::open()`：作業フォルダの下の `instances/instance1` から `instance8`（`MAX_INSTANCES` = 8）のうち、他のプロセスが `instance.lock` を持っていない最初のものを開きます（なければ作る）。全部ロックできなければ `instance1` をロックなしで使います。
- 同じ機械で 2 つ起動しても（通信対戦の両端など）、セーブ・設定・Wi-Fi の MAC が別になります。
- Tauri は作業フォルダではなく実行ファイルの隣を使います（`tauri dev` の作業フォルダ `src-tauri` に書くと、ファイル監視が再ビルドを起こすため）。
- `import_legacy_save(rom)`：以前の版が ROM の隣に置いた `.sav` を、そのゲームを初めて起動したときに `saves/` へコピーします（元のファイルはそのまま）。
- `import_save(rom, source)`：File > Import savefile。古いセーブを `.sav.bak` に残してから置き換えます。ゲームを止めてから呼ぶこと（動いているゲームが自分のセーブで上書きするため）。
- `launch_instance()`：System > Multiplayer > Launch new instance。自分自身をもう一度起動します（次のインスタンスフォルダを使う）。

### 17.3.1 `Settings`（`settings.json`）

| 項目 | 内容 |
|---|---|
| ログレベル | `error` / `warn` / `info` / `debug` / `trace` |
| `hle_bios` | HLE BIOS |
| BIOS / ファームのパス | 任意のダンプ |
| `frameskip` | 描画を省くフレーム数 |
| `mac_address` | `00:09:BF:xx:xx:xx`。インスタンスごとに 1 回生成（`ensure_mac`。OS の `RandomState`・時刻・プロセス ID で乱数） |

`Settings::apply(emu)` は `load_rom` より前に `Config` へ反映します（[1 章](01_boot.md)）。設定の変更は、次にゲームを開始・リセットしたときに効きます。
## 17.4 `Frontend`（freya / Tauri）

```text
 GUI のメニュー --Action--> Frontend::apply()   開始 / 停止 / 一時停止 / 表示 / 速度 / チート
               GUI 自身：ROM を開く、セーブの取り込み、終了、画面サイズ、ダイアログ
 Frontend::start(rom) --> Emulator + EmuThread
```

`Frontend` はインスタンス、`Settings`、`GuiConfig`、`AudioOutput`、`Shared`、`EmuThread`、実行中の ROM を持ちます（それぞれ `Mutex`。`lock()` は poisoned になったロックからも回復します）。

`start(rom)`：

1. `stop()`（前のゲームのセーブを先に書き出す）
2. `Emulator::new()`、`Settings::apply`、`config.save_dir` に `saves/` を設定、`import_legacy_save`
3. `load_rom`（失敗したら「Cannot load ファイル名: 理由」のエラー文字列）
4. `EmuThread::start(emu, shared, audio)`
5. `load_cheats()`、最近の ROM に追加、`config.json` を保存

`apply(action)` の分担：

| Action | 処理 |
|---|---|
| `OpenRomPath(path)` / `Reset` | `start` |
| `Eject` / `Stop` | `stop` |
| `TogglePause` / `FrameStep` | `Shared` の一時停止 / コマ送り |
| `OpenDirectory` / `LaunchInstance` | `reveal` / `launch_instance` |
| 表示・速度・解像度・フレームレート制限・最近の ROM の消去 | `GuiConfig.view` などを変え、`apply_view`（速度と解像度をスレッドへ）、保存 |
| `ToggleCheats` | `view.cheats_enabled` を反転、保存、`load_cheats` |
| `OpenRom` / `ImportSavefile` / `Quit` / `Open(window)` | GUI 側が処理（ダイアログやウィンドウが必要） |

`GuiConfig`（`config.json`）：音量（0〜100）、フレームレート制限、`View`（フラット化。古い `scale` / `layout` / `show_fps` キーも読める）、ROM フォルダ、選択中の ROM、最近の ROM。freya / Tauri は camelCase、egui は snake_case で書き、互いの形式も読めます。

## 17.5 エミュレータスレッド（`emu_thread.rs`）

### 17.5.1 `Shared`（UI とスレッドの共有状態）

| フィールド / メソッド | 内容 |
|---|---|
| `keys`（`set_keys`） | `key` のビット（A=0、B=1、SELECT=2、START=3、右=4、左=5、上=6、下=7、R=8、L=9、X=10、Y=11） |
| タッチ（`set_touch`、`touch_pos`） | `Some((x, y))` で押す（動かす）、`None` で離す。離しても最低 `TOUCH_MIN_FRAMES` = 4 フレームは押したまま |
| `frame`（`frame_since(last_seq)`） | 上画面・下画面の RGBA。`last_seq` より新しければ `(seq, width, rgba)` |
| `frame_taken` | フロントエンドが最新フレームを取った（次を作る価値がある） |
| `paused`, `step`（`frame_step`） | 一時停止、コマ送り（一時停止して 1 フレームだけ） |
| `unlimited` | フレームレート制限なし |
| `speed`（`set_speed`） | 速度（%） |
| `render`（`set_render(scale, xbrz)`） | bit 0-7 = 内部解像度（0 = 1 倍）、bit 8 = xBRZ |
| `cheats`（`set_cheats`） | 次のフレームで渡すコード |
| `fps()` | 直近 1 秒のエミュレートしたフレーム数 |
| `frame_seq` | 作ったフレームの通し番号 |

### 17.5.2 スレッド構成

```text
 エミュレータスレッド（emulate）              HD ワーカースレッド（hd_worker、"hd-render"）
 +---------------------------+
 | 一時停止中：10 ms 待つ     |
 | 速度・解像度・チートを反映 |
 | キー・タッチを反映         |
 | emu.run()                 |
 | 音声 -> TimeStretch -> 出力 |
 | 等倍：両画面 -> RGBA -> 共有 |
 | 2 倍以上：hd_frame()      |--try_send--> HdFrame::screen() x2
 |   （約 0.1 ms のコピー）   | （待ち合わせ     xBRZ・高解像度 3D
 | 次のフレーム時刻まで待つ   |   チャネル）     -> RGBA -> Shared::frame
 +---------------------------+
```

`EmuThread::start(emu, shared, audio)` がスコープ付きスレッドで両方を起動し、`stop()` で止めます（最後の `flush_save` とエミュレータの `Drop` でセーブが書かれます）。
### 17.5.3 1 フレームの処理（`emulate`）

1. 一時停止中でコマ送りの要求がなければ 10 ms 眠り、時刻の基準と速度計をリセットして次へ
2. 速度が変わったら `SpeedMeter::reset`、`TimeStretch::reset`
3. `set_render_scale`、チート（あれば）、キー、タッチを反映
4. `emu.run()`、`take_audio_samples`
5. `meter.tick(1)`。制限なしでも一時停止中でもなければ、1 倍はそのまま、それ以外はテンポ変換して `audio.push`
6. 次のどれかなら `publish_frame`：前のフレームが取られた、1 倍以下で制限あり、一時停止中（コマ送り）
   - 等倍：`screen_hd(upper, 1, false, ..)` で両画面 → RGBA → `store_frame`
   - 2 倍以上：`(hd_frame(), scale, xbrz)` を HD ワーカーへ。コマ送りは `send`（待ってでも渡す）、それ以外は `try_send`（ワーカーが忙しければこのフレームは表示しない）
7. `frame_seq` を増やし、1 秒ごとに fps を更新
8. 次の時刻（`next += 1 / (DS_FPS × 速度)`）まで眠る。制限なしなら待たない。4 フレーム分以上遅れたら、取り戻そうとせず基準をリセット

終了時：`audio.clear()`、`flush_save()`。

### 17.5.4 設計判断

- **チャネルは容量 0（待ち合わせ）。** ワーカーが空いているときだけジョブが渡ります。忙しければそのフレームを表示しないだけで、エミュレーションと音声は遅れません。
- **1 倍より速いときは、取りに来ていないフレームを作らない。** 見られないフレームを拡大しても無駄なためです。
- **大きく遅れたら基準をリセットする。** 取り戻そうとすると、遅れの後に早送りのような動きになります。

## 17.6 メニュー（`menu.rs`）

```text
 MenuState（何が読み込まれているか、一時停止か、何を選んだか）
   |  menu_bar()
   v
 Vec<Menu>  -- egui：menu_button / checkbox / radio で描く
            -- freya：Menu / SubMenu / MenuButton で描く
            -- Tauri：get_menu で返し、React が描く
   |  クリック
   v
 Action     -- egui：App::apply、freya：actions::run、Tauri：menu_action コマンド
```

- 構成は melonDS（`frontend/qt_sdl/Window.cpp`）と同じ File / System / View / Config / Help です。
- melonDS にあってこのエミュレータで実現できない項目は**無効のまま残し**、理由（`reason`）をホバーで表示します。例：`SAVESTATE`、`GBA_SLOT`、`DSI`、`FIRMWARE_BOOT`、`XBRZ_SCALE`（xBRZ は内部解像度 2 倍以上が必要）、`DATE_TIME`（DS の時計は PC の日時に従う）。
- `Item`：クリックできる項目（`checked` でチェックボックス、`radio` でラジオボタン、無効なら理由、ホバーの補足）、区切り、サブメニュー、文字だけの行（例：「DS slot: ゲーム名」）。
- `Window`（ダイアログ）：`RomInfo`、`Multiplayer`、`EmuSettings`、`Input`、`Audio`、`Wifi`、`Paths`、`About`、`Cheats`。
- ショートカット：Ctrl+O（ROM を開く）、Ctrl+P（一時停止）、Ctrl+R（リセット）、Ctrl+N（コマ送り）、Ctrl+Q（終了）。Ctrl なしの文字キーは DS のボタンに使います。
- 最近の ROM は `RECENT_LIMIT` = 10 件（`remember` が重複を除いて先頭に追加）。
- Config > Emulation speed（`SPEEDS` のラジオ）、Config > Video settings（内部解像度 1〜4 倍、xBRZ）、System > Enable cheats、System > Setup cheat codes。
## 17.7 画面の配置（`view.rs`）

```text
 View（config.json）            place(view, area_w, area_h)       GUI
 回転 / レイアウト / 間隔  -->  Placement { screens[上, 下]:  -->  各画面をその矩形に描く。
 入れ替え / サイズ / 整数倍       矩形 + 回転 }                     to_ds() でクリック位置を
                                                                  DS のタッチ座標に戻す
```

| `View` の項目 | 内容 |
|---|---|
| `scale` | View > Screen size の倍率（ウィンドウの大きさ） |
| `rotation` | 0 / 90 / 180 / 270（`ROTATIONS`、時計回り） |
| `layout` | `ScreenLayout`：`Natural`（回転した DS と同じ）/ `Vertical`（既定）/ `Horizontal` |
| `gap` | 画面の間隔（`SCREEN_GAPS` = 0、1、8、64、90、128 DS 画素） |
| `swap` | 上下の入れ替え |
| `sizing` | `ScreenSizing`：`Even`（既定）/ `EmphasizeTop`（上を 2 倍）/ `EmphasizeBottom` / `TopOnly` / `BottomOnly`（`weights()` が 0 の画面は隠す） |
| `integer_scaling` | 整数倍だけ（くっきりした画素） |
| `filtering` | バイリニア補間 |
| `show_fps` | fps 表示 |
| `speed` | 速度（%） |
| `render_scale` | 内部解像度 1〜4（`MAX_RENDER_SCALE`） |
| `xbrz` | xBRZ（`render_scale` が 2 以上のとき） |
| `cheats_enabled` | 有効なチートを実行 |

レイアウトは回転の後に決めます（melonDS と同じ）。各画面を個別に回転してから、縦に積むか横に並べます。`Natural` は 0° / 180° で縦、90° / 270° で横で、上画面は本体の上画面が来る側です。`ScreenRect::to_ds(px, py)` は、回転・拡大した画面上の点を DS の画素（0..256、0..192）に戻し、画面外なら `None` です。

## 17.8 ゲームパッド（`gamepad.rs`）

`gilrs` で自動検出します（抜き差し対応）。`poll()` でイベントを処理し、`apply(emu)`（egui）または `key_bits()`（他の GUI）でキーボードの入力に OR します。

- 顔のボタンは位置で対応：右 = A、下 = B、上 = X、左 = Y（DS と同じ配置）
- L / R：ショルダーとトリガーのどちらでも
- 左スティック：十字キーとしても使う（倒し量 0.5 以上、`STICK_THRESHOLD`）
- 左スティック押し込み（L3）：速度を次へ（`take_speed_presses` → `Action::CycleSpeed`）

## 17.9 ROM 情報（`rom_info.rs`）

System > ROM info で表示します。ヘッダ（`0x000` タイトル、`0x00C` ゲームコード、`0x010` メーカー、`0x01E` 版、`0x020` / `0x030` ARM9 / ARM7 のオフセット・エントリ・RAM アドレス・サイズ、`0x068` バナーのオフセット）と、バナー（`0x020` アイコン 32×32 の 4bpp 8×8 タイル、`0x220` パレット BGR555、`0x240 + n × 0x100` タイトル UTF-16。0 = 日本語、1 = 英語）を読みます。アイコンは RGBA に変換します（色 0 は透明）。

## 17.10 ログ（`log.rs`）

`setup_logging_in(dir)` で、`instances/instanceN/logs` に書く独自のロガーを入れます。`tracing` はエミュレータの遅延が大きくなるため使いません。ロガーはプロセス全体で 1 つなので、1 プロセスに複数のインスタンスがあるときは最初に呼んだものが勝ちます（複数スレッドから 1 ファイルに混ざって書かれるよりよいため）。

## 17.11 関連テスト

- `emu_thread.rs` の `#[ignore]` テスト（実 ROM が必要。環境変数 `LUNARIS_TEST_ROM`）：`thread_produces_frames`（約 60 fps でフレームが出る）、`pause_and_step`（一時停止でフレームが止まり、コマ送りでちょうど 1 枚）、`speed_and_resolution`（2 倍速でフレームレートが 2 倍、2 倍解像度でフレームが 2 倍の大きさ）
- `frontend.rs`：egui の snake_case の `config.json` を読めること
- `audio.rs`、`speed.rs`、`cheats.rs` のユニットテスト（[12 章](12_sound.md)、[16 章](16_cheats.md)）

## 17.12 ROM 実行テスト

このツールは GUI を通さずにエミュレータ本体を動かすので、この章の部品は対象外です。エミュレータスレッドは `ds_common` の `#[ignore]` テスト（17.11）で確かめます。（[20 章](20_rom_tests.md)）
