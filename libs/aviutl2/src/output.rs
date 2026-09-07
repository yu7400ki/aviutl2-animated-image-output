//! 出力プラグインの安全なAPI

use crate::config::IniConfig;
use crate::logger;
use crate::pixel::ColorFormat;
use crate::sys;
use anim_core::FrameDelay;
use std::ffi::c_void;
use std::fs::File;
use std::path::{Path, PathBuf};
use widestring::U16CStr;
use win32_ui::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// u32に収まらない値を、名前を添えたエラーにする
fn to_u32(value: i32, name: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{}が不正です: {}", name, value))
}

/// [`sys::OUTPUT_INFO`] の安全なラッパー
#[derive(Clone, Copy)]
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

    /// 寸法を検めて画像の取り込み口を作る
    pub fn video(&self) -> Result<Video<'a>, String> {
        Ok(Video {
            info: *self,
            width: to_u32(self.raw.w, "幅")?,
            height: to_u32(self.raw.h, "高さ")?,
        })
    }

    /// チャンネル数・サンプリングレート・サンプリング数を検めて音声の取り込み口を作る
    pub fn audio(&self) -> Result<Audio<'a>, String> {
        let channels = to_u32(self.raw.audio_ch, "音声チャンネル数")
            .ok()
            .filter(|&channels| channels > 0)
            .ok_or_else(|| format!("音声チャンネル数が不正です: {}", self.raw.audio_ch))?;
        Ok(Audio {
            info: *self,
            channels,
            rate: to_u32(self.raw.audio_rate, "音声サンプリングレート")?,
            samples: to_u32(self.raw.audio_n, "音声サンプリング数")?,
        })
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
}

/// 検めた寸法に結び付いた画像の取り込み口
///
/// フレームの長さはこの寸法から決まるので、[`OutputInfo::video`] を通った
/// 寸法だけがホストのバッファの読み出しに使われる。
#[derive(Clone, Copy)]
pub struct Video<'a> {
    info: OutputInfo<'a>,
    width: u32,
    height: u32,
}

impl<'a> Video<'a> {
    /// 幅
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 高さ
    pub fn height(&self) -> u32 {
        self.height
    }

    /// フレームを取り込むホストの情報
    pub(crate) fn info(&self) -> OutputInfo<'a> {
        self.info
    }

    /// BGRフォーマットのフレームデータをRGBに変換して取得
    pub fn get_rgb(&self, frame: i32) -> Option<Vec<u8>> {
        let data_ptr = unsafe { self.info.get_video_raw(frame, sys::BI_RGB) }?;
        let (w, h) = (self.width as usize, self.height as usize);

        let input_stride = (w * 3).next_multiple_of(4); // RGB24のストライド（4バイト境界アライメント）
        let data_slice =
            unsafe { std::slice::from_raw_parts(data_ptr as *const u8, input_stride * h) };

        Some(crate::convert::bgr_bottomup_to_rgb(data_slice, w, h))
    }

    /// PA64フォーマットのフレームデータをRGBAに変換して取得（アルファチャンネル付き）
    pub fn get_rgba(&self, frame: i32) -> Option<Vec<u8>> {
        let data_ptr = unsafe { self.info.get_video_raw(frame, sys::PA64) }?;
        let (w, h) = (self.width as usize, self.height as usize);

        let data_slice = unsafe { std::slice::from_raw_parts(data_ptr as *const u16, w * h * 4) };

        Some(crate::convert::pa64_to_rgba8(data_slice, w, h))
    }

    /// [`ColorFormat`] に応じてフレームデータを取得する
    pub fn get_frame(&self, frame: i32, format: ColorFormat) -> Option<Vec<u8>> {
        match format {
            ColorFormat::Rgb24 => self.get_rgb(frame),
            ColorFormat::Rgba32 => self.get_rgba(frame),
        }
    }
}

/// 検めたチャンネル数に結び付いた音声の取り込み口
///
/// サンプルの長さはこのチャンネル数から決まるので、[`OutputInfo::audio`] を
/// 通ったチャンネル数だけがホストのバッファの読み出しに使われる。
#[derive(Clone, Copy)]
pub struct Audio<'a> {
    info: OutputInfo<'a>,
    channels: u32,
    rate: u32,
    samples: u32,
}

impl Audio<'_> {
    /// チャンネル数 (1以上)
    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// サンプリングレート
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// サンプリング数
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// PCM 16bit形式の音声データを取得する (読み込まれたサンプル数×チャンネル数の長さ)
    pub fn get_pcm16(&self, start: i32, length: i32) -> Option<Vec<i16>> {
        let (ptr, len) = self.get_raw(start, length, sys::WAVE_FORMAT_PCM)?;
        Some(unsafe { std::slice::from_raw_parts(ptr as *const i16, len) }.to_vec())
    }

    /// PCM (float) 32bit形式の音声データを取得する (読み込まれたサンプル数×チャンネル数の長さ)
    pub fn get_f32(&self, start: i32, length: i32) -> Option<Vec<f32>> {
        let (ptr, len) = self.get_raw(start, length, sys::WAVE_FORMAT_IEEE_FLOAT)?;
        Some(unsafe { std::slice::from_raw_parts(ptr as *const f32, len) }.to_vec())
    }

    /// 音声データの位置と、そこに並ぶサンプルの数を取得する
    fn get_raw(&self, start: i32, length: i32, format: u32) -> Option<(*mut c_void, usize)> {
        let f = self.info.raw.func_get_audio?;
        let mut readed = 0;
        let ptr = unsafe { f(start, length, &mut readed, format) };
        if ptr.is_null() || readed <= 0 {
            return None;
        }
        Some((ptr, readed as usize * self.channels as usize))
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

/// 設定ダイアログの結末
pub enum ConfigDialog<C> {
    /// 利用者がOKを押し、入力から設定が組み上がった。この設定が保存される
    Accepted(C),
    /// 利用者がキャンセルを押した。設定は元のまま残る
    Cancelled,
    /// ダイアログを出せなかった、または入力から設定を組み上げられなかった。
    /// 設定は元のまま残り、利用者にはエラーが報される
    Failed,
}

/// 出力プラグインの実装トレイト
///
/// 実装した型を [`crate::register_output_plugin!`] に渡すことで
/// DLLエクスポート (`DllMain` / `GetOutputPluginTable`) が生成される。
pub trait OutputPlugin {
    /// iniへ永続化する設定
    type Config: IniConfig;

    /// エラー文言に載せる形式名 (「GIF出力エラー」の「GIF」)
    const FORMAT_NAME: &'static str;

    /// 設定ダイアログを持つ場合 `true` (falseなら `func_config` は登録されない)
    const HAS_CONFIG_DIALOG: bool = false;
    /// 出力設定テキストを提供する場合 `true` (falseなら `func_get_config_text` は登録されない)
    const HAS_CONFIG_TEXT: bool = false;

    /// プラグイン情報 (`GetOutputPluginTable` 初回呼び出し時に一度だけ評価される)
    fn info() -> PluginInfo;

    /// 設定に従って保存先へ書き出す
    fn encode(info: &OutputInfo, config: &Self::Config) -> Result<(), String>;

    /// 設定ダイアログを表示する (`HAS_CONFIG_DIALOG = true` の時のみ呼ばれる)
    fn show_config_dialog(hwnd: HWND, config: Self::Config) -> ConfigDialog<Self::Config> {
        let _ = (hwnd, config);
        ConfigDialog::Cancelled
    }

    /// 出力処理本体
    ///
    /// `Err` はマクロ側がメッセージボックス表示してホストへ `false` を返す。
    fn output(info: &OutputInfo) -> Result<(), String> {
        let config = Self::Config::load();
        Self::encode(info, &config).map_err(|e| format!("{}出力エラー: {}", Self::FORMAT_NAME, e))
    }

    /// 設定ダイアログを表示して、決まった設定を保存する
    ///
    /// 戻り値は設定が決まったかどうか。`HAS_CONFIG_DIALOG = true` の時のみ呼ばれる。
    fn config(hwnd: HWND, _dll_hinst: HINSTANCE) -> bool {
        let config = match Self::show_config_dialog(hwnd, Self::Config::load()) {
            ConfigDialog::Accepted(config) => config,
            ConfigDialog::Cancelled => return false,
            ConfigDialog::Failed => {
                logger::error("設定の取得に失敗しました。");
                MessageBox::error(Some(hwnd), "設定の取得に失敗しました。", "エラー");
                return false;
            }
        };

        if let Err(e) = config.save() {
            let error_msg = format!("設定保存エラー: {}", e);
            logger::warn(&error_msg);
            MessageBox::warning(Some(hwnd), &error_msg, "警告");
        }
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
pub fn write_or_discard<T, F>(path: &Path, write: F) -> std::result::Result<T, String>
where
    F: FnOnce(File) -> std::result::Result<T, String>,
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

    /// 音声の欄だけを持つ `OUTPUT_INFO`
    fn raw_audio(rate: i32, ch: i32, n: i32) -> sys::OUTPUT_INFO {
        sys::OUTPUT_INFO {
            audio_rate: rate,
            audio_ch: ch,
            audio_n: n,
            ..raw_info(1920, 1080, 30, 1, 24)
        }
    }

    fn info(raw: &sys::OUTPUT_INFO) -> OutputInfo<'_> {
        unsafe { OutputInfo::from_raw(raw) }.expect("OUTPUT_INFOがnull")
    }

    #[test]
    fn the_checked_values_carry_the_fields_as_they_are() {
        let raw = raw_info(1920, 1080, 30000, 1001, 24);
        let info = info(&raw);
        let video = info.video().unwrap();

        assert_eq!(video.width(), 1920);
        assert_eq!(video.height(), 1080);
        assert_eq!(info.rate().unwrap(), 30000);
        assert_eq!(info.scale().unwrap(), 1001);
        assert_eq!(info.num_frames().unwrap(), 24);
    }

    #[test]
    fn a_negative_value_is_rejected_with_the_name_of_the_field() {
        let raw = raw_info(-1, -2, -3, -4, -5);
        let info = info(&raw);

        assert_eq!(info.rate().unwrap_err(), "フレームレートが不正です: -3");
        assert_eq!(
            info.scale().unwrap_err(),
            "フレームレートのスケールが不正です: -4"
        );
        assert_eq!(info.num_frames().unwrap_err(), "フレーム数が不正です: -5");
    }

    /// 寸法が負なら取り込み口を作れないので、フレームの長さもそこから決まらない
    #[test]
    fn a_negative_dimension_keeps_the_frames_out_of_reach() {
        let negative_width = raw_info(-1, 1080, 30, 1, 24);
        let negative_height = raw_info(1920, -2, 30, 1, 24);

        assert_eq!(
            info(&negative_width)
                .video()
                .err()
                .expect("寸法を検めていない"),
            "幅が不正です: -1"
        );
        assert_eq!(
            info(&negative_height)
                .video()
                .err()
                .expect("寸法を検めていない"),
            "高さが不正です: -2"
        );
    }

    /// 音声の欄も検めてからでないと取り込み口にならない
    #[test]
    fn the_checked_audio_fields_carry_themselves_as_they_are() {
        let raw = raw_audio(48000, 2, 96000);
        let audio = info(&raw).audio().unwrap();

        assert_eq!(audio.channels(), 2);
        assert_eq!(audio.rate(), 48000);
        assert_eq!(audio.samples(), 96000);
    }

    #[test]
    fn a_channel_count_that_names_no_sample_keeps_the_audio_out_of_reach() {
        let no_channel = raw_audio(48000, 0, 96000);
        let negative_channel = raw_audio(48000, -2, 96000);
        let negative_rate = raw_audio(-48000, 2, 96000);
        let negative_samples = raw_audio(48000, 2, -96000);

        assert_eq!(
            info(&no_channel)
                .audio()
                .err()
                .expect("音声の欄を検めていない"),
            "音声チャンネル数が不正です: 0"
        );
        assert_eq!(
            info(&negative_channel)
                .audio()
                .err()
                .expect("音声の欄を検めていない"),
            "音声チャンネル数が不正です: -2"
        );
        assert_eq!(
            info(&negative_rate)
                .audio()
                .err()
                .expect("音声の欄を検めていない"),
            "音声サンプリングレートが不正です: -48000"
        );
        assert_eq!(
            info(&negative_samples)
                .audio()
                .err()
                .expect("音声の欄を検めていない"),
            "音声サンプリング数が不正です: -96000"
        );
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
