/*
---------------------------------
AviUtl ExEdit2 Plugin SDK License
---------------------------------

The MIT License

Copyright (c) 2025 Kenkun

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
 */
#![allow(dead_code)]

use crate::types::{DWORD, LPCWSTR};
use std::os::raw::{c_int, c_void};
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// 出力情報構造体
#[repr(C)]
pub struct OutputInfo {
    /// フラグ
    pub flag: c_int,
    /// 幅
    pub w: c_int,
    /// 高さ
    pub h: c_int,
    /// フレームレート
    pub rate: c_int,
    /// スケール
    pub scale: c_int,
    /// フレーム数
    pub n: c_int,
    /// 音声サンプリングレート
    pub audio_rate: c_int,
    /// 音声チャンネル数
    pub audio_ch: c_int,
    /// 音声サンプリング数
    pub audio_n: c_int,
    /// セーブファイル名へのポインタ
    pub savefile: LPCWSTR,

    /// DIB形式の画像データを取得
    /// - frame: フレーム番号
    /// - format: 画像フォーマット (0 = RGB24bit / 'YUY2' = YUY2)
    /// - 戻り値: データへのポインタ
    pub func_get_video: Option<extern "C" fn(frame: c_int, format: DWORD) -> *mut c_void>,

    /// PCM形式の音声データへのポインタを取得
    /// - start: 開始サンプル番号
    /// - length: 読み込むサンプル数
    /// - readed: 読み込まれたサンプル数
    /// - format: 音声フォーマット (1 = PCM16bit / 3 = PCM(float)32bit)
    /// - 戻り値: データへのポインタ
    pub func_get_audio: Option<
        extern "C" fn(
            start: c_int,
            length: c_int,
            readed: *mut c_int,
            format: DWORD,
        ) -> *mut c_void,
    >,

    /// 中断するか調べる
    /// - 戻り値: trueなら中断
    pub func_is_abort: Option<extern "C" fn() -> bool>,

    /// 残り時間を表示
    /// - now: 処理しているフレーム番号
    /// - total: 処理する総フレーム数
    pub func_rest_time_disp: Option<extern "C" fn(now: c_int, total: c_int)>,

    /// データ取得のバッファ数を設定
    /// - video_size: 画像データのバッファ数
    /// - audio_size: 音声データのバッファ数
    pub func_set_buffer_size: Option<extern "C" fn(video_size: c_int, audio_size: c_int)>,
}

impl OutputInfo {
    /// フラグ定数: 画像データあり
    pub const FLAG_VIDEO: c_int = 1;
    /// フラグ定数: 音声データあり  
    pub const FLAG_AUDIO: c_int = 2;

    /// 画像データを取得
    pub fn get_video(&self, frame: i32, format: u32) -> Option<*mut c_void> {
        self.func_get_video.map(|f| f(frame, format))
    }

    /// 音声データを取得
    pub fn get_audio(&self, start: i32, length: i32, format: u32) -> Option<(*mut c_void, i32)> {
        self.func_get_audio.map(|f| {
            let mut readed = 0;
            let ptr = f(start, length, &mut readed, format);
            (ptr, readed)
        })
    }

    /// 中断チェック
    pub fn is_abort(&self) -> bool {
        self.func_is_abort.map(|f| f()).unwrap_or(false)
    }

    /// 残り時間表示
    pub fn rest_time_disp(&self, now: i32, total: i32) {
        if let Some(f) = self.func_rest_time_disp {
            f(now, total);
        }
    }

    /// バッファサイズ設定
    pub fn set_buffer_size(&self, video_size: i32, audio_size: i32) {
        if let Some(f) = self.func_set_buffer_size {
            f(video_size, audio_size);
        }
    }
}

/// 出力プラグイン構造体
#[repr(C)]
pub struct OutputPluginTable {
    /// フラグ (未使用)
    pub flag: c_int,
    /// プラグインの名前
    pub name: LPCWSTR,
    /// ファイルのフィルタ
    pub filefilter: LPCWSTR,
    /// プラグインの情報
    pub information: LPCWSTR,

    /// 出力時に呼ばれる関数
    pub func_output: Option<extern "C" fn(oip: *mut OutputInfo) -> bool>,

    /// 出力設定のダイアログを要求された時に呼ばれる関数
    pub func_config: Option<extern "C" fn(hwnd: HWND, dll_hinst: HINSTANCE) -> bool>,

    /// 出力設定のテキスト情報を取得する時に呼ばれる関数
    pub func_get_config_text: Option<extern "C" fn() -> LPCWSTR>,
}

impl OutputPluginTable {
    /// フラグ定数: 画像をサポートする
    pub const FLAG_VIDEO: c_int = 1;
    /// フラグ定数: 音声をサポートする
    pub const FLAG_AUDIO: c_int = 2;
}

unsafe impl Send for OutputPluginTable {}
unsafe impl Sync for OutputPluginTable {}

/// 画像フォーマット定数
pub mod video_format {
    use crate::types::DWORD;

    /// RGB24bit
    pub const BI_RGB: DWORD = 0;
    /// YUY2
    pub const YUY2: DWORD = u32::from_le_bytes(*b"YUY2");
}

/// 音声フォーマット定数
pub mod audio_format {
    use crate::types::DWORD;

    /// PCM 16bit
    pub const WAVE_FORMAT_PCM: DWORD = 1;
    /// PCM (float) 32bit
    pub const WAVE_FORMAT_IEEE_FLOAT: DWORD = 3;
}
