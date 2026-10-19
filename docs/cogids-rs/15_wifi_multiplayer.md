# 15. Wi-Fi コントローラと通信対戦

## 15.1 責務

| 層 | クレート | 受け持ち |
|---|---|---|
| Wi-Fi コントローラ | `ds_peripherals::wifi` | ARM7 の無線 I/O（`0x04800000..0x0480FFFF`）、ベースバンド/RF チップ、マイクロ秒タイマ、送信スロット、受信リングバッファ、MP（CMD / reply / ACK）の手順。melonDS `Wifi.cpp` の移植 |
| 通信媒体の抽象 | `ds_peripherals::wifi::MpLink` | フレームの送受信。melonDS の `Platform::MP_*` と 1 対 1 |
| 通信の実装 | `ds_net`（`core/net`） | `NetLink`：melonDS の MP 層（`melonds_net`、C ABI）へつなぐ。Local（同一プロセス）/ LAN（ENet） |
| GUI 部品 | `ds_net_gui`（`gui/net`） | LAN セッションの開始・参加・離脱、準備ランプ（UDP の ping/pong） |

## 15.2 構成ファイル・主要型

| ファイル | 内容 |
|---|---|
| `core/peripherals/src/wifi/mod.rs` | `WiFi`、`TxSlot`、`reset`、`set_link`、`run`、`us_timer`、`ms_timer`、`read`、`write`、電源と割り込み |
| `core/peripherals/src/wifi/tx.rs` | 送信：`fire_tx`、`start_tx_loc`、`start_tx_cmd`、`start_tx_beacon`、`process_tx`、`tx_send_frame`、`send_mp_reply`、`send_mp_ack` |
| `core/peripherals/src/wifi/rx.rs` | 受信：`check_rx`、`start_rx`、`rx_tick`、`finish_rx`、`rx_filter`、`mp_client_reply_rx` |
| `core/peripherals/src/wifi/link.rs` | `MpLink`、`MpRecv` |
| `core/peripherals/src/wifi/tests.rs` | テスト用の擬似リンク |
| `core/net/src/lib.rs`, `link.rs` | `init(backend)`、`backend()`、`process()`、`NetLink`、`lan` |
| `gui/net/src/session.rs` | `LanSession`、`SessionState`、`MAX_PLAYERS` |
| `gui/net/src/ready.rs` | `ReadyProbe`、`PeerState`、`Want`、`Readiness`、`PROBE_PORT` |
| `gui/net/src/light.rs` | `ReadyLight`、`host_part`、`resolve_host` |
| `core/system/src/emulator/mod.rs`, `interrupt.rs` | `set_mp_link`、`raise_wifi_irq` |

## 15.3 Wi-Fi コントローラ

### 15.3.1 アドレスとブロック

```text
 ARM7 の無線ライブラリ（WM / wl、ARM7 RAM で動く）
   |  16 ビットの読み書き
   v
 0x04800000-0x04800FFF  I/O ポート（0x04808000 にミラー）    -> io
 0x04804000-0x04805FFF  8 KB の Wi-Fi RAM（送受信バッファ）   -> ram
   +-- W_BB_CNT / W_BB_WRITE / W_BB_READ  -> ベースバンド（bb_regs）
   +-- W_RF_DATA1 / W_RF_DATA2           -> RF チップ（rf_regs、チャネル）
   +-- W_POWER_* / W_MODE_RST            -> トランシーバの電源状態
   +-- W_US_COUNT / W_US_COMPARE         -> マイクロ秒タイマ、IRQ 14/15
   +-- W_TXBUF_LOC1..3 / CMD / BEACON    -> 送信スロット（tx）
   +-- W_RXBUF_*                         -> 受信リングバッファ（rx）
   `-- W_IF / W_IE                       -> ARM7 の IRQ 24（Wi-Fi）
 フレーム <-> dyn MpLink（link）：他の本体（ds_net / melonDS MP）
```

### 15.3.2 主な状態

| 項目 | 内容 |
|---|---|
| I/O ポート、RAM | I/O ポート（ハーフワード）、8 KB RAM |
| ベースバンド / RF レジスタと RF チップ種別 | ファームの値で初期化（`reset`） |
| チャネル表と現在のチャネル | チャネル 1〜14 を選ぶ RF レジスタの値（ファームの Wi-Fi 較正から）。0 は未選局。同じチャネルの本体とだけフレームを交換する |
| TX スロット（LOC1、CMD、LOC2、LOC3、ビーコン、MP 返信） | `TxSlot`：TX ヘッダの位置、フレーム長、レート（1 = 1 Mbit/s、2 = 2 Mbit/s） |
| 電源 | POWCNT2 bit 1、W_POWER_US bit 0 で時計が動くか、立ち上がり中の残り µs |
| Wi-Fi の時計 | 電源投入からの µs（フレームの送信時刻） |
| MP ホスト | 次の子機の返信枠までの時間、まだ返信する子機、返信しなかった子機、返信の中身 |
| MP 子機 | 最後に受けた CMD のシーケンス番号（重複を捨てる）、ホストに関連付け済みか、次に待つ時刻 |
| リンク | 通信相手（`Option<Box<dyn MpLink>>`）と、`begin()` 済みか |

### 15.3.3 時間の進め方

`WiFi::run(cycles)`：ARM7 のバスサイクルを受け取り、コントローラに電源が入っている間（POWCNT2 bit 1 が 1 かつ W_POWER_US bit 0 が 0）、`US_STEP` = 8 µs 単位で `us_timer()` を進めます。送信と受信も同じ単位で進みます（melonDS の `USTimer` イベントと同じ）。`W_US_COUNT` の 1024 µs ごとに `ms_timer()` を呼びます。

`run()`（[2 章](02_frame_scheduler.md)）は、スライスごとに `wifi.run(slice)` → `raise_wifi_irq()` を呼びます。`new_frame()` はフレームの最初に、リンクに通信処理の機会を与えます。

### 15.3.4 電源とリンク

`update_power_on()` は電源の立ち上がり / 立ち下がりで `link.begin()` / `link.end()` を呼びます（melonDS `MP_Begin` / `MP_End`）。リンクがなければフレームはどこにも行かず、ソフトからは「誰も近くにいない」本体に見えます。それでも `WM_Enable` / `WM_PowerOn` は成功します。

### 15.3.5 送信（`tx.rs`）

```text
 W_TXREQ_SET / W_TXBUF_LOCn / W_RXCNT bit 15 -> fire_tx()      スロットを選ぶ
 IRQ14 で W_TXBUF_BEACON が有効             -> start_tx_beacon()
 MP CMD を受信（rx.rs）                      -> send_mp_reply()
   v
 us_timer -> process_tx(slot)、段階ごとに：
   0  プリアンブル終了  IRQ7、フレームをリンクへ（tx_send_frame）
   1  送信終了          LOCn / ビーコン：IRQ1、スロット解放
                        CMD：子機の返信を集める（link.recv_replies）
   2  返信の窓          各子機の返信を、その子機の時間枠で受信
   3  ACK 送信済み      IRQ12、CMD スロット解放
   10/11                空の MP 返信（子機に送るものがなかった）
   13                   CMD のタイムアウト（CMD_COUNT が短すぎる）
```

優先順位（`next_tx_slot`）：MP 返信、ビーコン、LOC3、LOC2、CMD、LOC1。MP ホストの手順：プリアンブル、IRQ7、データ、（IRQ1）、子機の返信を待つ（`16 + (10 + CMD_REPLYTIME) × 子機数` µs）、IRQ7、ACK、IRQ12。MP フレームの宛先 MAC は `03:09:BF:00:00:00`（`MP_CMD_MAC`）などです。

### 15.3.6 受信（`rx.rs`）

```text
 us_timer（待機中、512 µs ごと）   -> check_rx(Any)    link.recv_packet（ブロックしない）
 us_timer（MP 子機、同期時刻）     -> check_rx(Host)   link.recv_host_packet（タイムアウトまで待つ）
 process_tx の段階 2（MP ホスト）  -> mp_client_reply_rx(aid)
   v
 start_rx()      IRQ6。フレームを W_RXBUF に 1 ハーフワードずつ書き始める
 rx_tick() × N   4 / 8 µs ごと。電波が届く速さで
 finish_rx()     アドレス / RX フィルタ、12 バイトの RX ヘッダ、W_RXBUF_WRCSR、IRQ0。
                 自分宛ての MP CMD には返信する（tx.rs send_mp_reply）
```

MP 子機はホストと**ロックステップ**で動きます。ホストのフレームは、子機の時計がその送信時刻に達したときに渡され、子機は次のホストのフレームが来るまで同期時刻で止まります。

### 15.3.7 割り込み

`W_IF & W_IE` が 0 から非 0 になったとき（立ち上がり）だけ、`take_irq()` が一度だけ真を返し、ARM7 の IRQ 24 になります。主な W_IF ビット：0 受信完了、1 送信完了、6 受信開始、7 送信開始、12 MP の終わり、13 ビーコン後、14 US_COMPARE / ビーコン数 / 強制、15 ビーコン前。

## 15.4 `MpLink` と `NetLink`

### 15.4.1 `MpLink`（`wifi/link.rs`）

| メソッド | 内容 |
|---|---|
| `begin()` / `end()` | コントローラの時計が入った / 切れた |
| `new_frame()` | 毎フレームの最初（既定は何もしない） |
| `send_packet(frame, ts)` | 通常のフレーム（ビーコン、管理、データ） |
| `send_cmd(frame, ts)` | MP ホストの CMD |
| `send_reply(frame, ts, aid)` | MP 子機の返信。空の `frame` は「返信なし」 |
| `send_ack(frame, ts)` | MP ホストの ACK |
| `recv_packet(buf) -> MpRecv` | 任意のフレームをブロックせずに受信 |
| `recv_host_packet(buf) -> MpRecv` | 子機：ホストの次の CMD / ACK を待つ（上限あり） |
| `recv_replies(buf, ts, aidmask) -> u16` | ホスト：`ts` の CMD への返信を集める。AID `n` の返信は `buf[(n - 1) * 1024..]`。受け取った AID のマスクを返す |

`MpRecv` は、何も来ない、`len` バイトを受信（送信時刻つき）、関連付けていたホストがいなくなった、の 3 通りです。フレームは 12 バイトの TX/RX ヘッダ + 802.11 フレームで、時刻は送信側の Wi-Fi のマイクロ秒時計です。

### 15.4.2 `NetLink`（`core/net/src/link.rs`）

```text
 ds_peripherals::wifi::WiFi（TX スロット、RX バッファ、MP CMD/reply/ACK）
   |  dyn MpLink
   v
 NetLink（ds_net）             待ち時間の予算、フレームごとの mp::process()
   |  melonds_net::mp::MpHandle（C ABI）
   v
 melonDS MPInterface：Local  同じプロセス内の本体
                      Lan    他のプロセス・他の機械（ENet、UDP 7064、発見は UDP 7063）
```

| 関数 | 内容 |
|---|---|
| `ds_net::init(backend)` | melonDS のネットワークログを `log` へ流し、プロセス全体の MP バックエンドを選ぶ。本体が媒体に乗っている間は変えない |
| `ds_net::process()` | バックエンドの処理（LAN：ENet 接続、発見、ping）。`NetLink` が毎フレーム呼ぶ。本体が動いていないのに LAN セッションがあるときは自分で呼ぶ（呼ばないと相手がタイムアウトする） |
| `NetLink::new(instance)` | `instance` はプロセス内で一意（`MAX_INSTANCES` 未満）。予算なし、毎フレーム `process()` を呼ぶ |
| 予算の設定 | 1 フレームあたりの待ち時間の上限 |
| `process()` を呼ぶかの設定 | 1 プロセスに複数の本体があるとき、1 つ以外は切る |

**待ち時間の予算（stall budget）：** 子機はホストのフレームを（`recv_host_packet`）、ホストは子機の返信を（`recv_replies`）、それぞれバックエンドの受信タイムアウト（melonDS は 25 ms）まで待ちます。これでロックステップを保ちますが、相手が止まると（一時停止、最小化）、8 µs ごとの Wi-Fi の段階のたびにタイムアウトまで待ち、フレームが終わりません。予算を設定すると、1 フレームの待ち時間の合計が予算に達した後は、ブロックせずに受信を試すだけになります。エミュレータ（と、それを UI スレッドで動かす GUI）は止まらず、ゲームは実機と同じく通信エラーとして扱います。予算なしは melonDS と同じ待ち方で、本体ごとにスレッドがある場合に向きます。

## 15.5 GUI 部品（`ds_net_gui`）

### 15.5.1 `LanSession`

```text
 LanSession::new()      ds_net::init(Lan)（本体が動く前に）
 host(name)             lan::host（すぐ）
 join(name, addr)       ワーカースレッドで lan::join（1 回最大約 5 秒、JOIN_TRIES = 3 回）
   busy() == true       この間、呼び出し側は本体を動かさない（LAN バックエンドはスレッド安全でない）
 tick(running)          ワーカー終了 → 参加済み / 失敗
                        本体が動いていない → ds_net::process()（ENet の接続を保つ）
 leave()                lan::leave
```

セッション状態は、なし、ホスト中、接続中（アドレス）、参加中（アドレス）、失敗の 5 つです。`generation()` はセッションの開始・終了で変わり、呼び出し側は本体のリンクを付け直します（melonDS は、本体が Wi-Fi の電源を入れたときに存在した LAN 媒体にしか本体を乗せないため）。`MAX_PLAYERS` = 16、プレイヤー一覧は 500 ms ごとに更新します。参加を 3 回試すのは、ホストが ENet を処理するのはウィンドウを再描画している間だけで、1 回目は取りこぼすことがあるためです。

### 15.5.2 準備ランプ（`ReadyProbe` / `ReadyLight`）

```text
 応答スレッド                           ping スレッド（1 秒ごと）
 UDP PROBE_PORT..（最初の空き）          UDP（任意のポート）
    <-------- PING (node, seq) -------- 各対象の PROBE_PORT..PROBE_PORT+PROBE_PORTS へ
    --------- PONG (node, seq, ------->   （1 台に複数インスタンス）
              hosting/joined/running,     往復時間 -> peers[addr]
              players, game code)
```

| 定数 | 値 |
|---|---|
| `PROBE_PORT` | 7065（`PROBE_PORTS` = 4 個、7065〜7068） |
| ping の間隔 | 1 秒 |
| `STALE_AFTER` | 3.5 秒（これ以上応答がなければ準備できていない） |

通信形式（リトルエンディアン）：`MAGIC`、版、種類（0 ping、1 pong）、ノード ID（u64、プロセスごとに乱数）、シーケンス（u32）。pong はさらに、フラグ（bit 0 ホスト中、1 参加中、2 実行中）、プレイヤー数、最大プレイヤー数、4 バイトのゲームコードを持ちます。

ランプの判定（`ReadyLight::evaluate`）：

| セッション状態 | ping する相手 | 緑の条件 |
|---|---|---|
| なし / 失敗 | 入力したホストのアドレス | 応答があり、空きのあるセッションをホスト中で、同じゲーム（`Want::Joinable`）。つまり「参加」が成功する |
| ホスト中 | セッションの全子機 | 全員が応答し、セッション内で、同じゲーム（`Want::InSession`） |
| 参加中 | ホストを含む他の全プレイヤー | 同上 |
| 接続中 | （変えない） | — |

アドレスの入力は呼び出し側をブロックしません。IP アドレスはその場で解釈し、ホスト名は入力が 400 ms 止まってからワーカースレッドで引きます。`host_part` はポートを落とします（LAN のポートは 7064 固定）。

ファイアウォールで開けるポート：UDP 7063（LAN の発見）、UDP 7064（ENet、MP フレーム）、UDP 7065〜7068（準備ランプ）。

## 15.6 設計判断と理由

- **Wi-Fi は全部移植する。** 以前のすべてゼロのスタブでは、ARM7 の無線ライブラリがハードウェア故障と判断し、あるソフトがタイトルメニューでタッチスクリーンを無効にしていました。リンクなしでも初期化が通るようにしています。
- **`MpLink` は melonDS の `Platform::MP_*` と 1 対 1。** コントローラのコードを `Wifi.cpp` の素直な移植のままにできます。
- **本体ごとに MAC を変える。** 内蔵ファームの MAC は固定なので、インスタンスごとに生成して `Config::mac_address` で書き込みます（[17 章](17_frontend_common.md)）。
- **egui は予算 100 ms（`STALL_BUDGET`）。** エミュレータを UI スレッドで動かすので、相手が止まってもウィンドウを固まらせないためです。
- **LAN セッション中は 1 倍速に固定する。** 相手と歩調を合わせるためです（egui の `speed()`）。

## 15.7 制約・未実装

- 仮想アクセスポイント（melonDS `WifiAP`、ニンテンドー Wi-Fi コネクション）は移植していません。
- 通信対戦の GUI は egui だけです。Tauri と freya のメニュー項目は、理由付きで無効になっています。
- **既知の問題：** ある通信対応ソフトでは、子機がホストを見つけるものの接続に至りません（ARM9 が再スキャンを繰り返す）。

## 15.8 関連テスト

- `core/net/tests/local_mp.rs`：LocalMP 上の 2 つの Wi-Fi コントローラ（実際の FFI 経路）で、一方が送ったフレームが他方の RX バッファに届くこと
- `core/peripherals/src/wifi/tests.rs`：擬似リンクによるコントローラのテスト
- `gui/net/src/ready.rs` のテスト：同じ機械の 2 つのプローブが、ループバックで互いを見つける
- `core/net/examples/mp_pair.rs`：1 プロセスで 2 台を別スレッドで動かします。引数は ROM、フレーム数、出力先です。環境変数は `PRESSN` / `TOUCHN` / `SAVEN`（N = 0 / 1）、`PNG_EVERY`、`SPEED`、`BACKEND=lan`、`CONSOLES=1`、`NOLINK=1` です。60 フレームごとにチャネル、AID、MP 状態、通信量を表示します（子機の AID が 0 以外なら、ホストに関連付けできています）。
- `gui/net/examples/lan_probe.rs`：GUI なしの準備ランプです。ホストするか、アドレスを指定して参加を試し、セッションの状態とランプを 1 秒ごとに表示します。

## 15.9 ROM 実行テスト（機能 ID `wifi`）

自分の ROM で確かめるときは、`docs/cogids-rs/local/roms.toml`（Git 管理外）にケースを書き、`cargo run --release -p ds_rom_test -- --feature wifi` で実行します（[20 章](20_rom_tests.md)）。

| 使うチェック | 確かめること |
|---|---|
| `no_terminate` | 無線ライブラリの初期化（`WM_Enable` / `WM_PowerOn`）で異常にならない |
| `screen_changes` | 無線の初期化の後も進む |

```toml
[[case]]
feature = "wifi"
name = "wifi-example"
rom = "my_game"            # [roms] の別名
frames = 1200
[case.expect]
no_terminate = true
screen_changes = true
```

2 台の通信は扱いません。`core/net/examples/mp_pair.rs` を使ってください（15.8）。
