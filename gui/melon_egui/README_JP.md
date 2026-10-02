# melon_egui

melonDS のエミュレーションコア（C++）を、Rust のバインディング `melonds-rs` 経由で動かし、
画面を [egui](https://github.com/emilk/egui) で描くフロントエンドです。

lunaris 本体の描画やワイヤレス通信の結果を、**動作が確かな melonDS と並べて比較する**ための
「参照用エミュレータ」として作られました。単体でも普通に遊べます
（2 台目の DS、LAN 対戦、リモートデスクトップ、チート、xBRZ など）。

このドキュメントは、**専門家でなくても保守できる**ことを目標に、
「どの処理が・どの順で・どのファイルで」行われるかを図とコードリンクで説明します。

---

## 目次

1. [ビルドと起動](#1-ビルドと起動)
2. [全体の構成](#2-全体の構成)
3. [1 フレーム（再描画 1 回）の流れ](#3-1-フレーム再描画-1-回の流れ)
4. [エミュレーションの実行と速度制御](#4-エミュレーションの実行と速度制御)
5. [入力（キーボード・パッド・タッチ）](#5-入力キーボードパッドタッチ)
6. [画面の描画](#6-画面の描画)
7. [音声](#7-音声)
8. [メニュー → Action → 実行](#8-メニュー--action--実行)
9. [2 台目のコンソール（別スレッド）](#9-2-台目のコンソール別スレッド)
10. [ローカル通信（同じ PC 内の 2 台）](#10-ローカル通信同じ-pc-内の-2-台)
11. [LAN モード](#11-lan-モード)
12. [リモートデスクトップモード](#12-リモートデスクトップモード)
13. [セーブ・ステートセーブ・チート](#13-セーブステートセーブチート)
14. [ファイルダイアログ](#14-ファイルダイアログ)
15. [設定とディレクトリ構成](#15-設定とディレクトリ構成)
16. [テストと動作確認](#16-テストと動作確認)
17. [よくある保守作業](#17-よくある保守作業)

---

## 1. ビルドと起動

melonDS は GCC/Clang 向けの C++ なので、Windows では **LLVM（clang）** が必要です。
`xtask` が Visual Studio 同梱の LLVM を自動で探します。

```text
cargo xtask run   --gui melon            # デバッグ版を起動
cargo xtask build --gui melon --release  # 配布用（コンソール無し・VC ランタイム静的リンク）の単一 exe
```

コマンドライン（[main.rs](src/main.rs#L114)）:

| 引数 | 意味 |
|------|------|
| `melon_egui <rom.nds>` | 起動と同時に ROM を読み込む |
| `--mp` | ROM 読み込み後、2 台目のコンソールも起動する |
| `--renderer <software\|opengl\|compute>[@倍率]` | 今回だけ描画方式を上書き（[take_renderer](src/main.rs#L242)） |
| `--selftest <rom> [frames] [--dump <prefix>]` | ウィンドウ無しで自己診断（[selftest.rs](src/selftest.rs#L32)） |
| `--shot <frames> <out.png> <rom>` | 指定フレームでウィンドウを撮影して終了（[service_shot](src/ui/window.rs#L149)） |

起動の順序:

```text
main()                                                    src/main.rs
 1. ロガー設置（instances/instance1/logs/ に出力）
 2. --renderer / --mp を引数から取り出す
 3. --selftest → 自己診断して終了コードで終わる
    --shot     → RTC を固定（毎回同じ絵になるように）
 4. settings.json からウィンドウサイズと VSync を読む
 5. eframe::run_native
      └ MelonEgui::new()   状態を作る                     src/app/boot.rs
      └ update() を毎回呼ぶ（1 秒に約 60 回）              src/ui/frame.rs
```

`MelonEgui::new` の中身は [boot.rs](src/app/boot.rs#L21) を参照
（設定読込 → OpenGL 初期化 [init_gl](src/app/boot.rs#L160) → 日本語フォント → ROM 起動）。

---

## 2. 全体の構成

```text
                       ┌──────────────────────────────────────────┐
                       │ MelonEgui（アプリ全体の状態, src/app/）    │
                       │   emu: Option<Emu>     1 台目のコンソール  │
                       │   guest: Option<Guest> 2 台目（別スレッド）│
                       │   settings 由来の値, 開いている窓, 通信... │
                       └───────┬──────────────┬───────────────────┘
          描画・入力・クリック │              │ 実行・ディスク
     ┌─────────────────────────┘              └─────────────────────────┐
     ▼                                                                    ▼
 src/ui/        画面・メニュー・設定窓              src/emu/     melonDS コア 1 台分
   frame.rs     1 フレームの順序（最初に読む）       console.rs  起動・フレーム実行・ステート
   menu/        メニューバー → Action を返す         bridge.rs   コア → こちらへのコールバック
   screen.rs    1 台目の画面                       src/guest/   2 台目のコンソールのスレッド
   layout.rs    画像化・描画・タッチ座標変換        src/file/    ROM・セーブ・チート・設定・ダイアログ
   view/        2 画面の配置計算（純粋な計算）      src/mp/      同一 PC 内の無線（2 台をつなぐ）
   window.rs    2 台目の窓・スクショ               src/lan/     LAN モードの UDP 通信
   panes/       設定・ツール窓                     src/remote/  リモートデスクトップ（映像/音声/入力）
   osd.rs       画面上のメッセージ                 src/gl_screen/ OpenGL 描画結果の表示
                                                  src/audio.rs  音声出力
 その他: bindings.rs（キー割当） pad.rs（ゲームパッド） upscale.rs（xBRZ） video.rs（映像設定）
        speed.rs（速度倍率） i18n/（英語・日本語） logger.rs（ログ） fonts.rs（CJK フォント）
```

**大原則**: UI のコードは「何がクリックされたか」を返すだけで、実際の処理は
[`MelonEgui::apply`](src/app/commands.rs#L21) が行います。
ディスクに触る処理は `src/file/` にまとまっています。

---

## 3. 1 フレーム（再描画 1 回）の流れ

eframe が毎回呼ぶ [`update`](src/ui/frame.rs#L20) がすべての起点です。

```text
update(ctx)                                                       src/ui/frame.rs
 │
 ├─ 1. advance()            コンソールを必要フレーム数だけ進め、画像を GPU へ   app/emulation.rs
 ├─ 2. update_window_info() ウィンドウ位置・サイズを記録（次回起動用）         ui/window.rs
 ├─ 3. menu::bar()          メニューを描き、クリックされた Action を受け取る   ui/menu/
 ├─ 4. screens() / osd()    2 画面と FPS・メッセージを描く                   ui/screen.rs, ui/osd.rs
 ├─ 5. panes::show()        開いている設定窓を描く                           ui/panes/
 ├─ 6. guest_view()         2 台目のコンソールの窓                           ui/window.rs
 ├─ 7. second_view()        1 台目の「もう 1 つの窓」                        ui/window.rs
 ├─ 8. apply(action)        メニューで選ばれた処理を実行                     app/commands.rs
 ├─ 9. service_shot()       --shot の撮影                                    ui/window.rs
 └─ 10. request_repaint()   動作中なら次の再描画を予約（停止中は省電力で待つ）
```

---

## 4. エミュレーションの実行と速度制御

### advance() の手順

[`advance`](src/app/emulation.rs#L26) は責務ごとに小さな関数へ分かれています。

```text
advance()
 1. poll_background()     別スレッドの結果を回収（LAN 接続・リモート接続・ファイルダイアログ）
 2. リモートクライアントなら → 受信した画面を表示して終わり（エミュレーションしない）
 3. pads.poll()           ゲームパッドを読む（左スティック押し込み = 速度切替）
 4. frames_due()          今回何フレーム進めるかを決める（下の図）
 5. sample_input()        キー・パッド・タッチを読む（キー割当て変更中は 0）
 6. apply_core_settings() 描画方式・表示画面・チートを、変化があったときだけコアへ
 7. run_frames()          フレームを実行。コアが止まったら理由を表示しクラッシュレポートを書く
 8. after_frames()        音声をスピーカーへ、画像を GPU へ
 9. report_fps() / flush_saves_periodically()   FPS 表示と、1 秒ごとのセーブ書き出し
```

リンク: [frames_due](src/app/emulation.rs#L81) /
[sample_input](src/app/emulation.rs#L117) /
[run_frames](src/app/emulation.rs#L142) /
[after_frames](src/app/emulation.rs#L182)

### 何フレーム進めるか（frames_due）

DS は **約 59.826 fps**（[`FRAME_RATE`](src/emu/clock.rs#L12)）で、
モニターの 60 Hz とずれます。そこで「経過時間 × フレームレート」で借り（debt）を貯め、
整数分だけ実行します（[earn_frames](src/app/emulation.rs#L357)）。

```text
            ┌ 一時停止中 / 非フォーカス → 0（コマ送りが 1 回だけ要求されていれば 1）
            ├ --shot 撮影待ち         → 最大 64 を、目標フレームちょうどで止まるように
due =       ├ フレームレート制限 OFF   → 64（最高速）
            └ 通常 → earn_frames(経過秒 × 59.826 × 速度倍率, 上限 4 × 倍率)
                     ※ LAN 接続中は「回線が維持できるフレームレート」を使う
                     ※ 上限を超えた借りは捨てる（窓をドラッグした後に早送りにならないように）
オーディオ同期 ON かつ等速で、音声バッファが 3/4 以上埋まっていたら → 0（音が追いつくのを待つ）
```

速度倍率（0.5x〜4x, [speed.rs](src/speed.rs#L21)）は、2 台目のコンソールや
LAN 接続があるときは **1x に固定** されます（相手と時間がずれると通信が壊れるため,
[effective_speed](src/app/emulation.rs#L320)）。

### コンソール 1 台分（Emu）

[`Emu`](src/emu/console.rs#L18) が melonDS コア 1 台を包みます。

```text
Emu::boot_inner()                                           src/emu/console.rs
 1. ROM と .sav を読む
 2. HostBridge（コア → こちらへの連絡口）を作る               src/emu/bridge.rs
      write_save  → 最新のセーブ内容を保留（1 秒ごとにファイルへ）
      signal_stop → 停止理由を記録
      mp_*（無線）→ LAN リンク > 同一 PC の無線席 > 何もなし の順で転送
 3. Nds::new → 時計(RTC)設定 → boot()（BIOS/ファームウェア不要の直接起動）
```

よく使う操作: [run_frame_checked](src/emu/console.rs#L119)（1 フレーム実行・停止検出）、
[set_input](src/emu/console.rs#L128)、
[save_state_to](src/emu/console.rs#L186) /
[load_state_from](src/emu/console.rs#L198)、
[import_save](src/emu/console.rs#L211)。

---

## 5. 入力（キーボード・パッド・タッチ）

```text
キーボード ─ bindings.key_mask() ─┐
                                   ├─ OR ─→ DS のボタン (u32 ビットマスク) ─→ Emu::set_input
ゲームパッド ─ pads.poll() ────────┘
   ・押されている状態 + 前回から今回までに「押された」イベントも拾う（短いタップを逃さない）
   ・左スティックは常に十字キー扱い、左スティック押し込みは速度切替

マウス（下画面上）─ touch_coords() ─→ DS のタッチ座標 (0..255, 0..191)
   ・画面の回転を逆算して座標を戻す。前回の描画位置を使う（1 フレーム遅れ, 見た目上問題なし）
```

- キー割当ての保存・変更: [bindings.rs](src/bindings.rs#L175)（設定ファイルに名前で保存）
- 割当て変更待ち中の処理: [poll_rebind](src/app/query.rs#L10)（次に押したキーを割当てに。Esc で取消）
- パッド: [Pads::poll](src/pad.rs#L94)
- タッチ座標: [touch_coords](src/ui/layout.rs#L55)
- 2 台目の窓の入力は、その窓自身の入力状態から読みます（[sample_guest_input](src/ui/window.rs#L22)）

---

## 6. 画面の描画

描画方式（Config ▸ Video settings）によって経路が 2 つあります。

```text
■ ソフトウェア描画
  コアのフレームバッファ (CPU, 256x192, BGRA)
     └ to_image()           RGBA に並べ替え（xBRZ が ON ならここで拡大）      ui/layout.rs
     └ upload_screens()     egui テクスチャへ転送                              ui/layout.rs
     └ paint_screens()      画面配置に従って描く（回転はテクスチャ座標で）      ui/layout.rs

■ OpenGL 描画（内部解像度を上げられる）
  コアが GL テクスチャ（2 層: 上画面/下画面）に直接描く
     └ screens()            egui の描画コールバック内で GL シェーダで表示       ui/screen.rs, gl_screen/
     └ filter_gl_2d()       2D 部分だけ 256x192 で読み戻し → CPU で xBRZ → GPU へ戻す
                            シェーダが「2D の画素は xBRZ 版、3D の画素はそのまま」を画素ごとに選ぶ
```

- 2 画面の配置（縦・横・ハイブリッド、回転、拡大率、アスペクト比）は純粋な計算で、
  [view::layout](src/ui/view/place.rs#L18) にまとまり、テストもあります。
- `Auto` サイズ指定は「真っ黒な画面は使っていない」とみなして片方だけ表示します
  （[resolved_view](src/app/query.rs#L244)）。
- OpenGL シェーダの判定ルールは [gl_screen/shader.rs](src/gl_screen/shader.rs#L50)、
  同じ判定の Rust 版テストは [gl_screen/rule.rs](src/gl_screen/rule.rs#L85)。

---

## 7. 音声

```text
コア (SPU, 48kHz ステレオ i16)
  └ Emu::drain_audio()          溜まった分を取り出す                     emu/console.rs
  └ Audio::push()               デバイスのレートへ線形補間で変換 → リングバッファへ   audio.rs
        ・バッファが半分になるよう再生速度を最大 ±0.5% 微調整（ずれの蓄積を防ぐ）
  └ (別スレッド) cpal のコールバックがリングから取り出して再生
```

- デバイスは専用スレッドで開きます（Windows で winit と COM の方式が衝突するため,
  [Audio::spawn](src/audio.rs#L69)）。
- 「オーディオ同期」はバッファが埋まり過ぎたら次のフレームを走らせないことで実現しています（§4）。

---

## 8. メニュー → Action → 実行

```text
menu::bar()                                     ui/menu/mod.rs
  ├ file_menu / system_menu / view_menu / config_menu / help_menu
  │    各項目: Picked::item(..., Action::Xxx)  ← クリックされた最初の 1 つを記録
  └ Option<Action> を返す
         ▼
apply(action)                                   app/commands.rs
  ├ 1 台目の窓            → apply()
  ├ 2 台目の窓            → apply_to_guest()    （2 台目のスレッドへ Command を送る）
  └ リモートクライアント   → apply_as_client()   （コンソールが無いので多くは「ホスト側の操作です」と表示）
```

3 つの振り分けはすべて **網羅的な match** で書かれています。
`Action` に項目を追加すると、3 か所すべてで扱いを決めるまでコンパイルエラーになります（意図的）。

- [`Action`](src/ui/menu/mod.rs#L57) の一覧
- [`Picked`](src/ui/menu/mod.rs#L127)（メニュークリックの記録係）
- [apply](src/app/commands.rs#L21) /
  [apply_to_guest](src/app/commands.rs#L86) /
  [apply_as_client](src/app/commands.rs#L153)
- 結果は画面左上のメッセージ（OSD）に出ます: `post` / `post_ok` / `post_warn` / `post_error`
  （[query.rs](src/app/query.rs#L183)）。色とログレベルは
  [Severity](src/ui/notice.rs#L17) で一元管理。

---

## 9. 2 台目のコンソール（別スレッド）

System ▸ Multiplayer ▸ Launch new instance で起動します（[launch_instance](src/app/instances.rs#L52)）。

**なぜ別スレッドか**: DS の無線は 1 フレームの中で「親が CMD を送る → 子が即座に返信 → 親が受け取る」
を完了させる必要があり、親の受信は返信が来るまで**待ち**ます。2 台を 1 つのスレッドで交互に動かすと、
子が動く前に親が待ちを終えてしまい、通信エラーになります。

```text
UI スレッド (MelonEgui)                         2 台目のスレッド (guest/run_loop.rs)
───────────────────────                         ─────────────────────────────────
Guest::spawn ──────────────────────────────────→ Emu::boot_mp（無線の席 1）, チート読込
Guest::set_input   (キー・タッチ)  ────────────→ 次のフレームの入力
Guest::send        (Command キュー) ───────────→ perform_commands（フレームの合間に実行）
Guest::set_paused  (一時停止フラグ) ───────────→ 停止 / 再開
Guest::take_screens ←───────── 最新の画面 ────── publish
Guest::take_note    ←───────── 「保存しました」等 ── Shared::say
drop(Guest)        (終了フラグ) ───────────────→ セーブを書き出して終了
```

- スレッド本体: [run](src/guest/run_loop.rs#L37)
  （コマンド処理 → 一時停止判定 → 時計に合わせてフレーム実行 → 画面公開・音声）
- コマンド一覧: [Command](src/guest/command.rs#L10)、
  実行: [perform_commands](src/guest/orders.rs#L14)
- 2 台目の窓の描画: [guest_view](src/ui/window.rs#L41)
  （2 台目専用の設定 `instance2/settings.json` を一時的に適用して描き、変化があったときだけ保存）

---

## 10. ローカル通信（同じ PC 内の 2 台）

melonDS の `LocalMP` を Rust に移植したもの（`src/mp/`）です。
[Airwaves](src/mp/airwaves.rs#L36)（電波）を 2 台で共有し、各台は
[Client](src/mp/client.rs#L13)（席）を持ちます。

```text
1 ラウンド（1 フレームに 1 回）
  親 ── CMD ──────────→ 子の packets キュー
  子 ── REPLY(aid) ───→ 親の replies キュー      親は mp_recv_replies で最大 25ms 待つ
  親 ── ACK ──────────→ 子
  ・古いラウンドへの返信（タイムスタンプが古い）は捨てる
  ・相手が 250ms 以上動いていなければ待たない（一時停止中に毎回 25ms 失うのを防ぐ）
```

通信の様子は System ▸ Multiplayer ▸ Wireless status の窓で確認できます
（CMD が 0 のままなら「ラウンドが始まっていない」, [wireless pane](src/ui/panes/wireless.rs#L18)）。

---

## 11. LAN モード

2 台の PC の**無線そのもの**を UDP で運びます（`src/lan/`, 接続は [lan_session.rs](src/app/lan_session.rs#L36)）。

```text
ホスト                                         ゲスト
start_lan(true)                                start_lan(false)
 1. 実行中の ROM を覚えて一旦停止               1. 同左
 2. 別スレッド: LanHost::accept  ←── HELLO ──── 2. 別スレッド: LanGuest::connect（1 秒ごと最大 10 回）
                                ─── WELCOME ×3 →
 3. 毎フレーム poll_lan: 接続完了 → 同じ ROM を「LAN リンクを無線として」起動
```

VPN 越しでも動くための工夫（melonDS 標準の LAN 実装との違い, [lan/mod.rs](src/lan/mod.rs) 冒頭に詳細）:

1. 返信の待ち時間を固定 25ms ではなく、Ping で**実測**した往復時間から決める
2. CMD と返信は 2 回ずつ送る（1 個落ちても大丈夫）。重複は通し番号で捨てる
3. ラウンドに関係ないパケットはまとめて送れる（既定 OFF）
4. フレームレートを回線が維持できる速さに合わせる（[LinkPace](src/lan/measure.rs#L87)）

ただし原理的に `1 / (16.7ms + 往復時間)` fps が上限です。遅い回線では次のリモートデスクトップを使います。

---

## 12. リモートデスクトップモード

**2 台ともホスト PC で動かし**、2 台目の「映像と音声」をクライアントへ送り、
クライアントの「ボタンとタッチ」を受け取ります。無線のラウンドはホスト内で完結するので、
回線が遅くても 59.8fps を保てます（`src/remote/`, 接続は [remote_session.rs](src/app/remote_session.rs#L34)）。

```text
ホスト PC                                                        クライアント PC
┌──────────────────────────┐   映像（16x16 タイル差分）+ 音声 →   ┌──────────────┐
│ 1 台目 ⇄ 2 台目（同一 PC  │ ─────────────────────────────────→ │ 表示とスピーカー│
│        内の無線）         │ ←───────── ボタン・タッチ ──────── │ 入力は最初に送る│
└──────────────────────────┘                                    └──────────────┘
```

- 映像: 変化したタイル + 順番に全体を描き直すタイルだけを送る。1 パケットだけで適用できるので、
  失われても数フレーム後に自然に直る（[Encoder](src/remote/encoder.rs#L28) /
  [Decoder](src/remote/decoder.rs#L16)）
- 帯域が足りないと送るフレームを間引く（遅延は増えない, [Pacer](src/remote/encoder.rs#L150)）
- 音声は 24kHz に落として送り、受信側で元に戻す（[Downsampler](src/remote/audio.rs#L46)）
- クライアントの毎フレーム処理: [service_remote_client](src/app/remote_session.rs#L161)
  （① 入力を送る → ② 画面を更新 → ③ 音声を再生）
- セーブ・ステートなどは**すべてホスト側**にあります。クライアントは何も保存しません。

---

## 13. セーブ・ステートセーブ・チート

```text
■ セーブ（ゲーム内セーブ, .sav）
  コアが書く → HostBridge::write_save で保留 → 1 秒ごと / ROM を閉じるとき にファイルへ
  File ▸ Import savefile: ファイル選択 → DeSmuME の .dsv フッターなら除去 → .sav に書く → 再起動

■ ステートセーブ（.mlN）
  スロット 1〜8: すぐ保存・読込（<rom>.ml1 ... <rom>.ml8）
  「ファイルから」: ダイアログを開き、答えが来たら実行（§14）
  読込の直前の状態をメモリに残す → File ▸ Undo state load で戻せる

■ チート（Action Replay, melonDS の .mch 形式そのまま）
  起動時 instances/instance1/cheats/<rom>.mch を読む
  チート窓で編集 → 保存 → 毎フレーム apply_cheats が「変化があれば」コアへ（2 台目にも送る）
```

- セーブ関連: [save.rs](src/file/save.rs#L37)
- 2 台目の同じ操作は 2 台目のスレッドで: [orders.rs](src/guest/orders.rs#L14)
- チート: [cheats.rs](src/file/cheats.rs#L171)、ファイル形式: [mch.rs](src/file/mch.rs#L111)
- ROM 読込: [load](src/file/rom.rs#L14)

---

## 14. ファイルダイアログ

OS のファイルダイアログは呼ぶと終わるまで戻らない（=その間ウィンドウが固まる）ため、
**別スレッドで開き、答えは後の再描画で受け取ります**。

```text
メニュー → ask(purpose, request)     ダイアログを別スレッドで開く         file/dialog.rs
毎フレーム → poll_dialog()           まだ → 何もしない（エミュは動き続ける）
                                    答えあり → purpose に応じて実行
                                      OpenRom → load / ImportSave → import_savefile_from / ...
```

`purpose`（[DialogPurpose](src/app/mod.rs#L127)）がダイアログと一緒に運ばれるので、
数フレーム後に答えが来ても別の操作に誤って適用されることがありません。
同じ「別スレッドで待って後で回収」の仕組みを LAN・リモート接続でも使っています（[worker.rs](src/app/worker.rs#L16)）。

---

## 15. 設定とディレクトリ構成

```text
<ワークスペース直下 or exe の隣>/instances/        （環境変数 MELON_EGUI_INSTANCES で変更可）
 ├ instance1/                1 台目
 │   ├ settings.json         すべての設定（表示・映像・キー割当て・最近の ROM ...）
 │   ├ saves/  states/  cheats/  logs/
 │   └ last-stop.txt         コンソールが止まったときのレポート
 └ instance2/                2 台目（同じ構成, セーブは 1 台目からコピーして開始）

./instances/translation.<eng|jpn>.json   UI 文言の上書き用（起動時に雛形を書き出す）
   ※ これだけは「起動したときのカレントディレクトリ」基準です（他は exe の位置基準）。
     cargo run とダブルクリック起動で場所が変わる点に注意（i18n/map.rs の i18n_path）。
```

- 場所の決め方: [instances_dir](src/file/settings.rs#L247)
  （実行時のカレントディレクトリに依存しないよう exe の位置から決める）
- 設定の読み書き: [Settings](src/file/settings.rs#L26)、
  アプリ状態との相互変換: [settings()](src/app/boot.rs#L195) /
  [apply_runtime_settings](src/app/boot.rs#L223)
- 壊れた / 古い settings.json は既定値で補われ、範囲外の値は丸められます（[normalize](src/file/settings.rs#L162)）。

---

## 16. テストと動作確認

```text
cargo test -p melon_egui                       単体テスト（178 件）
MELON_TEST_ROM=<rom.nds> cargo test ...        ROM を使うテストも実行（セーブ取込・LAN 起動）
melon_egui --selftest <rom> 600 --dump out     ウィンドウ無しの総合診断 + 画面を PNG 出力
melon_egui --shot 900 out.png <rom> [--mp] [--renderer opengl@2]
                                               実ウィンドウを撮影（描画経路の確認）
```

`--selftest` と `--shot` は時計を固定するので、**同じ条件なら毎回同じ PNG** になります。
リファクタリングの前後で PNG をバイト比較すれば、描画・実行に変化がないことを確認できます。

---

## 17. よくある保守作業

| やりたいこと | 触る場所 |
|--------------|----------|
| メニュー項目を追加 | [`Action`](src/ui/menu/mod.rs#L57) に追加 → 該当メニューで `m.item(...)` → `apply` / `apply_to_guest` / `apply_as_client` の 3 か所で処理（コンパイラが教えてくれる） |
| 設定窓を追加 | [`Pane`](src/ui/panes/mod.rs#L53) に追加 → `title` と `body` に 1 行ずつ → 描画関数を作る |
| 保存する設定を追加 | [`Settings`](src/file/settings.rs#L26) にフィールド（`#[serde(default)]` なので古いファイルも読める）→ `settings()` と `apply_runtime_settings` に 1 行ずつ |
| 翻訳を追加 | [`I18nKey`](src/i18n/keys.rs#L13) にバリアント（doc コメントが英語, `#[i18n(ja = "...")]` が日本語） |
| コアの機能を呼ぶ | `emu.nds.<メソッド>()` を直接呼ぶ。複数箇所で同じ手順になるなら [Emu](src/emu/console.rs#L34) にまとめる |
| 2 台目にも同じ操作をさせる | [Command](src/guest/command.rs#L10) を追加 → [perform_commands](src/guest/orders.rs#L14) で処理（コアは必ずそのスレッドで触る） |
| 映像設定を追加 | [`VideoOptions`](src/video.rs#L65) → 設定窓 [video_settings](src/ui/panes/settings.rs#L74) |

**注意点（はまりやすい所）**

- コア（`melonds`）は**そのコンソールを動かしているスレッドからだけ**触ること。2 台目は `Command` 経由。
- コアの設定（描画方式・チートなど）は「前回渡した値」と比べて**変化したときだけ**渡す設計です
  （描画方式の切替は重い）。`applied_*` フィールドがその記録です。
- 1 台目は起動時に必ず無線の席 0 を取ります。後から席を付けることはできません（コア生成時に固定）。
- `--shot` の結果が変わったら、描画経路のどこかが変わったということです（OSD を OFF にして比較）。
