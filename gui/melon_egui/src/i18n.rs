//! The UI's text, in English and Japanese.
//!
//! # How a string gets translated
//!
//! Every translatable string is a variant of [`I18nKey`]. Its **doc comment is
//! the English text** and its `#[i18n(ja = "...")]` attribute is the Japanese;
//! `i18n_derive` turns both into `const fn`s, so neither costs an allocation
//! and neither can drift out of sync with the key list. A variant with no `ja`
//! renders its English, which keeps a newly added key merely untranslated
//! rather than broken — and [`I18nKey::UNTRANSLATED`] says exactly which those
//! are, so the gap is a number rather than something a reader has to notice.
//!
//! # Overriding a translation without rebuilding
//!
//! [`I18nMap::load_with_fallback`] reads `instances/translation.<lang>.json`
//! over the built-in table, so a user who dislikes a wording can change it.
//! [`I18nMap::save_template`] writes the current language out in full as a
//! starting point. A key missing from the file keeps its built-in text, and an
//! unrecognised key deserialises to [`I18nKey::Invalid`] and is ignored — a
//! translation file from another version must never stop the emulator starting.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use snafu::ResultExt as _;

/// Which translation the UI is drawn in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// The language the strings are written in, and the fallback for every key
    /// no other language has translated.
    #[default]
    English,
    Japanese,
}

impl Language {
    /// Every language, for a settings dropdown.
    pub const ALL: &'static [Self] = &[Self::English, Self::Japanese];

    /// The name of the language **in that language**, which is how a language
    /// picker has to be labelled: someone looking for Japanese is looking for
    /// 日本語, not for the word "Japanese" they may not read.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Japanese => "日本語",
        }
    }

    /// The suffix of this language's override file, `translation.<code>.json`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "eng",
            Self::Japanese => "jpn",
        }
    }

    /// The built-in text for `key` in this language.
    #[must_use]
    pub const fn text(self, key: I18nKey) -> &'static str {
        match self {
            Self::English => key.default_eng(),
            Self::Japanese => key.default_jpn(),
        }
    }
}

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

/// The text the UI draws, for one language.
///
/// Built from the language's built-in table and then overlaid with whatever
/// `instances/translation.<code>.json` holds, so the map is always complete:
/// [`I18nMap::t`] never has to fall back at draw time.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct I18nMap {
    /// Which language this map is, so a settings pane can show it and
    /// [`I18nMap::save_template`] knows what to write.
    #[serde(default)]
    language: Language,
    #[serde(flatten)]
    strings: IndexMap<I18nKey, Cow<'static, str>>,
}

impl Default for I18nMap {
    /// English, which is what the front end draws before any setting has been
    /// read.
    fn default() -> Self {
        Self::built_in(Language::English)
    }
}

impl I18nMap {
    /// The complete built-in table for `language`, with no file overlaid.
    #[must_use]
    pub fn built_in(language: Language) -> Self {
        let strings =
            I18nKey::ALL.iter().map(|key| (*key, Cow::Borrowed(language.text(*key)))).collect();
        Self { language, strings }
    }

    /// Which language this map holds.
    #[must_use]
    pub const fn language(&self) -> Language {
        self.language
    }

    /// Translate `key`, falling back to the built-in English for a key the map
    /// somehow lacks.
    #[must_use]
    pub fn t(&self, key: I18nKey) -> &str {
        self.strings.get(&key).map_or_else(|| key.default_eng(), AsRef::as_ref)
    }

    /// Translate `key` into an owned string, for the many call sites that
    /// build a label with [`format!`] and cannot hold a borrow of `self` while
    /// they also borrow it mutably.
    #[must_use]
    pub fn s(&self, key: I18nKey) -> String {
        self.t(key).to_owned()
    }

    /// Load `path` as a translation file.
    ///
    /// # Errors
    /// If the file cannot be read or is not the JSON object this writes.
    #[inline]
    pub fn load<P>(path: P) -> Result<Self, Error>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).with_context(|_| ReadFileSnafu { path })?;
        serde_json::from_str(&content).with_context(|_| ParseJsonSnafu { path })
    }

    /// The table for `language`, with its override file applied if there is one.
    ///
    /// A missing file is not an error: it is the ordinary case, and the
    /// built-in table is already complete. A file that is present but
    /// unreadable *is* reported, because a user who wrote one wants to know it
    /// was ignored.
    ///
    /// # Errors
    /// If the override file exists but cannot be read or parsed.
    pub fn load_with_fallback(language: Language) -> Result<Self, Error> {
        let mut map = Self::built_in(language);
        let path = Self::i18n_path(language);
        if !path.exists() {
            log::info!("{} does not exist; using the built-in text.", path.display());
            return Ok(map);
        }
        let overlay = Self::load(&path)?;
        // Overlaid key by key rather than replacing the map: a file that only
        // changes one wording must not blank out everything it omits.
        for (key, text) in overlay.strings {
            if key != I18nKey::Invalid {
                map.strings.insert(key, text);
            }
        }
        Ok(map)
    }

    /// Where `language`'s override file lives.
    #[must_use]
    pub fn i18n_path(language: Language) -> PathBuf {
        // Shared between instances rather than per-instance: a translation is a
        // property of the person reading the screen, not of one console.
        PathBuf::from(crate::i18n::INSTANCES_DIR)
            .join(format!("translation.{}.json", language.code()))
    }

    /// Write this map out as a starting point for a hand translation.
    ///
    /// # Errors
    /// If the directory cannot be made, or the file cannot be written.
    pub fn save_template(&self) -> Result<PathBuf, Error> {
        let path = Self::i18n_path(self.language);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|_| WriteFileSnafu { path: path.clone() })?;
        }
        let text = serde_json::to_string_pretty(self)
            .with_context(|_| SerializeJsonSnafu { path: path.clone() })?;
        std::fs::write(&path, text).with_context(|_| WriteFileSnafu { path: path.clone() })?;
        Ok(path)
    }
}

/// Every language's strings, read once.
///
/// # Why they are all loaded up front
///
/// The two consoles have settings of their own, and may be set to different
/// languages. The front end applies each console's settings as it draws that
/// console's window, so a language switch happens *twice per repaint* whenever
/// the second window is open. Reading a file and rebuilding a map that often
/// would be absurd; reading both files once and indexing is not.
#[derive(Debug)]
pub struct Translations([I18nMap; Language::ALL.len()]);

impl Default for Translations {
    fn default() -> Self {
        Self::load()
    }
}

impl Translations {
    /// Read every language, applying each one's override file if it has one.
    ///
    /// A file that cannot be read is reported and skipped, leaving that
    /// language on its built-in text — a broken translation must never stop the
    /// emulator starting.
    #[must_use]
    pub fn load() -> Self {
        Self(std::array::from_fn(|index| {
            let language = Language::ALL[index];
            I18nMap::load_with_fallback(language).unwrap_or_else(|error| {
                eprintln!("melon_egui: {error}; using the built-in text");
                I18nMap::built_in(language)
            })
        }))
    }

    /// The strings for one language.
    #[must_use]
    pub fn get(&self, language: Language) -> &I18nMap {
        // `Language::ALL` is in declaration order, so the discriminant is the
        // index; the search keeps that from being a silent assumption.
        let index = Language::ALL.iter().position(|it| *it == language).unwrap_or(0);
        let map = &self.0[index];
        debug_assert_eq!(map.language(), language, "Translations is indexed out of order");
        map
    }
}

/// Where the instance tree lives, repeated here rather than taken from
/// `crate::config` because that module needs the emulator core linked and this
/// one does not.
const INSTANCES_DIR: &str = "./instances";

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("Failed to read file: {}", path.display()))]
    ReadFile { path: PathBuf, source: std::io::Error },

    #[snafu(display("Failed to write file: {}", path.display()))]
    WriteFile { path: PathBuf, source: std::io::Error },

    #[snafu(display("Failed to parse json: {}", path.display()))]
    ParseJson { path: PathBuf, source: serde_json::Error },

    #[snafu(display("Failed to serialize json: {}", path.display()))]
    SerializeJson { path: PathBuf, source: serde_json::Error },
}

#[cfg(test)]
mod tests {
    use super::{I18nKey, I18nMap, Language};

    /// Both tables must answer for every key, or a language switch would blank
    /// part of the UI.
    #[test]
    fn every_key_has_text_in_every_language() {
        for key in I18nKey::ALL {
            for language in Language::ALL {
                assert!(
                    !language.text(*key).is_empty(),
                    "{key:?} has no text in {}",
                    language.label()
                );
            }
        }
    }

    /// The point of the exercise: the Japanese must actually differ from the
    /// English. A `ja` that was forgotten falls back, and this is what catches
    /// it — stated as a count so adding a key is not an automatic failure.
    #[test]
    fn the_japanese_table_is_translated() {
        let translated =
            I18nKey::ALL.iter().filter(|key| key.default_jpn() != key.default_eng()).count();
        assert_eq!(
            I18nKey::UNTRANSLATED.len(),
            0,
            "these keys have no #[i18n(ja = ...)]: {:?}",
            I18nKey::UNTRANSLATED
        );
        assert!(
            translated * 10 >= I18nKey::ALL.len() * 9,
            "only {translated} of {} keys read differently in Japanese",
            I18nKey::ALL.len()
        );
    }

    /// Every language must be reachable through the loaded set, or a switch
    /// would silently leave the UI in the previous one.
    #[test]
    fn the_loaded_set_answers_for_every_language() {
        let translations = super::Translations::load();
        for language in Language::ALL {
            assert_eq!(translations.get(*language).language(), *language);
        }
        assert_eq!(translations.get(Language::Japanese).t(I18nKey::FileLabel), "ファイル");
        assert_eq!(translations.get(Language::English).t(I18nKey::FileLabel), "File");
    }

    #[test]
    fn a_built_in_map_translates_without_any_file() {
        let map = I18nMap::built_in(Language::Japanese);
        assert_eq!(map.t(I18nKey::FileLabel), "ファイル");
        assert_eq!(map.language(), Language::Japanese);

        let english = I18nMap::built_in(Language::English);
        assert_eq!(english.t(I18nKey::FileLabel), "File");
    }

    /// A file from another version must not stop the emulator starting, and an
    /// unknown key in it must not become visible text.
    #[test]
    fn an_unknown_key_deserialises_to_invalid() {
        let map: I18nMap =
            serde_json::from_str(r#"{"file_label": "書類", "no_such_key": "x"}"#).unwrap();
        assert_eq!(map.t(I18nKey::FileLabel), "書類");
        // Everything the file omits is simply absent from the overlay, and
        // `load_with_fallback` is what fills those in.
        assert_eq!(map.t(I18nKey::Quit), I18nKey::Quit.default_eng());
    }

    /// An override file that changes one wording must not blank the rest.
    #[test]
    fn an_overlay_only_replaces_what_it_names() {
        let mut map = I18nMap::built_in(Language::Japanese);
        let overlay: I18nMap = serde_json::from_str(r#"{"quit": "おわる"}"#).unwrap();
        for (key, text) in overlay.strings {
            if key != I18nKey::Invalid {
                map.strings.insert(key, text);
            }
        }
        assert_eq!(map.t(I18nKey::Quit), "おわる");
        assert_eq!(map.t(I18nKey::FileLabel), "ファイル");
    }
}
