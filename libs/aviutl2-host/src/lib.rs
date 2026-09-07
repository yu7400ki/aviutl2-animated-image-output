//! 偽のホストを立てて `OUTPUT_INFO` を組み、出力プラグインへ繋ぐ
//!
//! ホストの関数ポインタは引数に手掛かりを持たないので、台本と記録は
//! プロセス全体で1組しか置けない。[`Host`] が1本ずつに直列化する。

use aviutl2::{OutputInfo, OutputPlugin, sys};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;
use widestring::U16CString;

/// 偽ホストが返す素材と、そこへ書く台本
///
/// 幅と高さ、フレームレートの分子と分母は、既定でも互いに違う値になる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    width: u32,
    height: u32,
    rate: u32,
    scale: u32,
    frames: u32,
    savefile: Option<PathBuf>,
    missing_at: Option<i32>,
    abort_after: Option<i32>,
}

impl Default for Script {
    fn default() -> Script {
        Script {
            width: 32,
            height: 16,
            rate: 30000,
            scale: 1001,
            frames: 8,
            savefile: None,
            missing_at: None,
            abort_after: None,
        }
    }
}

impl Script {
    /// 全フレームを滞りなく返し、保存先を持たない台本
    pub fn new() -> Script {
        Script::default()
    }

    /// フレームの寸法
    pub fn size(self, width: u32, height: u32) -> Script {
        Script {
            width,
            height,
            ..self
        }
    }

    /// フレームレートの分子と分母
    pub fn frame_rate(self, rate: u32, scale: u32) -> Script {
        Script {
            rate,
            scale,
            ..self
        }
    }

    /// 素材のフレーム数
    pub fn frames(self, frames: u32) -> Script {
        Script { frames, ..self }
    }

    /// 保存先
    pub fn savefile(self, path: impl Into<PathBuf>) -> Script {
        Script {
            savefile: Some(path.into()),
            ..self
        }
    }

    /// `frame` を求められても返さない
    pub fn missing_at(self, frame: i32) -> Script {
        Script {
            missing_at: Some(frame),
            ..self
        }
    }

    /// `frames` フレーム求められた後は中断を報せる
    pub fn abort_after(self, frames: i32) -> Script {
        Script {
            abort_after: Some(frames),
            ..self
        }
    }
}

/// 偽ホストを使う権利。持っている間だけ台本と記録が自分のものになる
pub struct Host {
    #[expect(dead_code, reason = "持っている間だけ有効")]
    guard: MutexGuard<'static, ()>,
    script: Script,
    raw: sys::OUTPUT_INFO,
    /// `raw.savefile` が指す先
    #[expect(dead_code, reason = "raw が指している間だけ有効")]
    savefile: Option<U16CString>,
}

impl Host {
    /// 台本を据えて偽ホストを占有する
    pub fn open(script: Script) -> Host {
        let guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        *state() = State {
            width: script.width,
            height: script.height,
            missing_at: script.missing_at,
            abort_after: script.abort_after,
            ..State::new()
        };

        let savefile = script
            .savefile
            .as_ref()
            .map(|path| U16CString::from_os_str(path).expect("保存先の途中にNULがある"));
        let raw = sys::OUTPUT_INFO {
            flag: sys::OUTPUT_INFO::FLAG_VIDEO,
            w: script.width as i32,
            h: script.height as i32,
            rate: script.rate as i32,
            scale: script.scale as i32,
            n: script.frames as i32,
            audio_rate: 0,
            audio_ch: 0,
            audio_n: 0,
            savefile: savefile
                .as_ref()
                .map_or(std::ptr::null(), |savefile| savefile.as_ptr()),
            func_get_video: Some(get_video),
            func_get_audio: None,
            func_is_abort: Some(is_abort),
            func_rest_time_disp: Some(rest_time_disp),
            func_set_buffer_size: None,
        };

        Host {
            guard,
            script,
            raw,
            savefile,
        }
    }

    /// 偽ホストへ繋がった出力情報
    pub fn info(&self) -> OutputInfo<'_> {
        unsafe { OutputInfo::from_raw(&self.raw) }.expect("OUTPUT_INFOがnull")
    }

    /// 偽ホストの出力情報で `P` の書き出しを走らせる
    pub fn encode<P: OutputPlugin>(&self, config: &P::Config) -> Result<(), String> {
        P::encode(&self.info(), config)
    }

    /// フレーム `frame` を透過付きで取り込んだときの中身
    pub fn rgba(&self, frame: i32) -> Vec<u8> {
        rgba_frame(self.script.width, self.script.height, frame)
    }

    /// フレーム `frame` を透過無しで取り込んだときの中身
    pub fn rgb(&self, frame: i32) -> Vec<u8> {
        rgb_frame(self.script.width, self.script.height, frame)
    }

    /// 画像データの取得へ来たフレーム番号とフォーマット
    pub fn get_video(&self) -> Vec<(i32, u32)> {
        state().get_video.clone()
    }

    /// 残り時間表示へ来た引数
    pub fn rest_time(&self) -> Vec<(i32, i32)> {
        state().rest_time.clone()
    }

    /// ホストの関数を呼んだスレッド
    pub fn threads(&self) -> Vec<ThreadId> {
        state().threads.clone()
    }
}

/// 直前に返したフレームの中身
///
/// ホストが渡すポインタは次の取得までしか有効でないので、1枚だけ持つ。
enum Buffer {
    Pa64(Vec<u16>),
    Bgr(Vec<u8>),
}

impl Buffer {
    fn as_mut_ptr(&mut self) -> *mut c_void {
        match self {
            Buffer::Pa64(data) => data.as_mut_ptr() as *mut c_void,
            Buffer::Bgr(data) => data.as_mut_ptr() as *mut c_void,
        }
    }
}

/// 偽ホストが返す素材の寸法と台本、そこへ来た呼び出しの記録
struct State {
    width: u32,
    height: u32,
    missing_at: Option<i32>,
    abort_after: Option<i32>,
    get_video: Vec<(i32, u32)>,
    rest_time: Vec<(i32, i32)>,
    threads: Vec<ThreadId>,
    frame: Option<Buffer>,
}

impl State {
    const fn new() -> State {
        State {
            width: 0,
            height: 0,
            missing_at: None,
            abort_after: None,
            get_video: Vec::new(),
            rest_time: Vec::new(),
            threads: Vec::new(),
            frame: None,
        }
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());
static SESSION: Mutex<()> = Mutex::new(());

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// フレーム `frame` の画素 `(x, y)` の色
///
/// 3成分はそれぞれ x・y・フレーム番号だけで決まる。
fn pixel(x: u32, y: u32, frame: i32) -> [u8; 3] {
    [(x * 7) as u8, (y * 11) as u8, frame as u8]
}

/// 不透明なRGBA8のフレーム
fn rgba_frame(width: u32, height: u32, frame: i32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let [r, g, b] = pixel(x, y, frame);
                [r, g, b, u8::MAX]
            })
        })
        .collect()
}

/// [`rgba_frame`] と同じ色のRGB24のフレーム
fn rgb_frame(width: u32, height: u32, frame: i32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| pixel(x, y, frame)))
        .collect()
}

/// [`rgba_frame`] と同じ色の、不透明なPA64のフレーム
fn pa64_frame(width: u32, height: u32, frame: i32) -> Vec<u16> {
    (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let [r, g, b] = pixel(x, y, frame);
                [
                    u16::from(r) * 257,
                    u16::from(g) * 257,
                    u16::from(b) * 257,
                    u16::MAX,
                ]
            })
        })
        .collect()
}

/// [`rgb_frame`] と同じ色の、下から上・行4バイト境界のBGR24のフレーム
fn bgr_frame(width: u32, height: u32, frame: i32) -> Vec<u8> {
    let stride = (width as usize * 3).next_multiple_of(4);
    let mut buffer = vec![0u8; stride * height as usize];

    for y in 0..height {
        let row = (height - 1 - y) as usize * stride;
        for x in 0..width {
            let [r, g, b] = pixel(x, y, frame);
            let at = row + x as usize * 3;
            buffer[at..at + 3].copy_from_slice(&[b, g, r]);
        }
    }
    buffer
}

unsafe extern "C" fn get_video(frame: i32, format: u32) -> *mut c_void {
    let mut state = state();
    state.threads.push(std::thread::current().id());
    state.get_video.push((frame, format));

    if state.missing_at == Some(frame) {
        return std::ptr::null_mut();
    }

    let (width, height) = (state.width, state.height);
    let buffer = match format {
        sys::PA64 => Buffer::Pa64(pa64_frame(width, height, frame)),
        sys::BI_RGB => Buffer::Bgr(bgr_frame(width, height, frame)),
        _ => return std::ptr::null_mut(),
    };
    state.frame.insert(buffer).as_mut_ptr()
}

unsafe extern "C" fn is_abort() -> bool {
    let mut state = state();
    state.threads.push(std::thread::current().id());
    // 中断は一度報せたら戻らない
    let fetched = state.get_video.len() as i32;
    state.abort_after.is_some_and(|after| after <= fetched)
}

unsafe extern "C" fn rest_time_disp(now: i32, total: i32) {
    let mut state = state();
    state.threads.push(std::thread::current().id());
    state.rest_time.push((now, total));
}
