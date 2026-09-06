//! 出力プラグインの安全なAPI

use crate::pixel::ColorFormat;
use crate::sys;
use anim_core::FrameDelay;
use std::ffi::c_void;
use std::fs::File;
use std::path::{Path, PathBuf};
use widestring::U16CStr;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// u32に収まらない値を、名前を添えたエラーにする
fn to_u32(value: i32, name: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{}が不正です: {}", name, value))
}

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

    /// 検めた幅
    pub fn width(&self) -> Result<u32, String> {
        to_u32(self.raw.w, "幅")
    }

    /// 検めた高さ
    pub fn height(&self) -> Result<u32, String> {
        to_u32(self.raw.h, "高さ")
    }

    /// 検めたフレームレート (分子)
    pub fn rate(&self) -> Result<u32, String> {
        to_u32(self.raw.rate, "フレームレート")
    }

    /// 検めたスケール (分母)
    pub fn scale(&self) -> Result<u32, String> {
        to_u32(self.raw.scale, "フレームレートのスケール")
    }

    /// 検めたフレーム数
    pub fn num_frames(&self) -> Result<u32, String> {
        to_u32(self.raw.n, "フレーム数")
    }

    /// ホストが渡したままのフレーム数
    pub(crate) fn raw_num_frames(&self) -> i32 {
        self.raw.n
    }

    /// 1フレームの表示時間 (scale / rate 秒)
    pub fn frame_delay(&self) -> Result<FrameDelay, String> {
        let scale = self.scale()?;
        let rate = self.rate()?;
        FrameDelay::new(scale, rate).map_err(|e| format!("フレームレート設定エラー: {}", e))
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

    /// 画像データの生ポインタを取得する (ホストが返さなければ`None`)
    ///
    /// # Safety
    /// 戻り値のポインタは次に外部関数を使うかホストへ処理を戻すまでのみ有効。
    pub unsafe fn get_video_raw(&self, frame: i32, format: u32) -> Option<*mut c_void> {
        let f = self.raw.func_get_video?;
        let ptr = unsafe { f(frame, format) };
        (!ptr.is_null()).then_some(ptr)
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

/// `path` を作って `write` へ渡し、失敗したら書きかけのファイルを消す
///
/// `write` はファイルを持ったまま呼ばれ、戻るときに閉じる。開いたまま消すと
/// 削除は最後のハンドルが閉じるまで効かないため、閉じてから消す。
pub fn write_or_discard<F>(path: &Path, write: F) -> std::result::Result<(), String>
where
    F: FnOnce(File) -> std::result::Result<(), String>,
{
    let file = File::create(path).map_err(|e| format!("ファイル作成エラー: {}", e))?;

    write(file).map_err(|error| match std::fs::remove_file(path) {
        Ok(()) => error,
        Err(e) => format!("{} (書きかけのファイルが残りました: {})", error, e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 寸法と刻みだけを持つ `OUTPUT_INFO`
    fn raw_info(w: i32, h: i32, rate: i32, scale: i32, n: i32) -> sys::OUTPUT_INFO {
        sys::OUTPUT_INFO {
            flag: sys::OUTPUT_INFO::FLAG_VIDEO,
            w,
            h,
            rate,
            scale,
            n,
            audio_rate: 0,
            audio_ch: 0,
            audio_n: 0,
            savefile: std::ptr::null(),
            func_get_video: None,
            func_get_audio: None,
            func_is_abort: None,
            func_rest_time_disp: None,
            func_set_buffer_size: None,
        }
    }

    fn info(raw: &sys::OUTPUT_INFO) -> OutputInfo<'_> {
        unsafe { OutputInfo::from_raw(raw) }.expect("OUTPUT_INFOがnull")
    }

    #[test]
    fn the_checked_values_carry_the_fields_as_they_are() {
        let raw = raw_info(1920, 1080, 30000, 1001, 24);
        let info = info(&raw);

        assert_eq!(info.width().unwrap(), 1920);
        assert_eq!(info.height().unwrap(), 1080);
        assert_eq!(info.rate().unwrap(), 30000);
        assert_eq!(info.scale().unwrap(), 1001);
        assert_eq!(info.num_frames().unwrap(), 24);
    }

    #[test]
    fn a_negative_value_is_rejected_with_the_name_of_the_field() {
        let raw = raw_info(-1, -2, -3, -4, -5);
        let info = info(&raw);

        assert_eq!(info.width().unwrap_err(), "幅が不正です: -1");
        assert_eq!(info.height().unwrap_err(), "高さが不正です: -2");
        assert_eq!(info.rate().unwrap_err(), "フレームレートが不正です: -3");
        assert_eq!(
            info.scale().unwrap_err(),
            "フレームレートのスケールが不正です: -4"
        );
        assert_eq!(info.num_frames().unwrap_err(), "フレーム数が不正です: -5");
    }

    /// 1フレームはscale / rate秒なので、スケールが分子、レートが分母
    #[test]
    fn the_frame_rate_becomes_a_delay_in_seconds() {
        let raw = raw_info(1920, 1080, 30000, 1001, 24);
        let delay = info(&raw).frame_delay().unwrap();

        assert_eq!((delay.numerator(), delay.denominator()), (1001, 30000));
    }

    #[test]
    fn a_frame_rate_of_zero_is_rejected() {
        let raw = raw_info(1920, 1080, 0, 1, 24);

        assert_eq!(
            info(&raw).frame_delay().unwrap_err(),
            "フレームレート設定エラー: フレーム遅延の分母が0です"
        );
    }

    #[test]
    fn a_negative_rate_or_scale_is_rejected() {
        let negative_rate = raw_info(1920, 1080, -30, 1, 24);
        let negative_scale = raw_info(1920, 1080, 30, -1, 24);

        assert_eq!(
            info(&negative_rate).frame_delay().unwrap_err(),
            "フレームレートが不正です: -30"
        );
        assert_eq!(
            info(&negative_scale).frame_delay().unwrap_err(),
            "フレームレートのスケールが不正です: -1"
        );
    }
}
