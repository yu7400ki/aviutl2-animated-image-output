//! 偽のホストを立てて [`OutputInfo`] のフレーム取得と取り込みの配線を検証する
//!
//! ホストの関数ポインタは引数に手掛かりを持たないので、台本と記録は
//! プロセス全体で1組しか置けない。[`Session`] が1本ずつに直列化する。

use aviutl2::{ColorFormat, OutputInfo, PipelineError, sys};
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;

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

    let encoded = Mutex::new(Encoded::default());
    let result = info.encode_frames(ColorFormat::Rgba32, |data| {
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

    let result = info.encode_frames(ColorFormat::Rgba32, |_| Err("書き出しに失敗".to_string()));

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

    assert_eq!(
        info.get_video_frame(2, ColorFormat::Rgba32),
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

    assert_eq!(info.get_video_frame(1, ColorFormat::Rgba32), None);
    assert_eq!(info.get_video_frame(1, ColorFormat::Rgb24), None);
}
