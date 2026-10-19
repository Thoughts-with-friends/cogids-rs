# CogiDS-rs 詳細設計書（機能別）

この文書群は、[CogiDS-rs 詳細設計書（概要）](./0_abstrsact.md) を機能ごとに掘り下げたものです。概要が「どこに何があるか」を示すのに対し、こちらは「各機能がどのデータを持ち、どの順番で何を呼び、なぜそうなっているか」を、実装を変更・レビューできる粒度で記述します。

- 対象読者：CogiDS-rs のソースを修正・拡張する開発者
- 対象範囲：`core/*`（エミュレータ本体）、`gui/common`・`gui/net`（GUI 共通部品）、`gui/egui`・`gui/freya`・`gui/tauri`（各 GUI）
- 前提知識：ARM アセンブリの基礎、ニンテンドーDS のハードウェア構成（GBATEK 程度）

---

## 目次

| 章  | ファイル                                               | 機能                                               | 主なクレート                                     |
| --- | ------------------------------------------------------ | -------------------------------------------------- | ------------------------------------------------ |
| 1   | [01_boot.md](01_boot.md)                               | 起動・ROM 読み込み・ダイレクトブート               | `ds_emu`                                         |
| 2   | [02_frame_scheduler.md](02_frame_scheduler.md)         | 1 フレームの実行とスケジューラ                     | `ds_emu`                                         |
| 3   | [03_cpu.md](03_cpu.md)                                 | CPU（ARM9/ARM7、CP15、インタプリタ）               | `ds_cpu`, `ds_emu`                               |
| 4   | [04_bios_firmware.md](04_bios_firmware.md)             | BIOS（HLE / 代替イメージ）とファームウェア         | `ds_bios`, `ds_emu`                              |
| 5   | [05_memory_bus.md](05_memory_bus.md)                   | メモリバスとアドレスマップ                         | `ds_emu`                                         |
| 6   | [06_interrupt_timer_ipc.md](06_interrupt_timer_ipc.md) | 割り込み・タイマ・IPC・除算器/平方根器             | `ds_interrupt`, `ds_peripherals`, `ds_emu`       |
| 7   | [07_dma.md](07_dma.md)                                 | DMA                                                | `ds_emu`                                         |
| 8   | [08_video_2d.md](08_video_2d.md)                       | 2D 映像（VRAM・2D エンジン・画面出力・キャプチャ） | `ds_gpu`, `ds_emu`                               |
| 9   | [09_video_3d_geometry.md](09_video_3d_geometry.md)     | 3D ジオメトリエンジンと GX FIFO                    | `ds_gpu`, `ds_emu`                               |
| 10  | [10_video_3d_raster.md](10_video_3d_raster.md)         | 3D ソフトウェアラスタライザ                        | `ds_gpu`                                         |
| 11  | [11_hires.md](11_hires.md)                             | 高解像度出力と xBRZ                                | `ds_gpu`                                         |
| 12  | [12_sound.md](12_sound.md)                             | 音声（SPU・ホスト出力・テンポ変換）                | `ds_spu`, `ds_emu`, `ds_common`                  |
| 13  | [13_cartridge_save.md](13_cartridge_save.md)           | ゲームカード・暗号・セーブメモリ                   | `ds_cartridge`, `ds_emu`                         |
| 14  | [14_spi_rtc_input.md](14_spi_rtc_input.md)             | SPI 機器・RTC・キー入力・タッチ                    | `ds_peripherals`, `ds_emu`                       |
| 15  | [15_wifi_multiplayer.md](15_wifi_multiplayer.md)       | Wi-Fi コントローラと通信対戦                       | `ds_peripherals`, `ds_net`, `ds_net_gui`         |
| 16  | [16_cheats.md](16_cheats.md)                           | チート（Action Replay）                            | `ds_emu`, `ds_common`                            |
| 17  | [17_frontend_common.md](17_frontend_common.md)         | フロントエンド共通部品                             | `ds_common`                                      |
| 18  | [18_gui.md](18_gui.md)                                 | 各 GUI（egui / freya / Tauri）                     | `lunaris_ds_egui`, `lunaris_ds_freya`, `lunaris` |
| 19  | [19_testing.md](19_testing.md)                         | テストと検証手順                                   | 全体                                             |
| 20  | [20_rom_tests.md](20_rom_tests.md) | 手元の ROM による機能別の実行テスト | `ds_rom_test`（`tools/rom_test`） |

---

## 各章の構成

各章は、原則として次の見出しで構成します。該当する内容がない見出しは省略します。

1. **責務** — その機能が何を受け持ち、何を受け持たないか
2. **構成ファイル・主要型** — ソースの場所と中心になる型
3. **データ構造** — フィールド、型、意味、初期値
4. **インタフェース** — 公開関数。呼び出し元 → 処理 → 呼び出し先
5. **処理詳細** — 番号付きの手順とテキスト図
6. **レジスタ・アドレス・定数** — 実機の I/O アドレスや、コード中の定数
7. **異常系** — エラーの返し方、範囲外アクセスの扱い
8. **設計判断と理由** — 実際に起きた不具合と、現在の実装になった理由
9. **制約・未実装**
10. **関連テスト**
11. **ROM 実行テスト** — 自分の ROM でその機能を確かめるときの機能 ID・チェック・設定例（[20 章](20_rom_tests.md)）

「8. 設計判断と理由」は、変更時に同じ不具合を再発させないための記録です。実装を変える前に必ず確認してください。

---

## 表記規約

- コードの参照は「ファイルパス + 関数名」で書きます（例：`core/system/src/emulator/runner.rs` の `Emulator::run`）。行番号は変更で変わるため使いません。
- 識別子・レジスタ名・エラー文字列は、ソースのとおりに書きます。
- 数値は、アドレスとレジスタ値を 16 進（`0x04000130`）、サイクル数などを 10 進で書きます。
- 「バスサイクル」は ARM7 のクロック（33.513982 MHz）単位です。ARM9 のタイムスタンプは 2 倍のクロック（約 67 MHz）単位で数えます。
- 図の矢印 `->` は呼び出しまたはデータの流れを表します。`from:` は呼び出し元、`to:` は次に処理を引き継ぐ先です（ソースの `///` コメントと同じ書式）。

---

## 用語集

| 用語                 | 意味                                                                                          |
| -------------------- | --------------------------------------------------------------------------------------------- |
| ARM9 / ARM7          | DS の 2 つの CPU。ARM9（ARM946E-S）がゲーム本体、ARM7（ARM7TDMI）が音・無線・タッチなどを担当 |
| キャッチアップ方式   | スケジューラが区切りの時刻を決め、各 CPU と周辺機器をその時刻まで進める方式（2 章）           |
| スライス             | 区切りの時間。最大 64 バスサイクル                                                            |
| HLE BIOS             | `SWI` を CPU 上の BIOS コードで実行せず、Rust で直接処理する方式（4 章）                      |
| ダイレクトブート     | BIOS とファームウェアの起動処理を実行せず、起動直後の状態を直接作る方式（1 章）               |
| GXFIFO / GXPIPE      | 3D ジオメトリコマンドのキュー（256 段）と、その前段のパイプ（4 段）（9 章）                   |
| バンド               | 3D ラスタライザがフレームを横に分割した帯。スレッドごとに並列で描く（10 章）                  |
| final pass           | 3D ラスタのエッジマーキング・フォグ・アンチエイリアス処理（10 章）                            |
| HdFrame              | 高解像度出力に必要な情報を 1 フレーム分まとめた、所有データだけの構造体（11 章）              |
| WSOLA                | 波形の似た位置で重ね合わせて、音程を変えずにテンポを変える方式（12 章）                       |
| MP                   | 無線通信のマルチプレイ（ホスト CMD → 子機 reply → ホスト ACK の繰り返し）（15 章）            |
| インスタンスフォルダ | `instances/instanceN/`。プロセスごとのセーブ・設定・ログの置き場所（17 章）                   |

---

## 参照資料

- GBATEK（ニンテンドーDS ハードウェア仕様）：各章で「GBATEK "DS Video"」のように節名で参照します。
- melonDS：3D ラスタライザ、Wi-Fi、チート、DMA・GXFIFO の割り込み条件などの移植元です。各章で `GPU3D_Soft.cpp`、`Wifi.cpp`、`AREngine.cpp` のように元のファイル名と関数名を併記します。
- CorgiDS：エミュレータ全体の骨格の移植元です。ソース先頭の `//! emulator.hpp` などのコメントは、CorgiDS の対応ファイルを表します。
- 各ソースファイルの先頭コメント（`//!`）と関数コメント（`///`）：この文書の元情報です。食い違いがあれば、コードを正とします。

---

## 手元の ROM での実行テスト

各章の「ROM 実行テスト」の節に、その機能を自分の ROM で確かめる方法があります。ROM のパスとケースは `docs/cogids-rs/local/roms.toml` に書きます。`docs/cogids-rs/local/` は `.gitignore` で除外しているので、ROM の名前・パス・結果の画像は Git に残りません。ひな形は [roms.example.toml](roms.example.toml) です。

ROM は、設定ファイルに書くか、引数で 1 本指定します。

```text
cp docs/cogids-rs/roms.example.toml docs/cogids-rs/local/roms.toml   # 自分の ROM を書く
cargo run --release -p ds_rom_test -- --list
cargo run --release -p ds_rom_test -- --feature raster3d,hires
cargo run --release -p ds_rom_test -- --rom ROM のパス --feature boot --expect no_terminate
# 結果：docs/cogids-rs/local/results/latest.md
```

VS Code では、コマンドパレットの **Tasks: Run Task** → `rom_test: …` からも実行できます（[20 章](20_rom_tests.md) 20.3）。

