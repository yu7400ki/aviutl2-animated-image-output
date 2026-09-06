//! 偽のホストを立てて [`OutputInfo`] のフレーム取得と取り込みの配線を検証する
//!
//! ホストの関数ポインタは引数に手掛かりを持たないので、台本と記録は
//! プロセス全体で1組しか置けない。[`Session`] が1本ずつに直列化する。
//!
//! ホストが呼ぶ入口である [`OutputPlugin`] の既定実装も、同じ `OUTPUT_INFO`
//! を渡して検証する。設定ファイルの置き場所も1つなので [`ConfigSession`]
//! が直列化する。

use aviutl2::{
    ColorFormat, ConfigDialog, FileFilter, IniConfig, OutputInfo, OutputPlugin, PipelineError,
    PluginFlags, PluginInfo, sys,
};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// 偽ホストの寸法。PA64の1フレームは `WIDTH * HEIGHT * 4` 要素
const WIDTH: usize = 2;
const HEIGHT: usize = 2;

/// 偽ホストの台本と、そこへ来た呼び出しの記録
struct Host {
    /// フレームを返さないフレーム番号
    missing_at: Option<i32>,
    /// 中断を報せ始めるフレーム番号
    abort_at: Option<i32>,
    /// `func_get_video` へ来たフレーム番号とフォーマット
    get_video: Vec<(i32, u32)>,
    /// `func_rest_time_disp` へ来た引数
    rest_time: Vec<(i32, i32)>,
    /// ホストの関数を呼んだスレッド
    threads: Vec<ThreadId>,
    /// 直前に返したフレームの中身
    frame: Vec<u16>,
}

impl Host {
    const fn new() -> Self {
        Host {
            missing_at: None,
            abort_at: None,
            get_video: Vec::new(),
            rest_time: Vec::new(),
            threads: Vec::new(),
            frame: Vec::new(),
        }
    }
}

static HOST: Mutex<Host> = Mutex::new(Host::new());
static SESSION: Mutex<()> = Mutex::new(());

/// 偽ホストを使う権利。持っている間だけ台本と記録が自分のものになる
struct Session(#[expect(dead_code, reason = "持っている間だけ有効")] MutexGuard<'static, ()>);

impl Session {
    fn new() -> Session {
        let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        *host() = Host::new();
        Session(guard)
    }
}

fn host() -> MutexGuard<'static, Host> {
    HOST.lock().unwrap_or_else(|e| e.into_inner())
}

/// 全画素が `frame` の値で不透明なPA64のフレーム
fn pa64_frame(frame: i32) -> Vec<u16> {
    let channel = (frame as u16) * 257;
    [channel, channel, channel, u16::MAX].repeat(WIDTH * HEIGHT)
}

/// [`pa64_frame`] を変換して得られるはずのRGBA8
fn rgba8_frame(frame: i32) -> Vec<u8> {
    [frame as u8, frame as u8, frame as u8, u8::MAX].repeat(WIDTH * HEIGHT)
}

unsafe extern "C" fn get_video(frame: i32, format: u32) -> *mut c_void {
    let mut host = host();
    host.threads.push(std::thread::current().id());
    host.get_video.push((frame, format));

    if host.missing_at == Some(frame) {
        return std::ptr::null_mut();
    }

    host.frame = pa64_frame(frame);
    host.frame.as_mut_ptr() as *mut c_void
}

unsafe extern "C" fn is_abort() -> bool {
    let mut host = host();
    host.threads.push(std::thread::current().id());
    // 中断は一度報せたら戻らない
    let fetched = host.get_video.len() as i32;
    host.abort_at.is_some_and(|at| at <= fetched)
}

unsafe extern "C" fn rest_time_disp(now: i32, total: i32) {
    let mut host = host();
    host.threads.push(std::thread::current().id());
    host.rest_time.push((now, total));
}

/// 偽ホストへ繋いだ `OUTPUT_INFO`
fn output_info(frames: i32) -> sys::OUTPUT_INFO {
    sys::OUTPUT_INFO {
        flag: sys::OUTPUT_INFO::FLAG_VIDEO,
        w: WIDTH as i32,
        h: HEIGHT as i32,
        rate: 30,
        scale: 1,
        n: frames,
        audio_rate: 0,
        audio_ch: 0,
        audio_n: 0,
        savefile: std::ptr::null(),
        func_get_video: Some(get_video),
        func_get_audio: None,
        func_is_abort: Some(is_abort),
        func_rest_time_disp: Some(rest_time_disp),
        func_set_buffer_size: None,
    }
}

/// 符号化側が受け取ったフレームと、受け取ったスレッド
#[derive(Default)]
struct Encoded {
    frames: Vec<Vec<u8>>,
    threads: Vec<ThreadId>,
}

/// 偽ホストから `frames` フレームを取り込み、全て受け取る
fn encode(frames: i32) -> (Result<(), PipelineError<String>>, Encoded) {
    let raw = output_info(frames);
    let info = unsafe { OutputInfo::from_raw(&raw) }.expect("OUTPUT_INFOがnull");

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

/// ホストの関数は呼び出し元のスレッドに留まり、フレームだけが渡る
#[test]
fn the_host_stays_on_the_calling_thread_while_the_frames_cross() {
    let _session = Session::new();

    let (result, encoded) = encode(8);

    assert_eq!(result, Ok(()));
    assert_eq!(
        encoded.frames,
        (0..8).map(rgba8_frame).collect::<Vec<Vec<u8>>>()
    );

    let caller = std::thread::current().id();
    let host = host();
    assert!(
        host.threads.iter().all(|&id| id == caller),
        "ホストの関数が別のスレッドから呼ばれた"
    );
    assert!(
        encoded.threads.iter().all(|&id| id != caller),
        "符号化が呼び出し元のスレッドで走った"
    );

    assert_eq!(
        host.get_video,
        (0..8).map(|frame| (frame, sys::PA64)).collect::<Vec<_>>()
    );
    assert_eq!(
        host.rest_time,
        (0..8).map(|frame| (frame, 8)).collect::<Vec<_>>()
    );
}

/// 中断を報せたら取り込みを止め、残り時間もそこまでしか出さない
///
/// 溜めてあったフレームは符号化されてから畳まれる。捨てるのは
/// エンコーダそのものなので、ここで途中まで符号化されていても構わない。
#[test]
fn an_abort_stops_the_fetching_where_it_happened() {
    let _session = Session::new();
    host().abort_at = Some(3);

    let (result, encoded) = encode(64);

    assert_eq!(result, Err(PipelineError::Aborted));
    assert_eq!(
        encoded.frames,
        (0..3).map(rgba8_frame).collect::<Vec<Vec<u8>>>()
    );

    let host = host();
    assert_eq!(host.get_video.len(), 3);
    assert_eq!(host.rest_time.len(), 3);
}

/// ホストがフレームを返さなければ、そのフレーム番号を添えて止まる
#[test]
fn a_frame_the_host_refuses_names_itself() {
    let _session = Session::new();
    host().missing_at = Some(4);

    let (result, encoded) = encode(64);

    assert_eq!(result, Err(PipelineError::FrameUnavailable(4)));
    assert_eq!(
        encoded.frames,
        (0..4).map(rgba8_frame).collect::<Vec<Vec<u8>>>()
    );

    let host = host();
    assert_eq!(host.get_video.len(), 5);
    // 返さなかったフレームでは残り時間を出さない
    assert_eq!(host.rest_time.len(), 4);
}

/// 符号化の失敗はホストの取り込みを止め、その失敗が返る
#[test]
fn an_encoding_failure_stops_the_host() {
    let _session = Session::new();

    let raw = output_info(4096);
    let info = unsafe { OutputInfo::from_raw(&raw) }.expect("OUTPUT_INFOがnull");

    let video = info.video().expect("寸法");

    let result = video.encode_frames(ColorFormat::Rgba32, |_| Err("書き出しに失敗".to_string()));

    assert_eq!(result, Err(PipelineError::Encode("書き出しに失敗".into())));
    let fetched = host().get_video.len();
    assert!(fetched < 4096, "{fetched}フレーム取り込んだ");
}

/// ホストが返したフレームは、頼んだフォーマットで変換されて返る
#[test]
fn a_frame_the_host_returns_comes_back_converted() {
    let _session = Session::new();

    let raw = output_info(4);
    let info = unsafe { OutputInfo::from_raw(&raw) }.expect("OUTPUT_INFOがnull");

    let video = info.video().expect("寸法");

    assert_eq!(
        video.get_frame(2, ColorFormat::Rgba32),
        Some(rgba8_frame(2))
    );
    assert_eq!(host().get_video, vec![(2, sys::PA64)]);
}

/// ホストがフレームを返さなければ、その中身を読みに行かない
#[test]
fn a_frame_the_host_refuses_is_not_read() {
    let _session = Session::new();
    host().missing_at = Some(1);

    let raw = output_info(4);
    let info = unsafe { OutputInfo::from_raw(&raw) }.expect("OUTPUT_INFOがnull");

    let video = info.video().expect("寸法");

    assert_eq!(video.get_frame(1, ColorFormat::Rgba32), None);
    assert_eq!(video.get_frame(1, ColorFormat::Rgb24), None);
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

/// 出力は保存された設定で符号化し、失敗に形式名を添えて返す
#[test]
fn the_saved_config_reaches_the_encoder_and_its_failure_wears_the_format_name() {
    let _config = ConfigSession::new(7);

    let raw = output_info(1);
    let info = unsafe { OutputInfo::from_raw(&raw) }.expect("OUTPUT_INFOがnull");

    assert_eq!(
        TestPlugin::output(&info),
        Err("テスト出力エラー: 7".to_string())
    );
}

/// 設定ダイアログが返した設定は保存される
#[test]
fn the_config_the_dialog_returns_is_saved() {
    let _config = ConfigSession::new(7);

    assert!(TestPlugin::config(HWND::default(), HINSTANCE::default()));
    assert_eq!(TestConfig::load(), TestConfig { repeat: 8 });
}

/// 取り消された設定ダイアログは、保存された設定に触れない
#[test]
fn a_cancelled_dialog_leaves_the_saved_config_alone() {
    let _config = ConfigSession::new(7);

    assert!(!CancellingPlugin::config(
        HWND::default(),
        HINSTANCE::default()
    ));
    assert_eq!(TestConfig::load(), TestConfig { repeat: 7 });
}
