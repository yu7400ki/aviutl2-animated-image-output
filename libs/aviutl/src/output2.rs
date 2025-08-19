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

    /// BGRフォーマットのフレームデータをRGBに変換して取得
    #[inline(always)]
    pub fn get_video_rgb(&self, frame: i32) -> Option<Vec<u8>> {
        let data_ptr = self.get_video(frame, video_format::BI_RGB)?;

        unsafe {
            let input_stride = ((self.w * 3 + 3) / 4) * 4; // RGB24のストライド（4バイト境界アライメント）
            let data_slice =
                std::slice::from_raw_parts(data_ptr as *const u8, (input_stride * self.h) as usize);

            let mut image_buffer = Vec::with_capacity((self.w * self.h * 3) as usize);

            // BMPは下から上に格納されているので反転してBGR→RGB変換
            for y in (0..self.h).rev() {
                let row_start = (y * input_stride) as usize;
                let row_end = row_start + (self.w * 3) as usize;
                for bgr_pixel in data_slice[row_start..row_end].chunks_exact(3) {
                    image_buffer.push(bgr_pixel[2]); // R
                    image_buffer.push(bgr_pixel[1]); // G
                    image_buffer.push(bgr_pixel[0]); // B
                }
            }
            Some(image_buffer)
        }
    }

    /// PA64フォーマットのフレームデータをRGBAに変換して取得（アルファチャンネル付き）
    #[inline(always)]
    pub fn get_video_rgba(&self, frame: i32) -> Option<Vec<u8>> {
        let data_ptr = self.get_video(frame, video_format::PA64)?;

        let data_slice = unsafe {
            std::slice::from_raw_parts(data_ptr as *const u16, (self.w * self.h * 4) as usize)
        };

        let mut image_buffer = Vec::with_capacity((self.w * self.h * 4) as usize);

        for chunk in data_slice.chunks_exact(4) {
            let r = chunk[0] as u32;
            let g = chunk[1] as u32;
            let b = chunk[2] as u32;
            let a = chunk[3] as u32;

            let (r8, g8, b8, a8) = if a < 128 {
                (0, 0, 0, 0)
            } else {
                (
                    ((r * 255 + a / 2) / a) as u8,
                    ((g * 255 + a / 2) / a) as u8,
                    ((b * 255 + a / 2) / a) as u8,
                    ((a + 128) / 257) as u8,
                )
            };
            image_buffer.extend_from_slice(&[r8, g8, b8, a8]);
        }

        Some(image_buffer)
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

/// 画像フォーマット定数
pub mod video_format {
    use crate::types::DWORD;

    /// RGB24bit
    pub const BI_RGB: DWORD = 0;
    /// YUY2
    pub const YUY2: DWORD = u32::from_le_bytes(*b"YUY2");
    /// PA64
    /// DXGI_FORMAT_R16G16B16A16_UNORM(乗算済みα)
    pub const PA64: DWORD = u32::from_le_bytes(*b"PA64");
    /// HF64
    /// DXGI_FORMAT_R16G16B16A16_FLOAT(乗算済みα)
    pub const HF64: DWORD = u32::from_le_bytes(*b"HF64");
}

/// 音声フォーマット定数
pub mod audio_format {
    use crate::types::DWORD;

    /// PCM 16bit
    pub const WAVE_FORMAT_PCM: DWORD = 1;
    /// PCM (float) 32bit
    pub const WAVE_FORMAT_IEEE_FLOAT: DWORD = 3;
}
