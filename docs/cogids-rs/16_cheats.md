# 16. チート（Action Replay）

## 16.1 責務

- ゲームごとのチート一覧の保存・読み込み（`cheats/<ROM 名>.json`）
- Action Replay DS のコード文字列の解析
- 毎フレームの最初に、有効なコードを実行する（melonDS `AREngine.cpp` の移植）

GUI の編集画面は [18 章](18_gui.md) を参照してください（レイアウトは `docs/design/cheat_gui.png`）。

## 16.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/system/src/emulator/cheats.rs` | `parse_ar_code`、`ArState`、`Flow`、`set_cheats`、`run_cheats`、`run_cheat`、`ar_write`、`ar_condition`、`ar_control`、`ar_data`、`ar_copy_in`、`ar_copy`、`data_op` |
| `gui/common/src/cheats.rs` | `Cheat`、`ACTION_REPLAY`、`cheats_path`、`load`、`save`、`enabled_codes` |

## 16.3 データの流れ

```text
 cheats/<rom 名>.json  <--load / save-->  Vec<Cheat>  （ダイアログが編集する）
                                              |  enabled_codes(cheats, on)
                                              v
                            Emulator::set_cheats(Vec<Vec<u32>>)
                                              |
 run() の最初（毎フレーム）--> run_cheats() --> コードごとに run_cheat()
```

| 場面 | 呼び出し |
|---|---|
| ゲーム開始 | GUI が `enabled_codes` → `set_cheats`（egui：`App::load_cheats`、他：`Frontend::load_cheats`） |
| ダイアログの OK | 一覧を保存してから、同上 |
| System > Enable cheats の切り替え | `View::cheats` を変えて、同上（オフなら空の一覧） |
| エミュレータスレッド | `Shared::set_cheats(codes)` に置き、次のフレームの前に `emu.set_cheats` |

## 16.4 データ構造

### 16.4.1 `Cheat`（`gui/common/src/cheats.rs`、JSON）

| フィールド | 内容 |
|---|---|
| `name` | 名前 |
| `memo` | メモ（使い方など。実行しない） |
| `code` | 入力したままのコード文字列（1 行に `XXXXXXXX YYYYYYYY`） |
| `enabled` | 有効 |
| `kind` | 形式（現在は常に `ACTION_REPLAY` = `"Action Replay"`） |

コードは文字列のまま保存し、エミュレータへ渡すときだけ解析します。往復で情報が失われません。

### 16.4.2 `ArState`（1 つのコードの実行状態。melonDS `RunCheat` の局所変数）

| フィールド | 内容 |
|---|---|
| `pc` | 次のワードの索引 |
| `offset` | オフセットレジスタ |
| `datareg` | データレジスタ |
| `cond`, `condstack` | 現在の IF の結果と、入れ子の IF の結果のスタック（ビット列） |
| `loopstart`, `loopcount`, `loopcond`, `loopcondstack` | C0 ループ |
| `c5count` | C5 のカウンタ |

## 16.5 インタフェース

| 関数 | 内容 |
|---|---|
| `parse_ar_code(text) -> Result<Vec<u32>, String>` | 8 桁 16 進のワード、1 行 2 個（空白の種類は問わない）。8 桁でない、または対にならないとエラー文字列 |
| `Cheat::words()` | `parse_ar_code(&self.code)` |
| `enabled_codes(cheats, on)` | `on` が偽なら空。有効で解析できたコードだけを返し、解析できないものはログに出して飛ばす |
| `cheats_path(instance, rom)` | `<インスタンス>/cheats/<ROM 名>.json` |
| `load(path)` / `save(path, cheats)` | ファイルがなければ空。保存時はフォルダを作る |
| `Emulator::set_cheats(codes)` | 毎フレーム実行するコードを置き換える |

## 16.6 処理詳細（`run_cheat`）

```text
run_cheat(code)
  s = ArState::new()、budget = 1_000_000（無限ループ対策）
  while s.pc < code.len()：
    a = code[pc]、b = code[pc + 1]（範囲外は 0）、pc += 2、op = a >> 24
    IF が偽の間は、ENDIF / NEXT 系（D0〜D2）と C5 以外を飛ばす
      （飛ばすのが Ex のブロックコピーなら、続くデータワードも飛ばす）
    op の上位ニブルで分岐：
      0x0〜0x2  ar_write        32 / 16 / 8 ビットの書き込み（a + offset）
      0x3〜0xA  ar_condition    3x〜6x：ワード比較、7x〜Ax：マスク付きハーフワード比較 → IF を開く
      0xB       offset = [a & 0x0FFFFFFF + offset]
      0xC       ar_control      C0 ループ、C4（未対応）、C5 カウンタ IF、C6 オフセットの保存
      0xD       ar_data         D0〜D2 ブロックの終わり（ENDIF / NEXT / 全終了）、
                                D3〜DC オフセットとデータレジスタの操作（D4 は data_op で演算）
      0xE       ar_copy_in      続く b バイト（コード内）を a + offset へ
      0xF       ar_copy         offset から a へ b バイト
    Flow::Stop なら終了
```

読み書きは **ARM7 のバス**（`arm7_read_*` / `arm7_write_*`）を使います（melonDS と同じ）。メイン RAM は共有なので、ARM9 のデータにも届きます。GBATEK "DS Cart Cheat Action Replay DS" に従います。

## 16.7 設計判断と理由

- **毎フレームの最初に実行する。** melonDS と同じタイミングです（`run()` の手順 1。[2 章](02_frame_scheduler.md)）。
- **実行回数の上限（100 万ワード）。** 誤ったループのコードでエミュレータが止まらないようにします。
- **コードは文字列で保存する。** ユーザーが入力したとおりに残し、メモと一緒に往復できるようにします。保存時にだけ、有効なコードが正しいかを確認します（Tauri の `set_cheats` は、有効なコードに誤りがあれば何も保存せずに失敗します）。

## 16.8 制約・未実装

- `C4`（コード自身の書き換え）は未対応です。
- Action Replay 以外の形式（CodeBreaker など）は扱いません。

## 16.9 関連テスト

- `core/system/src/emulator/cheats.rs`：`parses_pairs`（文字列の解析）、`data_ops`（D4 の演算）、`runs_codes_on_main_ram`（ARM7 のバス経由のメイン RAM で、書き込み、偽の IF、ループ、ブロックコピー）
- `gui/common/src/cheats.rs`：`only_enabled_valid_codes_run`、`round_trips_through_a_file`
