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
