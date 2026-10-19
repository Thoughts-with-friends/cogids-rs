# 18. 各 GUI（egui / freya / Tauri）

## 18.1 責務と比較

3 つの GUI は、同じメニュー（`ds_common::menu`）、同じ画面配置（`ds_common::view`）、同じ設定ファイル（インスタンスフォルダの `config.json` / `settings.json`）を使います。違いは、エミュレーションを動かす場所と、画面の描き方です。

| | egui | freya | Tauri |
|---|---|---|---|
| クレート | `lunaris_ds_egui`（`gui/egui`） | `lunaris_ds_freya`（`gui/freya`） | `lunaris`（`gui/tauri/src-tauri`）＋ React（`gui/tauri/src`） |
| 位置付け | ワークスペースの既定ビルド対象 | 枠なし・半透明ウィンドウ | Web 技術の画面 |
| エミュレーションの実行場所 | UI スレッド（`run_frames`） | `ds_common::EmuThread` | `ds_common::EmuThread` |
| 状態の持ち主 | `App`（自前） | `ds_common::Frontend` | `ds_common::Frontend`（`AppState`） |
| 画面の受け渡し | 等倍：直接取得。2 倍以上：`hd.rs` の HD ワーカー | `Shared::frame_since` で RGBA | コマンド `get_frame` で RGBA をバイナリのまま |
| 入力 | egui のキーボード、`Gamepads::apply` | freya のキーイベント、`gilrs` を 8 ms ごとに | ブラウザのキーイベント、Web Gamepad API |
| 通信対戦 | あり（`multiplayer.rs`） | なし（メニューは理由付きで無効） | なし（同上） |
| チート画面 | `cheat_dialog.rs` | `dialogs/cheats.rs` | `CheatDialog` |

## 18.2 egui（`gui/egui`）

### 18.2.1 構成ファイル

| ファイル | 内容 |
|---|---|
| `main.rs` | `App`（`update`、`apply`、`start`、`import_savefile`、`run_frames`、`draw_screens`、`ui_welcome`、`load_cheats`、`speed`）、`paint_screen`、`swap_red_blue` |
| `hd.rs` | `HdWorker`、`HdImage`（2 倍以上の拡大を別スレッドで） |
| `menu.rs` | `bar(ui, menus) -> Option<Action>`（共通メニューを egui で描く） |
| `dialogs.rs` | `Dialogs`、`Parts`（ROM 情報、設定、通信対戦、About、チート） |
| `cheat_dialog.rs` | `CheatEditor`、`Outcome` |
| `multiplayer.rs` | `Multiplayer`（LAN のホスト / 参加、準備ランプ、`STALL_BUDGET` = 100 ms） |
| `input.rs` | `apply_keyboard`、`Touch`（`apply`、`update`、`TOUCH_MIN_FRAMES` = 4） |
| `config.rs` | `Config`（`config.json`、snake_case）、`set_config_path`（作業フォルダの古い `config.json` を移行） |
| `font.rs` | `setup_custom_fonts` |

### 18.2.2 1 回の描画（`App::update`）

```text
update()
  1. Ctrl+文字のショートカット            -> Action
  2. multiplayer.tick                    LAN セッション、準備ランプ、リンクの付け直し
  3. メニューバー（menu.rs）              -> Action
  4. apply(Action)                       開く / リセット / 一時停止 / 表示 / ダイアログ
  5. 画面：借りのフレームを実行（一時停止中は 0、コマ送りは 1）
     ds_common::view::place の位置に両画面を描き、下画面の ScreenRect でタッチ
  6. dialogs.rs のウィンドウ
```

### 18.2.3 フレームの実行（`run_frames`）

| 状態 | 実行するフレーム数 |
|---|---|
| 一時停止中 | コマ送りなら 1、それ以外 0（借りと速度計をリセット） |
| LAN の参加処理中（`multiplayer.busy()`） | 0（LAN バックエンドはワーカーが使っている） |
| フレームレート制限あり | 借り `frame_debt += 経過秒 × fps(速度)` の整数部。借りが 1 倍で約 4 フレーム分を超えたら（ウィンドウのドラッグなどで止まった）4 分の 1 に減らす |
| 制限なし | 上限なし |

各フレーム：キーボード → ゲームパッド → タッチを反映し、`emu.run()`、音声（1 倍はそのまま、それ以外はテンポ変換して `push`。制限なしでは出さない）。制限なし、または 1 倍より速いとき、1 回の描画で `UNLIMITED_BUDGET`（15 ms）を超えたら借りを捨てて打ち切ります（PC が追いつかないときにウィンドウの応答を保つ）。最後に `meter.tick(ran)`。毎回 `set_render_scale(view.render_scale())` を呼びます。LAN セッション中は速度を 1 倍に固定します（`speed()`）。

### 18.2.4 画面の描画（`draw_screens`）

- 等倍：`get_upper_frame` / `get_lower_frame` を取り、`swap_red_blue`（0xAARRGGBB → egui の RGBA バイト順。アルファは 0xFF に固定。電源が切れたエンジンはゼロ＝透明の画素を残すため）でテクスチャへ。
- 2 倍以上：新しいフレームがあれば `HdWorker` に `hd_frame()` を渡し、ワーカーが完成した画像を次の描画で受け取ってテクスチャへ。ワーカーは完成時に `request_repaint()` を呼びます。
- `paint_screen` が、`ScreenRect` の矩形に時計回りの回転をつけて描きます。

### 18.2.5 その他

- File > Import savefile：止める → 古いセーブを `.sav.bak` に残す → コピー → 再開始。
- `App::start`：今のゲームを止めてセーブを書き出してから開始します。
- 状態バーに最後のエラーを表示します（閉じるまで残る）。
## 18.3 freya（`gui/freya`）

### 18.3.1 構成

```text
 main：Instance + Settings + ログ -> Frontend（ds_common）-> 起動
 App（ルート）
   ├─ Titlebar   ロゴ、MenuBar（ds_common::menu）、ドラッグ領域、Appearance、最小化 / 最大化 / 閉じる
   ├─ Screens    エミュレータスレッドのフレーム、タッチ   （ゲームがなければ Welcome）
   ├─ StatusBar  状態、ゲーム、fps、速度、解像度、インスタンス
   └─ Dialogs    モーダルのカード（ROM 情報、設定、チートなど）
 input.rs：キーボード + ゲームパッド -> Shared::set_keys、状態の更新
 閉じる（✕、File > Quit、Alt+F4）-> Frontend::stop（セーブを書く）
```

| ファイル | 内容 |
|---|---|
| `main.rs` | ルートコンポーネント、`WINDOW_RADIUS` = 14 |
| `state.rs` | `Ctx`（全コンポーネントが共有：config、menus、status、dialog、error、look など）、`Look`（`freya.json`：ウィンドウの不透明度 0.3〜1.0、アクセント）、`refresh`、`refresh_status`、`update_config`、`update_look` |
| `actions.rs` | `run(ctx, action)`：ウィンドウが必要なもの（ファイル選択、終了、ウィンドウサイズ、ダイアログ）はここ、残りは `Frontend::apply`。`TITLEBAR_HEIGHT` = 44、`SCREEN_PADDING` = 12 |
| `screens.rs` | フレームのポンプ（4 ms ごとに `frame_since`）、`decode`、`rotate_rgba`、`screen_view` |
| `menubar.rs` | 共通メニューを freya の `Menu` / `SubMenu` / `MenuButton` で描く。無効項目は灰色で理由をツールチップに |
| `titlebar.rs` | 枠なしウィンドウのタイトルバー、Appearance のポップオーバー |
| `panels.rs` | Welcome 画面（ROM カード最大 12 枚、バナーのアイコンとタイトル）、状態バー |
| `input.rs` | キー → DS ボタン、Ctrl ショートカット（物理キーで判定）、`gilrs` のポーリング、状態の更新（約 0.5 秒ごと） |
| `dialogs/` | `mod.rs`（モーダル、`section`、`field`、`note`）、`views.rs`（ROM 情報・各設定・About・通信対戦）、`cheats.rs` |
| `theme.rs` | 暗い半透明の「ガラス」調のテーマ、`ACCENT` など |

### 18.3.2 設計判断

- **60 Hz のフレームは `Ctx` に入れない。** 画面コンポーネントだけが自分の状態として持ちます。新しいフレームでメニューや開いているダイアログを再描画しないためです。
- **フレームの回転は自前（`rotate_rgba`）。** freya の `rotate` は画像の画素を回さないためです。
- **ダイアログが開いている間は DS のキーを無視する。** チート名の入力でボタンが押されないようにします。
- **Ctrl ショートカットは物理キーで判定する。** キーボード配列や、Ctrl で入力文字が変わることに影響されません。

## 18.4 Tauri（`gui/tauri`）

### 18.4.1 バックエンド（`src-tauri`）

`runner.rs`：実行ファイルの隣のインスタンスフォルダを開き（`Instance::open_in`）、`Settings::load` と `ensure_mac`、ログを設定して、`AppState`（= `ds_common::Frontend`）を管理状態にします。ウィンドウを閉じるとき `stop()` でセーブを書きます。プラグイン：dialog、fs、opener。

コマンド（`commands.rs`）：

| コマンド | 内容 |
|---|---|
| `get_config` / `set_config` | `GuiConfig`（音量・フレーム制限・速度・解像度はすぐ反映） |
| `list_roms(dir)` | フォルダの `.nds` を名前順に |
| `start_emulator(path)` / `stop_emulator` / `is_running` | |
| `get_menu` / `menu_action(action)` | 共通メニュー。ダイアログやウィンドウが必要なものはフロントエンドが処理 |
| `import_savefile(path)` | 止める → バックアップ → コピー → 再開始 |
| `rom_info` | 実行中のゲームの ROM 情報 |
| `get_settings` / `set_settings` | `settings.json`（次の開始・リセットで反映） |
| `layout_screens(width, height)` | `ds_common::view::place`（CSS 画素） |
| `set_keys(keys)` / `set_touch(x, y)` | `emu_thread::key` のビット / 下画面の DS 画素（`null` で離す） |
| `get_frame(last_seq)` | 18.4.2 |
| `get_cheats` / `set_cheats` / `check_cheat_code(code)` | 有効なコードに誤りがあれば何も保存せずに失敗 / 入力中の検証 |
| `get_status` | 実行中のゲームのファイル名など |

### 18.4.2 `get_frame` のバイナリ形式

`tauri::ipc::Response` で、JSON を通さずに返します。新しいフレームがなければ空です。

| オフセット | 大きさ | 内容（リトルエンディアン） |
|---|---|---|
| 0 | 8 | フレーム番号 |
| 8 | 4 | 1 画面の幅 |
| 12 | 4 | 1 画面の高さ |
| 16 | 幅 × 高さ × 4 | 上画面 RGBA |
| 続き | 幅 × 高さ × 4 | 下画面 RGBA |

等倍では 256×192、内部解像度を上げるとその倍率の大きさになります。

### 18.4.3 フロントエンド（React、`gui/tauri/src`）

| 場所 | 内容 |
|---|---|
| `services/api/emulator.ts` | `invoke` のラッパー（`getConfig`、`start`、`setKeys`、`menu`、`menuAction`、`getCheats` など）と、`get_frame` の解読 |
| `components/organisms/EmulatorView` | `requestAnimationFrame` で `get_frame` を呼び、`putImageData` で 2 つの canvas に描く。キーイベント、`useGamepads`（Web Gamepad API）でボタンのビットをまとめて `set_keys`。タッチを `set_touch` |
| `components/organisms/DsMenuBar` | `get_menu` の木を描き、クリックを `menu_action` へ |
| `components/organisms/DsDialogs` | ROM 情報、設定などのダイアログ |
| `components/organisms/CheatDialog` | チートの編集（`cheatCode.ts` で入力を検証） |

`gui/tauri/src` には、元にしたテンプレートの部品（変換フォーム、エディタなど）も残っていますが、エミュレータの画面では使っていません。

## 18.5 チート画面（3 つの GUI で共通の配置）

`docs/design/cheat_gui.png` の配置：左にゲームの情報、中央にチート一覧（有効のチェック、名前、形式）、右に選んだチートの名前・メモ・コード。

```text
 開く     -> cheats/ROM 名.json + ROM 情報
 行を選ぶ -> その名前 / メモ / コードを編集欄へ（freya は、コードが正しければ編集中の内容を残す）
 追加 / 保存 / 削除 / チェック -> 作業用の一覧
 OK       -> ファイルに書き、エミュレータのコードを読み直す
 Cancel   -> 何も書かずに閉じる
```

## 18.6 関連テスト

- egui：`cheat_dialog.rs` のレイアウトのテスト（広い画面で数回ヘッドレスに配置）、`config.rs` の往復
- freya：`gui/freya/examples/min.rs` と、ヘッドレスの PNG スナップショット
- Tauri：React のテスト（`DsMenuBar.test.tsx`、`EmulatorView.test.tsx`、`CheatDialog.test.tsx`、`emulator.test.ts`）、実 ROM を使う Rust のテスト
- 3 つの GUI の `config.json` の往復テスト

## 18.7 ROM 実行テスト

GUI の操作はこのツールの対象外です。18.6 のテストと、配布用の実行ファイルでの手動確認を使います。（[20 章](20_rom_tests.md)）
