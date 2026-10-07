//! Every translatable string, as a key with its English and its Japanese.
//!
//! The doc comment on a variant is the English; `#[i18n(ja = "…")]` is the
//! Japanese. `i18n_derive` turns both into `const fn`s, so neither costs an
//! allocation and neither can drift out of sync with the key list.

/// Every translatable string in the front end.
///
/// The doc comment is the English; `#[i18n(ja = "…")]` is the Japanese.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize, i18n_derive::I18n)]
#[serde(rename_all = "snake_case")]
pub enum I18nKey {
    // -- menu bar ------------------------------------------------------------
    /// File
    #[i18n(ja = "ファイル")]
    FileLabel,

    /// System
    #[i18n(ja = "システム")]
    SystemLabel,

    /// View
    #[i18n(ja = "表示")]
    ViewLabel,

    /// Config
    #[i18n(ja = "設定")]
    ConfigLabel,

    /// Help
    #[i18n(ja = "ヘルプ")]
    HelpLabel,

    // -- File menu -----------------------------------------------------------
    /// Open ROM...
    #[i18n(ja = "ROM を開く...")]
    OpenRom,

    /// Open recent
    #[i18n(ja = "最近開いた ROM")]
    OpenRecent,

    /// (nothing yet)
    #[i18n(ja = "(まだありません)")]
    NothingYet,

    /// Clear
    #[i18n(ja = "消去")]
    Clear,

    /// Boot firmware
    #[i18n(ja = "ファームウェアを起動")]
    BootFirmware,

    /// Insert cart...
    #[i18n(ja = "カートリッジを挿入...")]
    InsertCart,

    /// Eject cart
    #[i18n(ja = "カートリッジを取り出す")]
    EjectCart,

    /// Import savefile
    #[i18n(ja = "セーブデータを読み込む")]
    ImportSavefile,

    /// Save state
    #[i18n(ja = "ステートセーブ")]
    SaveState,

    /// Load state
    #[i18n(ja = "ステートロード")]
    LoadState,

    /// Undo state load
    #[i18n(ja = "ステートロードを取り消す")]
    UndoStateLoad,

    /// File...
    #[i18n(ja = "ファイルを指定...")]
    FromFile,

    /// Open melon_egui directory
    #[i18n(ja = "melon_egui のフォルダを開く")]
    OpenDirectory,

    /// Quit
    #[i18n(ja = "終了")]
    Quit,

    // -- System menu ---------------------------------------------------------
    /// Pause
    #[i18n(ja = "一時停止")]
    Pause,

    /// Reset
    #[i18n(ja = "リセット")]
    Reset,

    /// Stop
    #[i18n(ja = "停止")]
    Stop,

    /// Frame step
    #[i18n(ja = "コマ送り")]
    FrameStep,

    /// Power management
    #[i18n(ja = "電源管理")]
    PowerManagement,

    /// Date and time
    #[i18n(ja = "日付と時刻")]
    DateAndTime,

    /// Enable cheats
    #[i18n(ja = "チートを有効にする")]
    EnableCheats,

    /// Setup cheat codes
    #[i18n(ja = "チートコードの設定")]
    SetupCheats,

    /// ROM info
    #[i18n(ja = "ROM 情報")]
    RomInfo,

    /// RAM search
    #[i18n(ja = "RAM 検索")]
    RamSearch,

    /// Manage DSi titles
    #[i18n(ja = "DSi タイトルの管理")]
    ManageDsiTitles,

    // -- Multiplayer ---------------------------------------------------------
    /// Multiplayer
    #[i18n(ja = "通信プレイ")]
    Multiplayer,

    /// Launch new instance
    #[i18n(ja = "2 台目を起動")]
    LaunchInstance,

    /// Close second instance
    #[i18n(ja = "2 台目を閉じる")]
    CloseInstance,

    /// Wireless status
    #[i18n(ja = "無線の状態")]
    WirelessStatus,

    /// LAN room
    #[i18n(ja = "LAN ルーム")]
    LanRoom,

    /// Host bind
    #[i18n(ja = "ホストの待ち受けアドレス")]
    HostBind,

    /// Guest IP
    #[i18n(ja = "接続先 IP アドレス")]
    GuestIp,

    /// Host LAN game
    #[i18n(ja = "LAN の親機になる")]
    HostLanGame,

    /// Guest LAN game
    #[i18n(ja = "LAN の子機として参加")]
    GuestLanGame,

    /// Disconnect
    #[i18n(ja = "切断")]
    Disconnect,

    /// Link quality
    #[i18n(ja = "回線品質")]
    LinkQuality,

    /// Round trip
    #[i18n(ja = "往復遅延")]
    RoundTrip,

    /// Jitter
    #[i18n(ja = "ゆらぎ")]
    Jitter,

    /// Reply budget
    #[i18n(ja = "応答待ち時間")]
    ReplyBudget,

    /// Rounds completed
    #[i18n(ja = "成立した通信ラウンド")]
    RoundsCompleted,

    /// Sustainable frame rate
    #[i18n(ja = "回線が支えられるフレームレート")]
    SustainableFps,

    /// Duplicates discarded
    #[i18n(ja = "重複として破棄")]
    DuplicatesDropped,

    /// VPN tuning
    #[i18n(ja = "VPN 向けの調整")]
    VpnTuning,

    // -- Remote Desktop ------------------------------------------------------
    /// Remote Desktop
    #[i18n(ja = "リモートデスクトップ")]
    RemoteDesktop,

    /// Host Remote Desktop game
    #[i18n(ja = "リモートデスクトップの親機になる")]
    HostRemoteDesktop,

    /// Join Remote Desktop game
    #[i18n(ja = "リモートデスクトップに参加")]
    JoinRemoteDesktop,

    /// End Remote Desktop session
    #[i18n(ja = "リモートデスクトップを終了")]
    StopRemoteDesktop,

    /// Both consoles run on the host. Only the picture, the sound and the \
    /// controls cross the network, so no emulated frame ever waits for it.
    #[i18n(ja = "2 台とも親機側で動きます。ネットワークを渡るのは映像・音声・操作だけなので、\
                 エミュレートされたフレームが通信を待つことはありません。")]
    RemoteDesktopExplained,

    /// The client owns nothing: saves, savestates and cheats all stay on the \
    /// host.
    #[i18n(ja = "子機は何も保持しません。セーブ・ステート・チートはすべて親機側にあります。")]
    RemoteClientOwnsNothing,

    /// Video
    #[i18n(ja = "映像")]
    Video,

    /// Input latency
    #[i18n(ja = "操作の遅延")]
    InputLatency,

    /// Refresh period (frames)
    #[i18n(ja = "全面更新にかける枚数")]
    RefreshPeriod,

    /// Stream audio
    #[i18n(ja = "音声を送る")]
    StreamAudio,

    /// Audio lag limit (ms)
    #[i18n(ja = "音声の遅れの上限 (ミリ秒)")]
    AudioLagLimit,

    /// Port
    #[i18n(ja = "ポート")]
    Port,

    /// Remote Desktop settings
    #[i18n(ja = "リモートデスクトップの設定")]
    RemoteDesktopSettings,

    /// Minimum reply wait (ms)
    #[i18n(ja = "応答待ちの下限 (ミリ秒)")]
    MinBudget,

    /// Maximum reply wait (ms)
    #[i18n(ja = "応答待ちの上限 (ミリ秒)")]
    MaxBudget,

    /// Jitter allowance
    #[i18n(ja = "ゆらぎの見込み倍率")]
    JitterFactor,

    /// Copies of each reply
    #[i18n(ja = "応答パケットの送信回数")]
    ReplyCopies,

    /// Batch window (ms)
    #[i18n(ja = "まとめ送りの待ち時間 (ミリ秒)")]
    BatchWindow,

    /// Follow the link's frame rate
    #[i18n(ja = "回線に合わせてフレームレートを下げる")]
    PaceToLink,

    // -- View menu -----------------------------------------------------------
    /// Screen size
    #[i18n(ja = "画面サイズ")]
    ScreenSize,

    /// Screen rotation
    #[i18n(ja = "画面の回転")]
    ScreenRotation,

    /// Screen gap
    #[i18n(ja = "画面の間隔")]
    ScreenGap,

    /// Screen layout
    #[i18n(ja = "画面の配置")]
    ScreenLayout,

    /// Swap screens
    #[i18n(ja = "上下の画面を入れ替える")]
    SwapScreens,

    /// Screen sizing
    #[i18n(ja = "画面の拡大方法")]
    ScreenSizing,

    /// Force integer scaling
    #[i18n(ja = "整数倍で拡大する")]
    IntegerScaling,

    /// Aspect ratio
    #[i18n(ja = "アスペクト比")]
    AspectRatio,

    /// Top
    #[i18n(ja = "上画面")]
    TopScreen,

    /// Bottom
    #[i18n(ja = "下画面")]
    BottomScreen,

    /// Open new window
    #[i18n(ja = "新しいウィンドウを開く")]
    NewWindow,

    /// Screen filtering
    #[i18n(ja = "画面を滑らかにする")]
    ScreenFiltering,

    /// Show OSD
    #[i18n(ja = "画面上にメッセージを表示")]
    ShowOsd,

    // -- Config menu ---------------------------------------------------------
    /// Emu settings
    #[i18n(ja = "エミュレータ設定")]
    EmuSettings,

    /// Preferences...
    #[i18n(ja = "環境設定...")]
    Preferences,

    /// Input and hotkeys
    #[i18n(ja = "入力とホットキー")]
    InputAndHotkeys,

    /// Video settings
    #[i18n(ja = "映像設定")]
    VideoSettings,

    /// Camera settings
    #[i18n(ja = "カメラ設定")]
    CameraSettings,

    /// Audio settings
    #[i18n(ja = "音声設定")]
    AudioSettings,

    /// Multiplayer settings
    #[i18n(ja = "通信プレイ設定")]
    MultiplayerSettings,

    /// Wifi settings
    #[i18n(ja = "無線 LAN 設定")]
    WifiSettings,

    /// Firmware settings
    #[i18n(ja = "ファームウェア設定")]
    FirmwareSettings,

    /// Interface settings
    #[i18n(ja = "インターフェース設定")]
    InterfaceSettings,

    /// Path settings
    #[i18n(ja = "フォルダ設定")]
    PathSettings,

    /// Limit framerate
    #[i18n(ja = "フレームレートを制限する")]
    LimitFramerate,

    /// Audio sync
    #[i18n(ja = "音声に同期する")]
    AudioSync,

    /// Language
    #[i18n(ja = "言語")]
    LanguageLabel,

    /// About...
    #[i18n(ja = "このソフトについて...")]
    About,

    // -- Remote Desktop: session messages -------------------------------------
    /// a Remote Desktop session is already being established
    #[i18n(ja = "リモートデスクトップの接続処理がすでに進行中です")]
    RdAlreadyPending,

    /// load a cart first — the host runs both consoles
    #[i18n(ja = "先にカートリッジを読み込んでください — 親機が 2 台とも動かします")]
    RdLoadCartFirst,

    /// not a valid address: {0}
    #[i18n(ja = "アドレスが正しくありません: {0}")]
    InvalidAddress,

    /// Remote Desktop host failed: {0}
    #[i18n(ja = "リモートデスクトップの親機を開始できませんでした: {0}")]
    RdHostFailed,

    /// Remote Desktop client failed: {0}
    #[i18n(ja = "リモートデスクトップに参加できませんでした: {0}")]
    RdClientFailed,

    /// cannot start a Remote Desktop session: {0}
    #[i18n(ja = "リモートデスクトップを開始できません: {0}")]
    RdCannotStart,

    /// Remote Desktop: hosting
    #[i18n(ja = "リモートデスクトップ: 親機")]
    RdRoomHosting,

    /// Remote Desktop: joining
    #[i18n(ja = "リモートデスクトップ: 参加中")]
    RdRoomJoining,

    /// Remote Desktop: connected
    #[i18n(ja = "リモートデスクトップ: 接続済み")]
    RdRoomConnected,

    /// Remote Desktop: offline
    #[i18n(ja = "リモートデスクトップ: オフライン")]
    RdRoomOffline,

    /// waiting for a client on {0}
    #[i18n(ja = "{0} で子機の参加を待っています")]
    RdWaitingOn,

    /// connecting to {0}
    #[i18n(ja = "{0} に接続しています")]
    RdConnectingTo,

    /// the Remote Desktop worker stopped unexpectedly
    #[i18n(ja = "リモートデスクトップの接続処理が予期せず停止しました")]
    RdWorkerStopped,

    /// Client {0} connected; listening on {1}
    #[i18n(ja = "子機 {0} が接続しました (待ち受け: {1})")]
    RdClientConnected,

    /// Remote Desktop: {0} is playing instance 2
    #[i18n(ja = "リモートデスクトップ: {0} が 2 台目を操作しています")]
    RdPlayingInstance2,

    /// Watching {0} from {1}
    #[i18n(ja = "{0} の画面を受信中 (自分: {1})")]
    RdWatching,

    /// Remote Desktop: connected to {0}
    #[i18n(ja = "リモートデスクトップ: {0} に接続しました")]
    RdConnectedTo,

    /// no Remote Desktop session is running
    #[i18n(ja = "リモートデスクトップのセッションは動いていません")]
    RdNoSession,

    /// No Remote Desktop session
    #[i18n(ja = "リモートデスクトップ未接続")]
    RdNoSessionStatus,

    /// Remote Desktop session ended
    #[i18n(ja = "リモートデスクトップを終了しました")]
    RdEnded,

    /// Remote Desktop connection cancelled
    #[i18n(ja = "リモートデスクトップの接続を取り消しました")]
    RdCancelled,

    // -- Remote Desktop: is the other machine ready? -------------------------
    /// Partner
    #[i18n(ja = "相手の状態")]
    PartnerLabel,

    /// Partner ready — answering in {0} ms
    #[i18n(ja = "相手の準備OK — 応答 {0} ms")]
    PeerReady,

    /// Partner not ready — no answer for {0} s
    #[i18n(ja = "相手の準備ができていません — {0} 秒間応答がありません")]
    PeerSilent,

    /// Partner not ready — no answer yet
    #[i18n(ja = "相手の準備ができていません — まだ応答がありません")]
    PeerNeverAnswered,

    /// Partner not ready — waiting for someone to join on port {0}
    #[i18n(ja = "相手の準備ができていません — ポート {0} で参加を待っています")]
    PeerHostWaiting,

    /// Partner not ready — {0} is not hosting yet (asking again every half second)
    #[i18n(
        ja = "相手の準備ができていません — {0} はまだ親機になっていません (0.5 秒ごとに再確認中)"
    )]
    PeerClientWaiting,

    /// Partner ready — {0} is hosting and waiting ({1} ms). Join will connect.
    #[i18n(ja = "相手の準備OK — {0} は親機として待機中です ({1} ms)。参加すると接続します。")]
    ProbeReady,

    /// Partner not ready — {0} is hosting, but already connected to someone else
    #[i18n(ja = "相手の準備ができていません — {0} は親機ですが、すでに別の相手と接続中です")]
    ProbeBusy,

    /// Partner not ready — no Remote Desktop host answers at {0}
    #[i18n(ja = "相手の準備ができていません — {0} でリモートデスクトップの親機が応答しません")]
    ProbeSilent,

    /// Partner unknown — the Guest IP box does not hold a valid address
    #[i18n(ja = "相手の状態は不明 — 接続先 IP アドレスが正しくありません")]
    ProbeBadAddress,

    /// This machine pings the other twice a second. Before a session it asks \
    /// the Guest IP whether it is hosting; during one, both ends answer each \
    /// other's pings.
    #[i18n(ja = "このマシンは相手に 1 秒間に 2 回 PING を送っています。接続前は接続先 IP が\
                 親機として待機しているかを確認し、接続中はお互いの PING への応答で確認します。")]
    PartnerExplained,

    // -- LAN session messages -------------------------------------------------
    /// a LAN connection is already being established
    #[i18n(ja = "LAN 接続の処理がすでに進行中です")]
    LanAlreadyPending,

    /// load a cart first
    #[i18n(ja = "先にカートリッジを読み込んでください")]
    LoadCartFirst,

    /// cannot start LAN connection: {0}
    #[i18n(ja = "LAN 接続を開始できません: {0}")]
    LanCannotStart,

    /// Hosting LAN room
    #[i18n(ja = "LAN ルームの親機")]
    LanRoomHosting,

    /// Joining LAN room
    #[i18n(ja = "LAN ルームに参加中")]
    LanRoomJoining,

    /// LAN room connected
    #[i18n(ja = "LAN ルーム接続済み")]
    LanRoomConnected,

    /// LAN room offline
    #[i18n(ja = "LAN ルームはオフラインです")]
    LanRoomOffline,

    /// No LAN room
    #[i18n(ja = "LAN ルームなし")]
    LanNoRoom,

    /// LAN room is offline
    #[i18n(ja = "LAN ルームはオフラインです")]
    LanStatusOffline,

    /// Checking: waiting for guest on {0}
    #[i18n(ja = "確認中: {0} で子機を待っています")]
    LanCheckingHost,

    /// waiting for a LAN guest on {0}
    #[i18n(ja = "{0} で LAN の子機を待っています")]
    LanWaitingGuest,

    /// Checking: connecting to {0}
    #[i18n(ja = "確認中: {0} に接続しています")]
    LanCheckingGuest,

    /// connecting to LAN host {0}
    #[i18n(ja = "LAN の親機 {0} に接続しています")]
    LanConnectingHost,

    /// LAN connection worker stopped unexpectedly
    #[i18n(ja = "LAN 接続の処理が予期せず停止しました")]
    LanWorkerStopped,

    /// LAN connected, but no cart is loaded
    #[i18n(ja = "LAN に接続しましたが、カートリッジが読み込まれていません")]
    LanNoCart,

    /// Connected: local {0}, remote {1}
    #[i18n(ja = "接続済み: 自分 {0}、相手 {1}")]
    LanConnectedStatus,

    /// LAN game connected: {0}
    #[i18n(ja = "LAN 通信プレイに接続しました: {0}")]
    LanGameConnected,

    /// Connection check failed: {0}
    #[i18n(ja = "接続の確認に失敗しました: {0}")]
    LanCheckFailed,

    /// LAN game failed: {0}
    #[i18n(ja = "LAN 通信プレイに失敗しました: {0}")]
    LanGameFailed,

    // -- Remote Desktop: session numbers and settings -------------------------
    /// {0} fps, {1} Mbit/s  ({2} tiles, {3} B in the last frame)
    #[i18n(ja = "{0} fps、{1} Mbit/s (直前のフレーム: タイル {2} 枚、{3} B)")]
    RdVideoValue,

    /// {0} Hz, {1} Mbit/s  (was {2} at 48 kHz)
    #[i18n(ja = "{0} Hz、{1} Mbit/s (48 kHz のままなら {2})")]
    RdAudioValue,

    /// Frames skipped
    #[i18n(ja = "送信を省いたフレーム")]
    RdFramesSkipped,

    /// {0} (skipping costs smoothness, never latency)
    #[i18n(ja = "{0} (省くと滑らかさは落ちますが、遅延は増えません)")]
    RdFramesSkippedValue,

    /// Frames
    #[i18n(ja = "フレーム")]
    RdFrames,

    /// {0} ({1} datagrams, {2} MiB, {3} discarded)
    #[i18n(ja = "{0} (データグラム {1} 個、{2} MiB、破棄 {3})")]
    RdFramesValue,

    /// Audio delivered
    #[i18n(ja = "届いた音声")]
    RdAudioDelivered,

    /// {0} pairs, {1} dropped to stay in step
    #[i18n(ja = "{0} サンプル組 (映像に遅れないよう {1} 組を破棄)")]
    RdAudioDeliveredValue,

    /// Input samples
    #[i18n(ja = "操作データの数")]
    RdInputSamples,

    /// Applies to the next Remote Desktop session.
    #[i18n(ja = "次のリモートデスクトップ接続から反映されます。")]
    RdAppliesNext,

    /// Every tile is repainted at least this often, which is the whole of the \
    /// loss recovery: a dropped datagram costs a few stale tiles for this many \
    /// frames and nothing more. Lower recovers faster and costs bandwidth.
    #[i18n(ja = "どのタイルも最低この間隔で描き直します。これがパケット消失からの回復のすべてで、\
                 失われたデータグラムはこの枚数のあいだ一部のタイルが古いままになるだけです。\
                 小さくすると回復が速くなる代わりに帯域を使います。")]
    RdRefreshPeriodHint,

    /// Audio queued past this is dropped rather than played. Sound that is \
    /// queued is sound that is late, and a queue that is never trimmed slides \
    /// further behind the picture for as long as the session lasts.
    #[i18n(
        ja = "これを超えて溜まった音声は再生せずに捨てます。溜まった音声はそれだけ遅れた音声で、\
                 削らなければ接続している間ずっと映像から遅れ続けます。"
    )]
    RdAudioLagHint,

    /// Reset to defaults
    #[i18n(ja = "初期値に戻す")]
    ResetToDefaults,

    // -- View options -----------------------------------------------------------
    /// Natural
    #[i18n(ja = "自然な向き")]
    LayoutNatural,

    /// Vertical
    #[i18n(ja = "縦に並べる")]
    LayoutVertical,

    /// Horizontal
    #[i18n(ja = "横に並べる")]
    LayoutHorizontal,

    /// Hybrid
    #[i18n(ja = "ハイブリッド")]
    LayoutHybrid,

    /// Even
    #[i18n(ja = "同じ大きさ")]
    SizingEven,

    /// Emphasize top
    #[i18n(ja = "上画面を大きく")]
    SizingEmphasizeTop,

    /// Emphasize bottom
    #[i18n(ja = "下画面を大きく")]
    SizingEmphasizeBottom,

    /// Auto
    #[i18n(ja = "自動")]
    SizingAuto,

    /// Top only
    #[i18n(ja = "上画面のみ")]
    SizingTopOnly,

    /// Bottom only
    #[i18n(ja = "下画面のみ")]
    SizingBottomOnly,

    /// 4:3 (native)
    #[i18n(ja = "4:3 (本来の比率)")]
    AspectNative,

    /// window
    #[i18n(ja = "ウィンドウに合わせる")]
    AspectWindow,

    // -- Renderers and filters ------------------------------------------------
    /// Software
    #[i18n(ja = "ソフトウェア")]
    RendererSoftware,

    /// OpenGL (compute shader)
    #[i18n(ja = "OpenGL (コンピュートシェーダー)")]
    RendererCompute,

    /// None
    #[i18n(ja = "なし")]
    UpscaleNone,

    // -- DS buttons -------------------------------------------------------------
    /// Start
    #[i18n(ja = "スタート")]
    ButtonStart,

    /// Select
    #[i18n(ja = "セレクト")]
    ButtonSelect,

    /// Up
    #[i18n(ja = "上")]
    DirUp,

    /// Down
    #[i18n(ja = "下")]
    DirDown,

    /// Left
    #[i18n(ja = "左")]
    DirLeft,

    /// Right
    #[i18n(ja = "右")]
    DirRight,

    /// No audio output device.
    #[i18n(ja = "音声の出力デバイスがありません。")]
    NoAudioDevice,

    // -- OSD messages: the console ----------------------------------------------
    /// console {0}
    #[i18n(ja = "本体: {0}")]
    ConsoleNote,

    /// second instance: {0}
    #[i18n(ja = "2 台目: {0}")]
    SecondConsoleNote,

    /// core stopped
    #[i18n(ja = "エミュレータのコアが停止しました")]
    CoreStopped,

    /// renderer: software, threaded
    #[i18n(ja = "レンダラ: ソフトウェア (マルチスレッド)")]
    RendererNowSoftwareThreaded,

    /// renderer: software
    #[i18n(ja = "レンダラ: ソフトウェア")]
    RendererNowSoftware,

    /// renderer: {0} at {1}x internal resolution
    #[i18n(ja = "レンダラ: {0} (内部解像度 {1} 倍)")]
    RendererNowGl,

    /// could not create the {0} renderer; on {1} instead
    #[i18n(ja = "{0} レンダラを作成できなかったため、{1} を使います")]
    RendererFellBack,

    /// speed {0} (not applied: a second console is running)
    #[i18n(ja = "速度 {0} (2 台目が動いているため適用されません)")]
    SpeedLocked,

    /// speed {0}
    #[i18n(ja = "速度 {0}")]
    SpeedNow,

    /// no cart loaded — File ▸ Open ROM...
    #[i18n(ja = "カートリッジが読み込まれていません — ファイル ▸ ROM を開く...")]
    NoCartHint,

    /// no cart loaded
    #[i18n(ja = "カートリッジが読み込まれていません")]
    NoCartLoaded,

    /// CJK fallback: {0}
    #[i18n(ja = "日本語フォント: {0}")]
    CjkFallback,

    /// No CJK font found; Japanese text will show as boxes.
    #[i18n(ja = "日本語フォントが見つかりません。日本語は四角で表示されます。")]
    NoCjkFont,

    /// loaded {0}
    #[i18n(ja = "{0} を読み込みました")]
    RomLoaded,

    /// failed to load {0}: {1}
    #[i18n(ja = "{0} を読み込めませんでした: {1}")]
    RomLoadFailed,

    /// cart ejected
    #[i18n(ja = "カートリッジを取り出しました")]
    CartEjected,

    /// recent list cleared
    #[i18n(ja = "最近開いた ROM の一覧を消去しました")]
    RecentCleared,

    /// second window opened
    #[i18n(ja = "2 つ目のウィンドウを開きました")]
    SecondWindowOpened,

    /// second window closed
    #[i18n(ja = "2 つ目のウィンドウを閉じました")]
    SecondWindowClosed,

    /// reset
    #[i18n(ja = "リセットしました")]
    ResetDone,

    /// that command belongs to the first console
    #[i18n(ja = "その操作は 1 台目のウィンドウで行ってください")]
    FirstConsoleOnly,

    /// this window is a Remote Desktop client — the host owns the console
    #[i18n(ja = "このウィンドウはリモートデスクトップの子機です — 本体は親機側にあります")]
    RemoteClientNoConsole,

    // -- OSD messages: the second console ---------------------------------------
    /// no second console is running
    #[i18n(ja = "2 台目は動いていません")]
    NoSecondConsole,

    /// second console stopped
    #[i18n(ja = "2 台目を停止しました")]
    SecondConsoleStopped,

    /// second console closed
    #[i18n(ja = "2 台目を閉じました")]
    SecondConsoleClosed,

    /// second instance launched - both consoles share the airwaves
    #[i18n(ja = "2 台目を起動しました — 2 台は同じ無線を共有しています")]
    SecondConsoleLaunched,

    /// second instance launched — its picture and sound go to the remote player
    #[i18n(ja = "2 台目を起動しました — 映像と音声はリモートの相手に送られます")]
    SecondConsoleStreamed,

    /// cannot make {0}: {1}; sharing the save
    #[i18n(ja = "{0} を作成できません: {1}。1 台目とセーブを共有します")]
    GuestSaveDirFailed,

    /// cannot seed {0}: {1}
    #[i18n(ja = "{0} にセーブをコピーできません: {1}")]
    GuestSaveSeedFailed,

    /// could not boot: {0}
    #[i18n(ja = "起動できませんでした: {0}")]
    GuestBootFailed,

    /// running from frame {0}
    #[i18n(ja = "フレーム {0} から動作中")]
    GuestRunningFrom,

    /// stopped
    #[i18n(ja = "停止しました")]
    GuestStoppedByUser,

    /// stopped: {0}
    #[i18n(ja = "停止しました: {0}")]
    GuestCrashed,

    /// save imported; console rebooted
    #[i18n(ja = "セーブを読み込み、再起動しました")]
    GuestSaveImported,

    /// nothing to undo
    #[i18n(ja = "取り消せるものがありません")]
    NothingToUndo,

    // -- OSD messages: saves and savestates ----------------------------------
    /// import failed: no cart is running - open the ROM first, then import its save
    #[i18n(
        ja = "読み込めません: カートリッジが動いていません。先に ROM を開いてからセーブを読み込んでください"
    )]
    ImportNoCart,

    /// import failed: {0} is empty
    #[i18n(ja = "読み込めません: {0} は空です")]
    ImportEmpty,

    /// import failed: {0}
    #[i18n(ja = "読み込めませんでした: {0}")]
    ImportFailed,

    /// imported {0} ({1}) - console restarted
    #[i18n(ja = "{0} を読み込みました ({1}) — 本体を再起動しました")]
    Imported,

    /// cannot read {0}: {1}
    #[i18n(ja = "{0} を読めません: {1}")]
    CannotRead,

    /// DeSmuME .dsv: its footer was trimmed off
    #[i18n(ja = "DeSmuME の .dsv: 末尾の付加情報を取り除きました")]
    DsvTrimmed,

    /// that looks like a DeSmuME save, but its footer is not one I know
    #[i18n(ja = "DeSmuME のセーブのようですが、末尾の形式が不明です")]
    DsvUnknownFooter,

    /// {0}; this cart reports no backup memory
    #[i18n(ja = "{0}。このカートリッジにはセーブ用メモリがありません")]
    FitNoBackup,

    /// {0} into {1} - padded; the game may not recognise it
    #[i18n(
        ja = "{0} を {1} に読み込み — 足りない分を埋めました。ゲームが認識しない可能性があります"
    )]
    FitPadded,

    /// {0} into {1} - truncated; check it is this cart's save
    #[i18n(
        ja = "{0} を {1} に読み込み — はみ出た分を切り捨てました。このソフトのセーブか確認してください"
    )]
    FitTruncated,

    /// state saved to {0} ({1} MiB)
    #[i18n(ja = "{0} にステートを保存しました ({1} MiB)")]
    StateSaved,

    /// save state failed: {0}
    #[i18n(ja = "ステートセーブに失敗しました: {0}")]
    StateSaveFailed,

    /// state loaded from {0}
    #[i18n(ja = "{0} からステートを読み込みました")]
    StateLoaded,

    /// load state failed: {0}
    #[i18n(ja = "ステートロードに失敗しました: {0}")]
    StateLoadFailed,

    /// state load undone
    #[i18n(ja = "ステートロードを取り消しました")]
    StateLoadUndone,

    /// undo failed: {0}
    #[i18n(ja = "取り消せませんでした: {0}")]
    UndoFailed,

    /// stop report written to {0}
    #[i18n(ja = "停止レポートを {0} に書き出しました")]
    StopReportWritten,

    // -- OSD messages: cheats -----------------------------------------------------
    /// no code selected
    #[i18n(ja = "コードが選ばれていません")]
    CheatNoneSelected,

    /// not a 32-bit hex word: {0}
    #[i18n(ja = "32 ビットの 16 進数ではありません: {0}")]
    CheatBadWord,

    /// that code is no longer in the list
    #[i18n(ja = "そのコードはもう一覧にありません")]
    CheatGone,

    /// saved, but that code has no words in it
    #[i18n(ja = "保存しましたが、そのコードは空です")]
    CheatSavedEmpty,

    /// saved, but that code has an odd number of words
    #[i18n(ja = "保存しましたが、そのコードのワード数が奇数です")]
    CheatSavedOdd,

    /// removed {0}
    #[i18n(ja = "{0} を削除しました")]
    CheatRemoved,

    /// cheats written to {0}
    #[i18n(ja = "チートを {0} に書き出しました")]
    CheatsWritten,

    /// {0} codes read from {1}
    #[i18n(ja = "{1} から {0} 件のコードを読み込みました")]
    CheatsRead,

    /// New cheat
    #[i18n(ja = "新しいチート")]
    NewCheat,

    /// Unnamed
    #[i18n(ja = "名前なし")]
    UnnamedCheat,

    // -- OSD messages: files and folders --------------------------------------
    /// cannot open a file dialog: {0}
    #[i18n(ja = "ファイル選択ダイアログを開けません: {0}")]
    CannotOpenDialog,

    /// a file dialog is already open
    #[i18n(ja = "ファイル選択ダイアログがすでに開いています")]
    DialogAlreadyOpen,

    /// cannot create {0}: {1}
    #[i18n(ja = "{0} を作成できません: {1}")]
    CannotCreate,

    /// opened {0}
    #[i18n(ja = "{0} を開きました")]
    Opened,

    /// cannot open {0}: {1}
    #[i18n(ja = "{0} を開けません: {1}")]
    CannotOpen,

    /// RAM search: {0} addresses hold {1}
    #[i18n(ja = "RAM 検索: {1} を持つアドレスが {0} 件あります")]
    RamSearchFound,

    /// RAM search: narrowed {0} to {1}
    #[i18n(ja = "RAM 検索: {0} 件から {1} 件に絞り込みました")]
    RamSearchNarrowed,

    /// Playing on {0}
    #[i18n(ja = "{0} で再生中")]
    AudioPlayingOn,

    /// No audio output: {0}
    #[i18n(ja = "音声を出力できません: {0}")]
    AudioNone,

    /// set to {0}
    #[i18n(ja = "{0} に設定しました")]
    ClockSet,

    // -- File dialogs ---------------------------------------------------------------
    /// Open a Nintendo DS ROM
    #[i18n(ja = "ニンテンドー DS の ROM を開く")]
    PickOpenRom,

    /// Nintendo DS ROM
    #[i18n(ja = "ニンテンドー DS の ROM")]
    FilterDsRom,

    /// Import a save file
    #[i18n(ja = "セーブデータを読み込む")]
    PickImportSave,

    /// save file
    #[i18n(ja = "セーブデータ")]
    FilterSaveFile,

    /// savestate
    #[i18n(ja = "ステート")]
    FilterSavestate,

    /// Save instance 2 state
    #[i18n(ja = "2 台目のステートを保存")]
    PickSaveGuestState,

    /// Load instance 2 state
    #[i18n(ja = "2 台目のステートを読み込む")]
    PickLoadGuestState,

    /// Import a save into instance 2
    #[i18n(ja = "2 台目にセーブデータを読み込む")]
    PickImportGuestSave,

    /// Open melonDS cheats
    #[i18n(ja = "melonDS のチートファイルを開く")]
    PickOpenCheats,

    /// melonDS cheats
    #[i18n(ja = "melonDS のチートファイル")]
    FilterCheats,

    /// Choose a directory
    #[i18n(ja = "フォルダを選択")]
    PickDirectory,

    // -- ROM info -------------------------------------------------------------------
    /// Title
    #[i18n(ja = "タイトル")]
    InfoTitle,

    /// Game code
    #[i18n(ja = "ゲームコード")]
    InfoGameCode,

    /// Maker code
    #[i18n(ja = "メーカーコード")]
    InfoMakerCode,

    /// ROM size
    #[i18n(ja = "ROM サイズ")]
    InfoRomSize,

    /// File
    #[i18n(ja = "ファイル")]
    InfoFile,

    // -- Emu settings / Preferences ---------------------------------------------
    /// Off runs the core as fast as it will go.
    #[i18n(ja = "オフにすると、コアをできる限り速く動かします。")]
    LimitFramerateHint,

    /// Only takes effect at 1.00x: any other speed deliberately outruns the \
    /// sound card, so pacing against it would cancel the speed setting out.
    #[i18n(
        ja = "1.00x のときだけ効きます。ほかの速度はわざとサウンドカードより速く (遅く) 動かすので、\
                 音声に合わせると速度設定が打ち消されてしまいます。"
    )]
    AudioSyncSpeedHint,

    /// Pace emulation against the sound card instead of the clock.
    #[i18n(ja = "時計ではなくサウンドカードに合わせてエミュレーションの速さを調整します。")]
    AudioSyncHint,

    /// Console: DS, direct boot, FreeBIOS + generated firmware.
    #[i18n(ja = "本体: DS、ダイレクトブート、FreeBIOS + 生成したファームウェア。")]
    ConsoleKind,

    /// The shim offers no other boot mode, so there is nothing else to pick.
    #[i18n(ja = "shim がほかの起動方法を提供していないため、選べるものはこれだけです。")]
    NoOtherBootMode,

    /// Microphone: white noise
    #[i18n(ja = "マイク: ホワイトノイズ")]
    MicWhiteNoise,

    /// The only mic input this build has; carts wanting a breath hear static.
    #[i18n(ja = "このビルドにあるマイク入力はこれだけです。息を吹きかける操作には雑音が届きます。")]
    MicWhiteNoiseHint,

    /// Emulation speed
    #[i18n(ja = "エミュレーション速度")]
    EmulationSpeed,

    /// Speed
    #[i18n(ja = "速度")]
    Speed,

    /// Held at 1.00x: a second console or a LAN link is running, and both \
    /// consoles have to agree about time.
    #[i18n(
        ja = "1.00x に固定中: 2 台目か LAN 接続が動いていて、2 台の時間を揃える必要があります。"
    )]
    SpeedHeld,

    /// Clicking the pad's left stick steps through the same list. The console \
    /// is not reclocked -- what changes is how many emulated frames one repaint \
    /// may run, so a frame at 2x is the same frame it would have been at 1x.
    #[i18n(
        ja = "パッドの左スティック押し込みでも同じ一覧を順に切り替えます。本体のクロックは変えません。\
                 変わるのは 1 回の描画で進めるフレーム数なので、2 倍速のフレームも 1 倍速と同じ内容です。"
    )]
    SpeedExplained,

    /// Pause when the window loses focus
    #[i18n(ja = "ウィンドウが非アクティブになったら一時停止する")]
    PauseUnfocused,

    /// Ask before quitting with a cart running
    #[i18n(ja = "カートリッジ動作中に終了するときは確認する")]
    ConfirmQuit,

    /// Settings are written to:
    #[i18n(ja = "設定の保存先:")]
    SettingsWrittenTo,

    // -- Video settings -----------------------------------------------------------
    /// 3D renderer
    #[i18n(ja = "3D レンダラ")]
    Renderer3d,

    /// No OpenGL context: melon_egui could not bind the GL entry points (or its \
    /// blitter's shader would not build), so only the software renderer can draw.
    #[i18n(ja = "OpenGL コンテキストがありません: melon_egui が GL の関数を取得できなかった \
                 (または転送用シェーダーをビルドできなかった) ため、ソフトウェアレンダラしか使えません。")]
    NoGl,

    /// This context is not OpenGL 4.3, which the compute-shader renderer needs.
    #[i18n(
        ja = "この環境は OpenGL 4.3 ではありません。コンピュートシェーダーレンダラには 4.3 が必要です。"
    )]
    NoCompute,

    /// Threaded software renderer
    #[i18n(ja = "ソフトウェアレンダラをマルチスレッドで動かす")]
    ThreadedSoftware,

    /// Rasterise 3D on worker threads. Faster where there are cores to spare; \
    /// melonDS ships it off, so this does too.
    #[i18n(ja = "3D の描画を別スレッドで行います。CPU コアに余裕があれば速くなります。\
                 melonDS の初期設定がオフなので、こちらもオフです。")]
    ThreadedSoftwareHint,

    /// OpenGL options
    #[i18n(ja = "OpenGL の設定")]
    OpenGlOptions,

    /// Internal resolution
    #[i18n(ja = "内部解像度")]
    InternalResolution,

    /// Select an OpenGL renderer above to change the internal resolution.
    #[i18n(ja = "内部解像度を変えるには、上で OpenGL レンダラを選んでください。")]
    SelectGlForResolution,

    /// This rasterises the 3D geometry itself at the higher resolution, so it \
    /// adds real detail — unlike Display scale below, which magnifies the \
    /// finished picture.
    #[i18n(ja = "3D の形状そのものを高い解像度で描くので、実際に細部が増えます。\
                 できあがった画像を拡大するだけの下の「表示倍率」とは違います。")]
    InternalResolutionExplained,

    /// Better polygons
    #[i18n(ja = "ポリゴン分割を改善")]
    BetterPolygons,

    /// Improved polygon splitting. Closes the seams upscaling opens in some \
    /// geometry, for some speed. Regular OpenGL renderer only.
    #[i18n(
        ja = "ポリゴンの分割を改善し、高解像度化で一部の形状に出る隙間を塞ぎます。少し遅くなります。\
                 通常の OpenGL レンダラ専用です。"
    )]
    BetterPolygonsHint,

    /// High-resolution coordinates
    #[i18n(ja = "高精度の座標")]
    HiresCoordinates,

    /// Keep the extra vertex precision upscaling makes visible instead of \
    /// rounding to the DS's own grid. Compute-shader renderer only.
    #[i18n(ja = "頂点座標を DS 本来の格子に丸めず、高解像度化で見えるようになる精度を保ちます。\
                 コンピュートシェーダーレンダラ専用です。")]
    HiresCoordinatesHint,

    /// GL display
    #[i18n(ja = "GL で表示")]
    GlDisplay,

    /// Always on: this front end composites through egui's OpenGL painter \
    /// whichever renderer the core draws with, so there is nothing to turn off.
    #[i18n(
        ja = "常にオン: このフロントエンドはコアのレンダラに関係なく egui の OpenGL 描画で合成するため、\
                 オフにするものがありません。"
    )]
    GlDisplayHint,

    /// 2D upscaling
    #[i18n(ja = "2D の高解像度化")]
    Upscaling2d,

    /// Factor
    #[i18n(ja = "倍率")]
    Factor,

    /// xBRZ redraws the picture edge by edge, which is what smooths sprites and \
    /// text rather than blurring them. It is the only setting here that improves \
    /// a 2D layer — those are built from tiles at 256x192 whatever the renderer \
    /// does, so the internal resolution above cannot touch them.
    #[i18n(ja = "xBRZ は輪郭ごとに絵を描き直すので、スプライトや文字をぼかさずに滑らかにします。\
                 2D レイヤーを改善できるのはこの設定だけです。2D はレンダラに関係なく 256x192 のタイルから\
                 作られるため、上の内部解像度では変わりません。")]
    XbrzExplained,

    /// Software renderer: filtered on the CPU at {0}x, once per screen per frame.
    #[i18n(ja = "ソフトウェアレンダラ: CPU で {0} 倍に変換します (1 フレームにつき各画面 1 回)。")]
    XbrzSoftwareRoute,

    /// OpenGL renderer: the same filter, at the same 256x192, on the same CPU — \
    /// the 2D content is read back off the GPU at the DS's own size, filtered, \
    /// and shown wherever the picture came from the 2D engine. The 3D never \
    /// makes the trip and keeps every pixel the internal resolution above drew. \
    /// So both settings apply at once and neither is capped by the other.
    #[i18n(ja = "OpenGL レンダラ: 同じフィルタを同じ 256x192 で、同じく CPU で掛けます。\
                 2D の内容を DS 本来の大きさで GPU から読み戻して変換し、2D エンジン由来の部分に表示します。\
                 3D は読み戻さないので、上の内部解像度で描いた画素がそのまま残ります。\
                 つまり両方の設定が同時に効き、互いに制限しません。")]
    XbrzGlRoute,

    /// Costs one {0} readback per screen per frame, and {0} pixels through xBRZ \
    /// — the same work the software renderer already does.
    #[i18n(
        ja = "1 フレームにつき各画面で {0} の読み戻しが 1 回と、{0} 画素分の xBRZ が掛かります。\
                 ソフトウェアレンダラがもともと行っている処理と同じ量です。"
    )]
    XbrzGlCost,

    /// Display scale
    #[i18n(ja = "表示倍率")]
    DisplayScale,

    /// Draw at a fixed scale
    #[i18n(ja = "固定の倍率で描く")]
    FixedScale,

    /// Scale
    #[i18n(ja = "倍率")]
    Scale,

    /// Each screen drawn at {0} x {1} pixels.
    #[i18n(ja = "各画面を {0} x {1} ピクセルで描きます。")]
    EachScreenDrawnAt,

    /// Larger than the window simply crops; the layout still centres it.
    #[i18n(ja = "ウィンドウより大きい部分は切れます。配置は中央揃えのままです。")]
    LargerCrops,

    /// Fitting to the window (use Screen filtering to choose how it is sampled).
    #[i18n(ja = "ウィンドウに合わせています (補間方法は「画面を滑らかにする」で選びます)。")]
    FittingWindow,

    /// Display
    #[i18n(ja = "表示")]
    Display,

    /// Takes effect the next time melon_egui starts: the surface's present mode \
    /// is fixed when the window is created.
    #[i18n(
        ja = "次に melon_egui を起動したときに反映されます。表示方式はウィンドウ作成時に決まるためです。"
    )]
    VsyncHint,

    /// Smooth the picture when scaled, instead of square pixels.
    #[i18n(ja = "拡大したときに、四角いドットのままではなく滑らかに表示します。")]
    ScreenFilteringHint,

    /// Compositing
    #[i18n(ja = "画面の合成")]
    Compositing,

    /// Render frames
    #[i18n(ja = "フレームを描画する")]
    RenderFrames,

    /// Off, the console keeps running but stops composing a picture. Emulation \
    /// is unaffected -- melonDS documents it as bit-identical either way -- so \
    /// this only makes the window go still.
    #[i18n(
        ja = "オフにすると本体は動き続けますが、画面を作らなくなります。エミュレーション自体は変わらない \
                 (melonDS はどちらでもビット単位で同一と説明しています) ので、ウィンドウが止まって見えるだけです。"
    )]
    RenderFramesHint,

    /// Skip screens the layout hides
    #[i18n(ja = "配置で隠れる画面は描かない")]
    SkipHiddenScreens,

    /// In the Top only / Bottom only sizings, tell the core not to compose the \
    /// screen nobody is looking at. Most of the 2D renderer's work, saved.
    #[i18n(ja = "「上画面のみ」「下画面のみ」のとき、誰も見ていない画面をコアに作らせません。\
                 2D 描画の処理の大半を省けます。")]
    SkipHiddenScreensHint,

    // -- Audio settings -----------------------------------------------------------
    /// Volume
    #[i18n(ja = "音量")]
    Volume,

    /// Above 100% is a boost; loud passages may clip.
    #[i18n(ja = "100% を超えると増幅になります。大きな音は割れることがあります。")]
    VolumeBoost,

    /// Source rate: {0} Hz (the core's SPU output; fixed by the bindings)
    #[i18n(ja = "元のサンプリングレート: {0} Hz (コアの SPU 出力。バインディングで固定)")]
    SourceRate,

    // -- Input and hotkeys --------------------------------------------------------
    /// Keyboard
    #[i18n(ja = "キーボード")]
    Keyboard,

    /// Controller
    #[i18n(ja = "コントローラー")]
    Controller,

    /// press...
    #[i18n(ja = "入力待ち...")]
    PressKey,

    /// Click to rebind, right-click to clear.
    #[i18n(ja = "クリックで割り当て直し、右クリックで解除します。")]
    RebindHint,

    /// Touch
    #[i18n(ja = "タッチ")]
    Touch,

    /// click the bottom screen
    #[i18n(ja = "下画面をクリック")]
    TouchHow,

    /// Waiting for a press. Escape cancels.
    #[i18n(ja = "入力を待っています。Esc で取り消します。")]
    WaitingForPress,

    /// Reset to melonDS's defaults
    #[i18n(ja = "melonDS の初期設定に戻す")]
    ResetBindings,

    /// Controllers
    #[i18n(ja = "コントローラー")]
    Controllers,

    /// None connected. A pad is picked up as soon as it is plugged in.
    #[i18n(ja = "接続されていません。つなぐとすぐに認識されます。")]
    NoControllers,

    /// The left stick always steers, whatever the D-pad is bound to: a stick is \
    /// an axis and the D-pad is four switches, and many pads report their D-pad \
    /// as that axis anyway. Pad and keyboard are merged, so either works at any \
    /// time.
    #[i18n(
        ja = "十字キーの割り当てに関係なく、左スティックでも常に操作できます。スティックは軸、十字キーは \
                 4 つのスイッチで、多くのパッドは十字キーをその軸として報告するためです。\
                 パッドとキーボードの入力は合成されるので、どちらもいつでも使えます。"
    )]
    StickExplained,

    // -- Window titles ----------------------------------------------------------
    /// Cheat codes
    #[i18n(ja = "チートコード")]
    CheatCodesTitle,

    /// Why the console stopped
    #[i18n(ja = "本体が停止した理由")]
    CrashTitle,

    /// Preferences
    #[i18n(ja = "環境設定")]
    PreferencesTitle,

    /// About melon_egui
    #[i18n(ja = "melon_egui について")]
    AboutTitle,

    /// melon_egui - instance 2
    #[i18n(ja = "melon_egui - 2 台目")]
    Instance2Title,

    /// melon_egui - second view
    #[i18n(ja = "melon_egui - 2 つ目の表示")]
    SecondViewTitle,

    // -- Power / Date and time / ROM info / Stop report ---------------------------
    /// No cart running.
    #[i18n(ja = "カートリッジが動いていません。")]
    NoCartRunning,

    /// Lid closed
    #[i18n(ja = "ふたを閉じる")]
    LidClosed,

    /// Closing the lid raises the lid IRQ, which is how a cart is told to sleep.
    #[i18n(ja = "ふたを閉じると割り込みが発生し、ソフトにスリープを知らせます。")]
    LidExplained,

    /// Battery level
    #[i18n(ja = "バッテリー残量")]
    BatteryLevel,

    /// Okay
    #[i18n(ja = "十分")]
    BatteryOkay,

    /// Low
    #[i18n(ja = "少ない")]
    BatteryLow,

    /// What SPI's power-management chip reports; "Low" is what a cart's \
    /// low-battery warning reads.
    #[i18n(
        ja = "電源管理チップが報告する値です。「少ない」にすると、ソフトのバッテリー残量警告が反応します。"
    )]
    BatteryExplained,

    /// Nothing has stopped this session.
    #[i18n(ja = "このセッションでは何も停止していません。")]
    NothingStopped,

    /// Copy
    #[i18n(ja = "コピー")]
    Copy,

    /// Also written to {0}
    #[i18n(ja = "{0} にも書き出しています")]
    AlsoWrittenTo,

    /// The report below is a diagnostic for bug reports, and stays in English.
    #[i18n(ja = "以下のレポートは不具合報告用の診断情報のため、英語のままです。")]
    ReportStaysEnglish,

    /// The DS clock is set at boot and runs on emulated time from there.
    #[i18n(ja = "DS の時計は起動時に設定され、その後はエミュレーション上の時間で進みます。")]
    ClockExplained,

    /// Year
    #[i18n(ja = "年")]
    Year,

    /// Month
    #[i18n(ja = "月")]
    Month,

    /// Day
    #[i18n(ja = "日")]
    Day,

    /// Hour
    #[i18n(ja = "時")]
    Hour,

    /// Minute
    #[i18n(ja = "分")]
    Minute,

    /// Second
    #[i18n(ja = "秒")]
    Second,

    /// Apply
    #[i18n(ja = "適用")]
    Apply,

    /// Now (UTC)
    #[i18n(ja = "現在時刻 (UTC)")]
    NowUtc,

    // -- Interface settings / About ---------------------------------------------
    /// egui's own fonts are Latin-only, so a system font is borrowed for \
    /// Japanese, Chinese and Korean. Set MELON_EGUI_FONT to a .ttf/.otf/.ttc to \
    /// choose a different one.
    #[i18n(
        ja = "egui 標準のフォントは欧文のみのため、日本語・中国語・韓国語にはシステムのフォントを借りています。\
                 別のフォントを使うには MELON_EGUI_FONT に .ttf/.otf/.ttc を指定してください。"
    )]
    FontHint,

    /// Dark theme
    #[i18n(ja = "ダークテーマ")]
    DarkTheme,

    /// UI scale
    #[i18n(ja = "UI の拡大率")]
    UiScale,

    /// Apply UI scale
    #[i18n(ja = "UI の拡大率を適用")]
    ApplyUiScale,

    /// Write translation templates
    #[i18n(ja = "翻訳ファイルのひな形を書き出す")]
    WriteTemplates,

    /// Writes instances/translation.<lang>.json for every language. Edit one to \
    /// change a wording without rebuilding; it is read over the built-in text at \
    /// startup.
    #[i18n(ja = "すべての言語について instances/translation.<lang>.json を書き出します。\
                 編集すると再ビルドせずに文言を変えられます。起動時に内蔵の文言の上から読み込まれます。")]
    WriteTemplatesHint,

    /// wrote {0}
    #[i18n(ja = "{0} を書き出しました")]
    Wrote,

    /// {0} of {1} strings are translated; the rest show in English.
    #[i18n(ja = "{1} 件中 {0} 件の文言が翻訳済みです。残りは英語で表示されます。")]
    TranslationCoverage,

    /// version {0}
    #[i18n(ja = "バージョン {0}")]
    Version,

    /// An egui front end for the melonDS core, through the melonds-rs bindings. \
    /// Built as a reference picture to compare lunaris against.
    #[i18n(ja = "melonds-rs バインディング経由で melonDS のコアを動かす egui フロントエンドです。\
                 lunaris と見比べるための基準として作られています。")]
    AboutText,

    /// GPL-3.0-or-later, as is the melonDS core it embeds.
    #[i18n(ja = "ライセンスは、組み込んでいる melonDS のコアと同じ GPL-3.0-or-later です。")]
    License,

    // -- Path settings --------------------------------------------------------------
    /// Empty means "beside the ROM". By default each console keeps its own files \
    /// under instances/instanceN/, which is where lunaris keeps its.
    #[i18n(ja = "空欄は「ROM と同じ場所」を意味します。初期設定では各本体が instances/instanceN/ \
                 の下に自分のファイルを置きます (lunaris と同じ場所です)。")]
    PathsExplained,

    /// (beside the ROM)
    #[i18n(ja = "(ROM と同じ場所)")]
    BesideRom,

    /// Reset
    #[i18n(ja = "元に戻す")]
    PathReset,

    /// Save files
    #[i18n(ja = "セーブデータ")]
    SaveFiles,

    /// Savestates
    #[i18n(ja = "ステート")]
    Savestates,

    /// Choose save files...
    #[i18n(ja = "セーブデータのフォルダを選ぶ...")]
    ChooseSaveFolder,

    /// Choose savestates...
    #[i18n(ja = "ステートのフォルダを選ぶ...")]
    ChooseStateFolder,

    /// Per-instance directories
    #[i18n(ja = "本体ごとのフォルダ")]
    PerInstanceDirs,

    /// Each console gets saves/, states/, cheats/ and its own settings.json. \
    /// Instance 2 is the console System ▸ Multiplayer ▸ Launch new instance opens.
    #[i18n(ja = "各本体に saves/、states/、cheats/ と専用の settings.json があります。\
                 2 台目は「システム ▸ 通信プレイ ▸ 2 台目を起動」で開く本体です。")]
    PerInstanceExplained,

    /// Open the instances folder
    #[i18n(ja = "instances フォルダを開く")]
    OpenInstancesFolder,

    /// These take effect for the next cart loaded.
    #[i18n(ja = "次に読み込むカートリッジから反映されます。")]
    PathsNextCart,

    // -- RAM search ---------------------------------------------------------------------
    /// Value:
    #[i18n(ja = "値:")]
    SearchValue,

    /// First scan
    #[i18n(ja = "最初の検索")]
    FirstScan,

    /// Narrow
    #[i18n(ja = "絞り込み")]
    Narrow,

    /// not a number
    #[i18n(ja = "数値ではありません")]
    NotANumber,

    /// {0} matching addresses
    #[i18n(ja = "一致したアドレス: {0} 件")]
    MatchingAddresses,

    /// (first 200 shown — narrow the search to see fewer)
    #[i18n(ja = "(最初の 200 件を表示中 — 絞り込むと件数が減ります)")]
    First200Shown,

    /// 8-bit
    #[i18n(ja = "8 ビット")]
    Bits8,

    /// 16-bit
    #[i18n(ja = "16 ビット")]
    Bits16,

    /// 32-bit
    #[i18n(ja = "32 ビット")]
    Bits32,

    // -- Cheat codes ----------------------------------------------------------------------
    /// Off hands the console an empty list, so the codes cost nothing at all \
    /// rather than merely doing nothing.
    #[i18n(
        ja = "オフにすると本体には空の一覧を渡すので、コードは何もしないだけでなく処理の負荷もなくなります。"
    )]
    EnableCheatsHint,

    /// Open .mch...
    #[i18n(ja = ".mch を開く...")]
    OpenMch,

    /// File: {0}
    #[i18n(ja = "ファイル: {0}")]
    CheatFile,

    /// No cart running; codes load with one.
    #[i18n(ja = "カートリッジが動いていません。コードはカートリッジと一緒に読み込まれます。")]
    CheatsNoCart,

    /// Add cheat
    #[i18n(ja = "チートを追加")]
    AddCheat,

    /// Adds an empty, disabled code and opens it in the editor.
    #[i18n(ja = "空の無効なコードを追加して、エディタで開きます。")]
    AddCheatHint,

    /// Delete
    #[i18n(ja = "削除")]
    Delete,

    /// Removes the selected code, and writes the list to the cart's .mch.
    #[i18n(ja = "選んだコードを削除し、一覧をカートリッジの .mch に書き出します。")]
    DeleteCheatHint,

    /// Save
    #[i18n(ja = "保存")]
    Save,

    /// Writes the editor back into the selected code, and the whole list to the \
    /// cart's .mch.
    #[i18n(
        ja = "エディタの内容を選んだコードに反映し、一覧全体をカートリッジの .mch に書き出します。"
    )]
    SaveCheatHint,

    /// Available cheats:
    #[i18n(ja = "使えるチート:")]
    AvailableCheats,

    /// Name
    #[i18n(ja = "名前")]
    CheatName,

    /// Type
    #[i18n(ja = "種類")]
    CheatType,

    /// This code has an odd number of words.
    #[i18n(ja = "このコードはワード数が奇数です。")]
    CheatOddWords,

    /// No codes. Add one, or read a melonDS .mch file.
    #[i18n(ja = "コードがありません。追加するか、melonDS の .mch ファイルを読み込んでください。")]
    NoCheats,

    /// Click a row to edit it. Hold it for a moment to pick it up, then drop it \
    /// on another row to reorder — the new order is saved as it lands.
    #[i18n(
        ja = "行をクリックすると編集できます。少し押し続けてつかみ、別の行に落とすと並べ替えられます \
                 (落とした時点で新しい順番が保存されます)。"
    )]
    CheatListHint,

    /// Name:
    #[i18n(ja = "名前:")]
    CheatNameLabel,

    /// Notes:
    #[i18n(ja = "メモ:")]
    CheatNotesLabel,

    /// What this code does, and where it came from.
    #[i18n(ja = "このコードの効果と入手元。")]
    CheatNotesHint,

    /// Code:
    #[i18n(ja = "コード:")]
    CheatCodeLabel,

    /// Select a code on the left, or press Add cheat.
    #[i18n(ja = "左でコードを選ぶか、「チートを追加」を押してください。")]
    SelectCheatHint,

    // -- Wireless status --------------------------------------------------------------
    /// Status
    #[i18n(ja = "状態")]
    Status,

    /// Second console: running, frame {0}
    #[i18n(ja = "2 台目: 動作中 (フレーム {0})")]
    SecondConsoleRunning,

    /// No second console. System ▸ Multiplayer ▸ Launch new instance.
    #[i18n(ja = "2 台目はありません。システム ▸ 通信プレイ ▸ 2 台目を起動 で起動できます。")]
    NoSecondConsoleHint,

    /// No console is on the air yet. A cart only joins when it opens its wireless \
    /// menu, so this stays empty until then.
    #[i18n(
        ja = "まだ無線に参加している本体はありません。ソフトが通信メニューを開いたときに参加するので、\
                 それまでは空のままです。"
    )]
    NobodyOnAir,

    /// {0} console(s) on the air, {1} frames exchanged, but no CMD frame has been sent.
    #[i18n(
        ja = "{0} 台が無線に参加し、{1} フレームをやり取りしましたが、CMD フレームはまだ送られていません。"
    )]
    OnAirNoCmd,

    /// Beacons and the association handshake are ordinary frames; local play \
    /// only begins when the host starts an MP round with a CMD. This is the exact \
    /// point lunaris does not get past.
    #[i18n(
        ja = "ビーコンと接続手続きは通常のフレームです。ローカル通信プレイは、親機が CMD で MP ラウンドを\
                 始めて初めて開始されます。lunaris が越えられないのはまさにこの地点です。"
    )]
    OnAirNoCmdExplained,

    /// MP rounds are running: {0} CMD, {1} replies, {2} ACK.
    #[i18n(ja = "MP ラウンドが動いています: CMD {0}、応答 {1}、ACK {2}。")]
    RoundsRunning,

    /// The host is asking but no client has answered.
    #[i18n(ja = "親機は呼びかけていますが、子機が応答していません。")]
    NoClientAnswered,

    /// Per console
    #[i18n(ja = "本体ごとの集計")]
    PerConsole,

    /// on air
    #[i18n(ja = "参加中")]
    ColOnAir,

    /// wifi clock
    #[i18n(ja = "無線クロック")]
    ColWifiClock,

    /// sent pkt
    #[i18n(ja = "送信パケット")]
    ColSentPkt,

    /// reply
    #[i18n(ja = "応答")]
    ColReply,

    /// recv pkt
    #[i18n(ja = "受信パケット")]
    ColRecvPkt,

    /// recv CMD
    #[i18n(ja = "受信 CMD")]
    ColRecvCmd,

    /// recv reply
    #[i18n(ja = "受信 応答")]
    ColRecvReply,

    /// stale
    #[i18n(ja = "期限切れ")]
    ColStale,

    /// AID mask
    #[i18n(ja = "AID マスク")]
    ColAidMask,

    /// yes
    #[i18n(ja = "はい")]
    Yes,

    /// no
    #[i18n(ja = "いいえ")]
    No,

    /// "stale" counts replies discarded for arriving outside the host's round, \
    /// and "AID mask" is what the last reply collection returned - a host asking \
    /// and getting 0000 is a host nobody answered.
    #[i18n(
        ja = "「期限切れ」は親機のラウンド外に届いて破棄された応答の数、「AID マスク」は直前の応答収集の結果です。\
                 呼びかけて 0000 が返る親機は、誰にも応答されていない親機です。"
    )]
    PerConsoleExplained,

    /// Traffic
    #[i18n(ja = "通信ログ")]
    Traffic,

    /// packet
    #[i18n(ja = "パケット")]
    KindPacket,

    /// inst {0}  t={1} {2} {3} bytes
    #[i18n(ja = "本体 {0}  t={1} {2} {3} バイト")]
    TrafficLine,

    /// {0} ms (jitter {1} ms)
    #[i18n(ja = "{0} ms (ゆらぎ {1} ms)")]
    RoundTripValue,

    /// {0}%  ({1} of {2})
    #[i18n(ja = "{0}%  ({2} 回中 {1} 回)")]
    RoundsValue,

    /// no round yet
    #[i18n(ja = "まだラウンドがありません")]
    NoRoundYet,

    /// Datagrams
    #[i18n(ja = "データグラム")]
    Datagrams,

    /// {0} sent, {1} received, {2} duplicates discarded
    #[i18n(ja = "送信 {0}、受信 {1}、重複として破棄 {2}")]
    DatagramsValue,

    /// Wireless frames
    #[i18n(ja = "無線フレーム")]
    WirelessFrames,

    /// {0} sent, {1} received
    #[i18n(ja = "送信 {0}、受信 {1}")]
    SentReceived,

    /// Stale replies
    #[i18n(ja = "期限切れの応答")]
    StaleReplies,

    /// Wireless
    #[i18n(ja = "無線")]
    Wireless,

    /// on (the cart has opened its wireless menu)
    #[i18n(ja = "オン (ソフトが通信メニューを開きました)")]
    WirelessOn,

    /// off (the cart has not started multiplayer yet)
    #[i18n(ja = "オフ (ソフトはまだ通信を始めていません)")]
    WirelessOff,

    /// Applies to the next LAN connection. The reply budget is measured from the \
    /// link itself; these only bound and shape it.
    #[i18n(
        ja = "次の LAN 接続から反映されます。応答待ち時間は回線から実測し、ここではその範囲と形を決めるだけです。"
    )]
    VpnAppliesNext,

    /// The worst link that will still be played over. Past this a game's own \
    /// timeouts give up anyway.
    #[i18n(
        ja = "通信プレイを続ける最悪の回線の目安です。これを超えるとゲーム自身のタイムアウトで結局切れます。"
    )]
    MaxBudgetHint,

    /// Multiples of the measured jitter added on top of the round trip.
    #[i18n(ja = "往復遅延に上乗せする、実測したゆらぎの倍数です。")]
    JitterFactorHint,

    /// A lost reply is a lost round, and a lost round is a communication error. \
    /// Sending two copies costs bandwidth and removes most single-packet losses.
    #[i18n(ja = "応答を 1 つ失うとラウンドを失い、ラウンドを失うと通信エラーになります。\
                 2 回送ると帯域を使う代わりに、1 パケットだけの消失はほとんど防げます。")]
    ReplyCopiesHint,

    /// How long ordinary frames (beacons, association) may wait to share one \
    /// datagram. 0 sends each on its own. Rounds are never batched: a round has to \
    /// finish inside its own emulated frame.
    #[i18n(
        ja = "通常のフレーム (ビーコンや接続手続き) を 1 つのデータグラムにまとめるために待てる時間です。\
                 0 にすると 1 つずつ送ります。ラウンドは 1 エミュレーションフレーム内で終える必要があるため、まとめません。"
    )]
    BatchWindowHint,

    /// Run the console at the rate the link can sustain instead of dropping the \
    /// rounds it cannot service.
    #[i18n(
        ja = "回線がこなせないラウンドを落とす代わりに、回線が支えられる速さで本体を動かします。"
    )]
    PaceToLinkHint,

    // -- OSD ----------------------------------------------------------------------------
    /// [paused]
    #[i18n(ja = "[一時停止中]")]
    OsdPaused,

    /// [rendering off]
    #[i18n(ja = "[描画オフ]")]
    OsdRenderingOff,

    // -- shared words --------------------------------------------------------
    /// DS slot
    #[i18n(ja = "DS スロット")]
    DsSlot,

    /// GBA slot
    #[i18n(ja = "GBA スロット")]
    GbaSlot,

    /// (none)
    #[i18n(ja = "(なし)")]
    None,

    /// Insert ROM cart...
    #[i18n(ja = "ROM カートリッジを挿入...")]
    InsertRomCart,

    /// Insert add-on cart
    #[i18n(ja = "拡張カートリッジを挿入")]
    InsertAddonCart,

    /// Not reachable: the melonds-rs bindings expose no FFI entry point for this.
    #[i18n(ja = "利用できません: melonds-rs のバインディングに対応する FFI 入口がありません。")]
    UnavailableBindings,

    // NOTE: Using `skip_serializing` causes an error when attempting to serialize `Invalid`.
    /// Invalid key comes here when deserializing unknown strings.
    #[i18n(ja = "不明なキー")]
    #[serde(other)]
    Invalid,
}
