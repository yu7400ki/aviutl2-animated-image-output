//! 偽のホストを立てて [`OutputInfo`] のフレーム取得を検証する
//!
//! ホストの関数ポインタは引数に手掛かりを持たないので、台本と記録は
//! プロセス全体で1組しか置けない。[`Session`] が1本ずつに直列化する。

use aviutl2::{ColorFormat, OutputInfo, sys};
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

/// 偽ホストの寸法。PA64の1フレームは `WIDTH * HEIGHT * 4` 要素
const WIDTH: usize = 2;
const HEIGHT: usize = 2;

/// 偽ホストの台本と、そこへ来た呼び出しの記録
struct Host {
    /// フレームを返さないフレーム番号
    missing_at: Option<i32>,
    /// `func_get_video` へ来たフレーム番号とフォーマット
    get_video: Vec<(i32, u32)>,
    /// 直前に返したフレームの中身
    frame: Vec<u16>,
}

impl Host {
    const fn new() -> Self {
        Host {
            missing_at: None,
            get_video: Vec::new(),
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
    host.get_video.push((frame, format));

    if host.missing_at == Some(frame) {
        return std::ptr::null_mut();
    }

    host.frame = pa64_frame(frame);
    host.frame.as_mut_ptr() as *mut c_void
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
        func_is_abort: None,
        func_rest_time_disp: None,
        func_set_buffer_size: None,
    }
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
