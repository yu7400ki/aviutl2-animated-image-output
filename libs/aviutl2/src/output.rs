//! 出力プラグインの安全なAPI

use crate::pixel::ColorFormat;
use crate::sys;
use std::ffi::c_void;
use std::path::PathBuf;
use widestring::U16CStr;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// [`sys::OUTPUT_INFO`] の安全なラッパー
pub struct OutputInfo<'a> {
    raw: &'a sys::OUTPUT_INFO,
}

impl<'a> OutputInfo<'a> {
    /// 生ポインタからラッパーを構築する (nullなら`None`)
    ///
    /// # Safety
    /// `ptr` はnullであるか、有効な `OUTPUT_INFO` を指していること。
    /// 参照はホストへ処理を戻すまでの間のみ有効。
    pub unsafe fn from_raw(ptr: *const sys::OUTPUT_INFO) -> Option<OutputInfo<'a>> {
        unsafe { ptr.as_ref() }.map(|raw| OutputInfo { raw })
    }

    /// 保存ファイルパス
    pub fn savefile(&self) -> PathBuf {
        if self.raw.savefile.is_null() {
            return PathBuf::new();
        }
        let s = unsafe { U16CStr::from_ptr_str(self.raw.savefile) };
        PathBuf::from(s.to_string_lossy())
    }

    /// 幅
    pub fn width(&self) -> i32 {
        self.raw.w
    }

    /// 高さ
    pub fn height(&self) -> i32 {
        self.raw.h
    }

    /// フレームレート (分子)
    pub fn rate(&self) -> i32 {
        self.raw.rate
    }

    /// スケール (分母)
    pub fn scale(&self) -> i32 {
        self.raw.scale
    }

    /// フレーム数
    pub fn num_frames(&self) -> i32 {
        self.raw.n
    }

    /// 音声サンプリングレート
    pub fn audio_rate(&self) -> i32 {
        self.raw.audio_rate
    }

    /// 音声チャンネル数
    pub fn audio_ch(&self) -> i32 {
        self.raw.audio_ch
    }

    /// 音声サンプリング数
    pub fn audio_samples(&self) -> i32 {
        self.raw.audio_n
    }

    /// 画像データがあるか
    pub fn has_video(&self) -> bool {
        self.raw.flag & sys::OUTPUT_INFO::FLAG_VIDEO != 0
    }

    /// 音声データがあるか
    pub fn has_audio(&self) -> bool {
        self.raw.flag & sys::OUTPUT_INFO::FLAG_AUDIO != 0
    }

    /// 中断チェック
    pub fn is_abort(&self) -> bool {
        self.raw
            .func_is_abort
            .map(|f| unsafe { f() })
            .unwrap_or(false)
    }

    /// 残り時間表示
    pub fn rest_time_disp(&self, now: i32, total: i32) {
        if let Some(f) = self.raw.func_rest_time_disp {
            unsafe { f(now, total) };
        }
    }

    /// データ取得のバッファ数を設定
    pub fn set_buffer_size(&self, video_size: i32, audio_size: i32) {
        if let Some(f) = self.raw.func_set_buffer_size {
            unsafe { f(video_size, audio_size) };
        }
    }

    /// 画像データの生ポインタを取得する
    ///
    /// # Safety
    /// 戻り値のポインタは次に外部関数を使うかホストへ処理を戻すまでのみ有効。
    pub unsafe fn get_video_raw(&self, frame: i32, format: u32) -> Option<*mut c_void> {
        self.raw.func_get_video.map(|f| unsafe { f(frame, format) })
    }

    /// BGRフォーマットのフレームデータをRGBに変換して取得
    pub fn get_video_rgb(&self, frame: i32) -> Option<Vec<u8>> {
        let data_ptr = unsafe { self.get_video_raw(frame, sys::BI_RGB) }?;
        let (w, h) = (self.raw.w as usize, self.raw.h as usize);

        let input_stride = (w * 3).next_multiple_of(4); // RGB24のストライド（4バイト境界アライメント）
        let data_slice =
            unsafe { std::slice::from_raw_parts(data_ptr as *const u8, input_stride * h) };

        Some(crate::convert::bgr_bottomup_to_rgb(data_slice, w, h))
    }

    /// PA64フォーマットのフレームデータをRGBAに変換して取得（アルファチャンネル付き）
    pub fn get_video_rgba(&self, frame: i32) -> Option<Vec<u8>> {
        let data_ptr = unsafe { self.get_video_raw(frame, sys::PA64) }?;
        let (w, h) = (self.raw.w as usize, self.raw.h as usize);

        let data_slice = unsafe { std::slice::from_raw_parts(data_ptr as *const u16, w * h * 4) };

        Some(crate::convert::pa64_to_rgba8(data_slice, w, h))
    }

    /// [`ColorFormat`] に応じてフレームデータを取得する
    pub fn get_video_frame(&self, frame: i32, format: ColorFormat) -> Option<Vec<u8>> {
        match format {
            ColorFormat::Rgb24 => self.get_video_rgb(frame),
            ColorFormat::Rgba32 => self.get_video_rgba(frame),
        }
    }

    /// PCM 16bit形式の音声データを取得する (読み込まれたサンプル数×チャンネル数の長さ)
    pub fn get_audio_pcm16(&self, start: i32, length: i32) -> Option<Vec<i16>> {
        let f = self.raw.func_get_audio?;
        let mut readed = 0;
        let ptr = unsafe { f(start, length, &mut readed, sys::WAVE_FORMAT_PCM) };
        if ptr.is_null() || readed <= 0 {
            return None;
        }
        let len = readed as usize * self.raw.audio_ch.max(1) as usize;
        Some(unsafe { std::slice::from_raw_parts(ptr as *const i16, len) }.to_vec())
    }

    /// PCM (float) 32bit形式の音声データを取得する (読み込まれたサンプル数×チャンネル数の長さ)
    pub fn get_audio_f32(&self, start: i32, length: i32) -> Option<Vec<f32>> {
        let f = self.raw.func_get_audio?;
        let mut readed = 0;
        let ptr = unsafe { f(start, length, &mut readed, sys::WAVE_FORMAT_IEEE_FLOAT) };
        if ptr.is_null() || readed <= 0 {
            return None;
        }
        let len = readed as usize * self.raw.audio_ch.max(1) as usize;
        Some(unsafe { std::slice::from_raw_parts(ptr as *const f32, len) }.to_vec())
    }
}

/// 出力プラグインのフラグ
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PluginFlags(i32);

impl PluginFlags {
    /// 画像をサポートする
    pub const VIDEO: PluginFlags = PluginFlags(sys::OUTPUT_PLUGIN_TABLE::FLAG_VIDEO);
    /// 音声をサポートする
    pub const AUDIO: PluginFlags = PluginFlags(sys::OUTPUT_PLUGIN_TABLE::FLAG_AUDIO);
    /// 静止画出力のみサポートする
    pub const IMAGE: PluginFlags = PluginFlags(sys::OUTPUT_PLUGIN_TABLE::FLAG_IMAGE);
    /// プロジェクトファイルの設定保持をサポートする
    pub const PROJECT_CONFIG: PluginFlags =
        PluginFlags(sys::OUTPUT_PLUGIN_TABLE::FLAG_PROJECT_CONFIG);

    /// 生のフラグ値
    pub const fn as_raw(self) -> i32 {
        self.0
    }
}

impl std::ops::BitOr for PluginFlags {
    type Output = PluginFlags;

    fn bitor(self, rhs: PluginFlags) -> PluginFlags {
        PluginFlags(self.0 | rhs.0)
    }
}

/// `"説明\0パターン\0...\0\0"` 形式のファイルフィルタを構築するビルダー
///
/// ```ignore
/// FileFilter::new()
///     .add("GIF Files (*.gif)", "*.gif")
///     .add("All Files (*)", "*")
/// ```
#[derive(Clone, Default)]
pub struct FileFilter(Vec<(String, String)>);

impl FileFilter {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// 説明とパターンの組を追加する
    pub fn add(mut self, description: impl Into<String>, pattern: impl Into<String>) -> Self {
        self.0.push((description.into(), pattern.into()));
        self
    }

    /// NUL区切り・二重NUL終端のUTF-16へ変換する
    pub(crate) fn to_wide(&self) -> Vec<u16> {
        let mut buffer = Vec::new();
        for (description, pattern) in &self.0 {
            buffer.extend(description.encode_utf16());
            buffer.push(0);
            buffer.extend(pattern.encode_utf16());
            buffer.push(0);
        }
        buffer.push(0);
        buffer
    }
}

/// プラグイン情報
pub struct PluginInfo {
    /// フラグ
    pub flags: PluginFlags,
    /// プラグインの名前
    pub name: String,
    /// ファイルのフィルタ
    pub file_filter: FileFilter,
    /// プラグインの情報
    pub information: String,
}

/// 出力プラグインの実装トレイト
///
/// 実装した型を [`crate::register_output_plugin!`] に渡すことで
/// DLLエクスポート (`DllMain` / `GetOutputPluginTable`) が生成される。
pub trait OutputPlugin {
    /// 出力エラー型。`Err` はマクロ側がメッセージボックス表示してホストへ `false` を返す
    type Error: std::fmt::Display;

    /// 設定ダイアログを持つ場合 `true` (falseなら `func_config` は登録されない)
    const HAS_CONFIG_DIALOG: bool = false;
    /// 出力設定テキストを提供する場合 `true` (falseなら `func_get_config_text` は登録されない)
    const HAS_CONFIG_TEXT: bool = false;

    /// プラグイン情報 (`GetOutputPluginTable` 初回呼び出し時に一度だけ評価される)
    fn info() -> PluginInfo;

    /// 出力処理本体
    fn output(info: &OutputInfo) -> Result<(), Self::Error>;

    /// 設定ダイアログ表示 (`HAS_CONFIG_DIALOG = true` の時のみ呼ばれる)
    fn config(hwnd: HWND, dll_hinst: HINSTANCE) -> bool {
        let _ = (hwnd, dll_hinst);
        true
    }

    /// 出力設定のテキスト情報 (`HAS_CONFIG_TEXT = true` の時のみ呼ばれる)
    fn config_text() -> String {
        String::new()
    }
}
