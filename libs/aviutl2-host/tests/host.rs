//! 偽ホストが [`OutputInfo`] のフレーム取得と取り込みへ繋がっていることを検証する
//!
//! ホストが呼ぶ入口である [`OutputPlugin`] の既定実装も、同じ偽ホストを
//! 通して検証する。設定ファイルの置き場所は1つなので [`ConfigSession`]
//! が直列化する。

use aviutl2::{
    ColorFormat, ConfigDialog, ConfigOutcome, FileFilter, IniConfig, OutputInfo, OutputPlugin,
    PipelineError, PluginFlags, PluginInfo, sys, write_or_discard,
};
use aviutl2_host::{Host, Script, temp_path};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// 符号化側が受け取ったフレームと、受け取ったスレッド
#[derive(Default)]
struct Encoded {
    frames: Vec<Vec<u8>>,
    threads: Vec<ThreadId>,
}

/// 偽ホストから全フレームを取り込み、全て受け取る
fn encode(host: &Host) -> (Result<(), PipelineError<String>>, Encoded) {
    let info = host.info();
    let video = info.video().expect("寸法");

    let encoded = Mutex::new(Encoded::default());
    let result = video.encode_frames(ColorFormat::Rgba32, |data| {
        let mut encoded = encoded.lock().unwrap();
        encoded.frames.push(data);
        encoded.threads.push(std::thread::current().id());
        Ok(())
    });

    (result, encoded.into_inner().unwrap())
}

/// 台本が名指した寸法・刻み・保存先が、そのまま出力情報になる
#[test]
fn the_fields_the_script_names_reach_the_output_info() {
    let path = temp_path("txt");
    let host = Host::open(
        Script::new()
            .size(96, 40)
            .frame_rate(24000, 1001)
            .frames(3)
            .savefile(&path),
    );

    let info = host.info();
    let video = info.video().expect("寸法");

    assert_eq!((video.width(), video.height()), (96, 40));
    assert_eq!(info.rate(), Ok(24000));
    assert_eq!(info.scale(), Ok(1001));
    assert_eq!(info.num_frames(), Ok(3));
    assert_eq!(info.savefile(), path);
}

/// ホストの関数は呼び出し元のスレッドに留まり、フレームだけが渡る
#[test]
fn the_host_stays_on_the_calling_thread_while_the_frames_cross() {
    let host = Host::open(Script::new().frames(8));

    let (result, encoded) = encode(&host);

    assert_eq!(result, Ok(()));
    assert_eq!(
        encoded.frames,
        (0..8)
            .map(|frame| host.rgba(frame))
            .collect::<Vec<Vec<u8>>>()
    );

    let caller = std::thread::current().id();
    assert!(
        host.threads().iter().all(|&id| id == caller),
        "ホストの関数が別のスレッドから呼ばれた"
    );
    assert!(
        encoded.threads.iter().all(|&id| id != caller),
        "符号化が呼び出し元のスレッドで走った"
    );

    assert_eq!(
        host.get_video(),
        (0..8).map(|frame| (frame, sys::PA64)).collect::<Vec<_>>()
    );
    assert_eq!(
        host.rest_time(),
        (0..8).map(|frame| (frame, 8)).collect::<Vec<_>>()
    );
}

/// 中断を報せたら取り込みを止め、残り時間もそこまでしか出さない
///
/// 溜めてあったフレームは符号化されてから畳まれる。捨てるのは
/// エンコーダそのものなので、ここで途中まで符号化されていても構わない。
#[test]
fn an_abort_stops_the_fetching_where_it_happened() {
    let host = Host::open(Script::new().frames(64).abort_after(3));

    let (result, encoded) = encode(&host);

    assert_eq!(result, Err(PipelineError::Aborted));
    assert_eq!(
        encoded.frames,
        (0..3)
            .map(|frame| host.rgba(frame))
            .collect::<Vec<Vec<u8>>>()
    );

    assert_eq!(host.get_video().len(), 3);
    assert_eq!(host.rest_time().len(), 3);
}

/// ホストがフレームを返さなければ、そのフレーム番号を添えて止まる
#[test]
fn a_frame_the_host_refuses_names_itself() {
    let host = Host::open(Script::new().frames(64).missing_at(4));

    let (result, encoded) = encode(&host);

    assert_eq!(result, Err(PipelineError::FrameUnavailable(4)));
    assert_eq!(
        encoded.frames,
        (0..4)
            .map(|frame| host.rgba(frame))
            .collect::<Vec<Vec<u8>>>()
    );

    assert_eq!(host.get_video().len(), 5);
    // 返さなかったフレームでは残り時間を出さない
    assert_eq!(host.rest_time().len(), 4);
}

/// 符号化の失敗はホストの取り込みを止め、その失敗が返る
#[test]
fn an_encoding_failure_stops_the_host() {
    let host = Host::open(Script::new().frames(4096));

    let info = host.info();
    let video = info.video().expect("寸法");

    let result = video.encode_frames(ColorFormat::Rgba32, |_| Err("書き出しに失敗".to_string()));

    assert_eq!(result, Err(PipelineError::Encode("書き出しに失敗".into())));
    let fetched = host.get_video().len();
    assert!(fetched < 4096, "{fetched}フレーム取り込んだ");
}

/// ホストが返したフレームは、頼んだフォーマットで変換されて返る
#[test]
fn a_frame_the_host_returns_comes_back_converted() {
    let host = Host::open(Script::new().frames(4));

    let info = host.info();
    let video = info.video().expect("寸法");

    assert_eq!(video.get_frame(2, ColorFormat::Rgba32), Some(host.rgba(2)));
    assert_eq!(host.get_video(), vec![(2, sys::PA64)]);
}

/// ホストがフレームを返さなければ、その中身を読みに行かない
#[test]
fn a_frame_the_host_refuses_is_not_read() {
    let host = Host::open(Script::new().frames(4).missing_at(1));

    let info = host.info();
    let video = info.video().expect("寸法");

    assert_eq!(video.get_frame(1, ColorFormat::Rgba32), None);
    assert_eq!(video.get_frame(1, ColorFormat::Rgb24), None);
}

/// 頼まれたフォーマットが変わっても、返るのは同じ絵
#[test]
fn a_frame_comes_back_in_the_format_the_caller_asked_for() {
    // 行の埋めが要る幅
    let host = Host::open(Script::new().size(5, 3).frames(2));

    let info = host.info();
    let video = info.video().expect("寸法");

    let rgba = video.get_frame(1, ColorFormat::Rgba32).expect("透過付き");
    let rgb = video.get_frame(1, ColorFormat::Rgb24).expect("透過無し");

    assert_eq!(rgba, host.rgba(1));
    assert_eq!(rgb, host.rgb(1));
    assert_eq!(
        rgb,
        rgba.chunks_exact(4)
            .flat_map(|pixel| &pixel[..3])
            .copied()
            .collect::<Vec<u8>>()
    );
    assert_eq!(host.get_video(), vec![(1, sys::PA64), (1, sys::BI_RGB)]);
}

/// ループ回数だけを持つ設定
#[derive(Debug, Default, PartialEq, Eq)]
struct TestConfig {
    repeat: u32,
}

impl IniConfig for TestConfig {
    const FILE_NAME: &'static str = "host-test.ini";

    fn load_from(section: Option<&aviutl2::ini::Properties>) -> Self {
        TestConfig {
            repeat: aviutl2::read(section, "repeat", 0),
        }
    }

    fn save_to(&self, ini: &mut aviutl2::ini::Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string());
    }

    /// テスト実行ファイルの隣ではなく一時ディレクトリへ置く
    fn config_path() -> Result<PathBuf, String> {
        Ok(std::env::temp_dir().join(format!(
            "aviutl2-{}-{}",
            std::process::id(),
            Self::FILE_NAME
        )))
    }
}

static CONFIG: Mutex<()> = Mutex::new(());

/// 設定ファイルを使う権利。`repeat` を書き置いた状態で始まり、離すと消える
struct ConfigSession(#[expect(dead_code, reason = "持っている間だけ有効")] MutexGuard<'static, ()>);

impl ConfigSession {
    fn new(repeat: u32) -> ConfigSession {
        let guard = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
        TestConfig { repeat }.save().expect("設定の書き置き");
        ConfigSession(guard)
    }
}

impl Drop for ConfigSession {
    fn drop(&mut self) {
        let path = TestConfig::config_path().expect("設定の置き場所");
        let _ = std::fs::remove_file(path);
    }
}

/// 読み込んだ設定を符号化と設定ダイアログの双方へ通すプラグイン
struct TestPlugin;

impl OutputPlugin for TestPlugin {
    type Config = TestConfig;

    const FORMAT_NAME: &'static str = "テスト";
    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "テスト出力プラグイン".into(),
            file_filter: FileFilter::new(),
            information: "テスト出力プラグイン".into(),
        }
    }

    /// 受け取った設定をそのままエラー文言にする
    fn encode(_info: &OutputInfo, config: &TestConfig) -> Result<(), String> {
        Err(config.repeat.to_string())
    }

    /// 受け取った設定を1つ進めて返す
    fn show_config_dialog(_hwnd: HWND, config: TestConfig) -> ConfigDialog<TestConfig> {
        ConfigDialog::Accepted(TestConfig {
            repeat: config.repeat + 1,
        })
    }
}

/// 設定ダイアログを取り消すプラグイン
struct CancellingPlugin;

impl OutputPlugin for CancellingPlugin {
    type Config = TestConfig;

    const FORMAT_NAME: &'static str = "テスト";
    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        TestPlugin::info()
    }

    fn encode(info: &OutputInfo, config: &TestConfig) -> Result<(), String> {
        TestPlugin::encode(info, config)
    }

    fn show_config_dialog(_hwnd: HWND, _config: TestConfig) -> ConfigDialog<TestConfig> {
        ConfigDialog::Cancelled
    }
}

/// 取り込んだ素材の姿を保存先へ書き出すプラグイン
struct WritingPlugin;

impl OutputPlugin for WritingPlugin {
    type Config = TestConfig;

    const FORMAT_NAME: &'static str = "テスト";

    fn info() -> PluginInfo {
        TestPlugin::info()
    }

    fn encode(info: &OutputInfo, config: &TestConfig) -> Result<(), String> {
        let video = info.video()?;

        write_or_discard(&info.savefile(), |mut file| {
            let mut frames = 0;
            video
                .encode_frames(ColorFormat::Rgba32, |_| {
                    frames += 1;
                    Ok::<(), String>(())
                })
                .map_err(|e| e.to_string())?;

            writeln!(
                file,
                "{}x{} {}/{} {}枚 repeat={}",
                video.width(),
                video.height(),
                info.rate()?,
                info.scale()?,
                frames,
                config.repeat
            )
            .map_err(|e| e.to_string())
        })
    }
}

/// プラグインの書き出しは、台本が名指した素材と保存先を受け取る
#[test]
fn the_encoding_receives_the_material_the_script_names() {
    let path = temp_path("txt");
    let host = Host::open(
        Script::new()
            .size(96, 40)
            .frame_rate(24000, 1001)
            .frames(3)
            .savefile(&path),
    );

    assert_eq!(
        host.encode::<WritingPlugin>(&TestConfig { repeat: 5 }),
        Ok(())
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("保存先"),
        "96x40 24000/1001 3枚 repeat=5\n"
    );

    std::fs::remove_file(&path).expect("保存先の後始末");
}

/// 出力は保存された設定で符号化し、失敗に形式名を添えて返す
#[test]
fn the_saved_config_reaches_the_encoder_and_its_failure_wears_the_format_name() {
    let _config = ConfigSession::new(7);

    let host = Host::open(Script::new().frames(1));

    assert_eq!(
        TestPlugin::output(&host.info()),
        Err("テスト出力エラー: 7".to_string())
    );
}

/// 設定ダイアログが返した設定は保存される
#[test]
fn the_config_the_dialog_returns_is_saved() {
    let _config = ConfigSession::new(7);

    assert_eq!(
        TestPlugin::config(HWND::default(), HINSTANCE::default()),
        ConfigOutcome::Saved
    );
    assert_eq!(TestConfig::load(), TestConfig { repeat: 8 });
}

/// 取り消された設定ダイアログは、保存された設定に触れない
#[test]
fn a_cancelled_dialog_leaves_the_saved_config_alone() {
    let _config = ConfigSession::new(7);

    assert_eq!(
        CancellingPlugin::config(HWND::default(), HINSTANCE::default()),
        ConfigOutcome::Cancelled
    );
    assert_eq!(TestConfig::load(), TestConfig { repeat: 7 });
}

/// 設定ダイアログが設定を返せないプラグイン
struct FailingDialogPlugin;

impl OutputPlugin for FailingDialogPlugin {
    type Config = TestConfig;

    const FORMAT_NAME: &'static str = "テスト";
    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        TestPlugin::info()
    }

    fn encode(info: &OutputInfo, config: &TestConfig) -> Result<(), String> {
        TestPlugin::encode(info, config)
    }

    fn show_config_dialog(_hwnd: HWND, _config: TestConfig) -> ConfigDialog<TestConfig> {
        ConfigDialog::Failed
    }
}

/// 無いディレクトリの下を置き場所に名指す設定
#[derive(Default)]
struct UnwritableConfig;

impl IniConfig for UnwritableConfig {
    const FILE_NAME: &'static str = "host-test-unwritable.ini";

    fn load_from(_section: Option<&aviutl2::ini::Properties>) -> Self {
        UnwritableConfig
    }

    fn save_to(&self, _ini: &mut aviutl2::ini::Ini) {}

    fn config_path() -> Result<PathBuf, String> {
        Ok(std::env::temp_dir()
            .join(format!("aviutl2-{}-missing", std::process::id()))
            .join(Self::FILE_NAME))
    }
}

/// 保存できない設定をダイアログが返すプラグイン
struct UnwritablePlugin;

impl OutputPlugin for UnwritablePlugin {
    type Config = UnwritableConfig;

    const FORMAT_NAME: &'static str = "テスト";
    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        TestPlugin::info()
    }

    fn encode(_info: &OutputInfo, _config: &UnwritableConfig) -> Result<(), String> {
        Ok(())
    }

    fn show_config_dialog(_hwnd: HWND, config: UnwritableConfig) -> ConfigDialog<UnwritableConfig> {
        ConfigDialog::Accepted(config)
    }
}

/// 設定を返せなかったダイアログは、保存された設定に触れない
#[test]
fn a_failed_dialog_leaves_the_saved_config_alone() {
    let _config = ConfigSession::new(7);

    assert_eq!(
        FailingDialogPlugin::config(HWND::default(), HINSTANCE::default()),
        ConfigOutcome::Failed
    );
    assert_eq!(TestConfig::load(), TestConfig { repeat: 7 });
}

/// 保存できなかった設定は、その理由を連れて返る
#[test]
fn a_config_that_cannot_be_saved_comes_back_with_the_reason() {
    let path = UnwritableConfig::config_path().expect("設定の置き場所");

    let outcome = UnwritablePlugin::config(HWND::default(), HINSTANCE::default());

    let ConfigOutcome::NotSaved(message) = &outcome else {
        panic!("保存の失敗が返らなかった: {outcome:?}");
    };
    assert!(message.starts_with("設定保存エラー: "), "{message}");
    assert!(!path.exists(), "書けない場所に設定が残った");
}
