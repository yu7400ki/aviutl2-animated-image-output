//! `avifEncoder` のRAIIと全体の駆動

use crate::error::{EncodingError, Error};
use crate::image::{Image, RwData};
use crate::layout::Layout;
use crate::{Config, YuvFormat};
use avif_sys::{
    AVIF_ADD_IMAGE_FLAG_NONE, AVIF_ADD_IMAGE_FLAG_SINGLE, AVIF_REPETITION_COUNT_INFINITE,
    AVIF_RESULT_OK, AVIF_RESULT_OUT_OF_MEMORY, avifEncoder, avifEncoderAddImage, avifEncoderCreate,
    avifEncoderDestroy, avifEncoderFinish,
};
use std::ffi::{CStr, c_int};
use std::io::Write;

/// aomの動作の用途
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    AllIntra,
    GoodQuality,
    Realtime,
}

/// `speed` から解決されるaomの動作点
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperatingPoint {
    pub usage: Usage,
    pub cpu_used: u8,
}

impl Config {
    /// 符号化に使われるaomの動作点
    ///
    /// `single` は単葉として符号化するとき。
    pub fn operating_point(&self, single: bool) -> OperatingPoint {
        let usage = if single {
            Usage::AllIntra
        } else if self.speed >= 7 {
            Usage::Realtime
        } else {
            Usage::GoodQuality
        };
        OperatingPoint {
            usage,
            cpu_used: self.speed.min(9),
        }
    }
}

/// 再生回数を `avifEncoder::repetitionCount` の「追加の繰り返し回数」へ写す
///
/// 0は無限。n回の再生はn-1回の繰り返しで、欄に収まらない値は飽和させる。
fn repetition_count(num_plays: u32) -> c_int {
    match num_plays {
        0 => AVIF_REPETITION_COUNT_INFINITE,
        plays => c_int::try_from(plays - 1).unwrap_or(c_int::MAX),
    }
}

/// `avifEncoder` のRAII
struct Raw {
    ptr: *mut avifEncoder,
}

impl Raw {
    /// 設定を写した符号化器を作る
    fn new(config: &Config) -> Result<Self, Error> {
        let ptr = unsafe { avifEncoderCreate() };
        if ptr.is_null() {
            return Err(Error::Encode(EncodingError::new(
                AVIF_RESULT_OUT_OF_MEMORY,
                String::new(),
            )));
        }
        let raw = Raw { ptr };

        let encoder = unsafe { &mut *ptr };
        encoder.quality = c_int::from(config.quality);
        encoder.qualityAlpha = c_int::from(config.quality);
        encoder.speed = c_int::from(config.speed);
        encoder.maxThreads = c_int::try_from(config.max_threads).unwrap_or(c_int::MAX);
        encoder.timescale = u64::from(config.timescale);
        encoder.repetitionCount = repetition_count(config.num_plays);

        Ok(raw)
    }

    /// 失敗した `avifResult` に、符号化器が書いた原因を添えて写す
    fn check(&self, result: c_int) -> Result<(), Error> {
        if result == AVIF_RESULT_OK {
            return Ok(());
        }
        let detail = unsafe { CStr::from_ptr((*self.ptr).diag.error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Err(Error::Encode(EncodingError::new(result, detail)))
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe { avifEncoderDestroy(self.ptr) };
    }
}

/// AVIFのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
///
/// フレーム数が2以上ならシーケンスになり、1なら単葉の静止画AVIFになる。
/// 符号化した内容はlibavifが内部に溜め、[`Encoder::finish`] が組み立てた
/// ファイル全体を一括で書き出す。
pub struct Encoder<W: Write> {
    writer: W,
    raw: Raw,
    layout: Layout,
    yuv_format: YuvFormat,
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
}

// SAFETY: 抱える生ポインタは唯一の所有で別名を持たず、libavifとaomの符号化経路は
// スレッド固有の状態を持たない。同時アクセスは [`Sync`] を付けないことで防ぐ。
unsafe impl<W: Write + Send> Send for Encoder<W> {}

impl<W: Write> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// # Errors
    /// フレーム数が0のとき [`Error::InvalidFrameCount`]。時間刻み数が0のとき
    /// [`Error::InvalidTimescale`]。寸法が0か行間が欄に収まらないとき
    /// [`Error::InvalidDimensions`]。符号化器を確保できないとき [`Error::Encode`]。
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        num_frames: u32,
        config: Config,
    ) -> Result<Self, Error> {
        if num_frames == 0 {
            return Err(Error::InvalidFrameCount);
        }
        if config.timescale == 0 {
            return Err(Error::InvalidTimescale);
        }
        let layout = Layout::new(width, height, config.color_type)?;
        let raw = Raw::new(&config)?;

        Ok(Encoder {
            writer,
            raw,
            layout,
            yuv_format: config.yuv_format,
            num_frames,
            frames_accepted: 0,
        })
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく並んで
    /// いること。`duration` は [`Config::timescale`] 刻みの表示時間で、単葉では
    /// 書かれない。
    ///
    /// # Errors
    /// 宣言したフレーム数を超えたとき [`Error::FrameCountMismatch`]。表示時間が
    /// 0のとき [`Error::InvalidDuration`]。バイト数が寸法と色種別から決まる長さと
    /// 違うとき [`Error::FrameSizeMismatch`]。符号化に失敗したとき
    /// [`Error::Encode`]。
    pub fn add_frame(&mut self, data: &[u8], duration: u32) -> Result<(), Error> {
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }
        if duration == 0 {
            return Err(Error::InvalidDuration);
        }

        let image = Image::import(data, &self.layout, self.yuv_format)?;
        let flags = if self.num_frames == 1 {
            AVIF_ADD_IMAGE_FLAG_SINGLE
        } else {
            AVIF_ADD_IMAGE_FLAG_NONE
        };
        let result = unsafe {
            avifEncoderAddImage(self.raw.ptr, image.as_ptr(), u64::from(duration), flags)
        };
        self.raw.check(result)?;

        self.frames_accepted += 1;
        Ok(())
    }

    /// ファイル全体を組み立てて `writer` へ書き切る
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。組み立てに失敗したとき [`Error::Encode`]。
    /// 書き出しに失敗したとき [`Error::Io`]。
    pub fn finish(self) -> Result<W, Error> {
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        let Encoder {
            mut writer, raw, ..
        } = self;

        let mut output = RwData::new();
        raw.check(unsafe { avifEncoderFinish(raw.ptr, output.as_mut_ptr()) })?;

        writer.write_all(output.as_slice())?;
        writer.flush()?;
        Ok(writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColorType;
    use std::fs::File;
    use std::io::{BufWriter, Cursor};

    fn config() -> Config {
        Config {
            color_type: ColorType::Rgb8,
            quality: 60,
            speed: 8,
            yuv_format: YuvFormat::Yuv420,
            num_plays: 3,
            timescale: 30000,
            max_threads: 4,
        }
    }

    /// 符号化を作業スレッドへ渡せる
    #[test]
    fn the_encoder_moves_across_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<Encoder<BufWriter<File>>>();
    }

    /// 符号化器へ書いた項目を読み戻す
    #[test]
    fn the_configuration_reaches_the_encoder() {
        let raw = Raw::new(&config()).unwrap();
        let encoder = unsafe { &*raw.ptr };
        assert_eq!(encoder.quality, 60);
        assert_eq!(encoder.qualityAlpha, 60, "αの品質が色と揃っていない");
        assert_eq!(encoder.speed, 8);
        assert_eq!(encoder.maxThreads, 4);
        assert_eq!(encoder.timescale, 30000);
        assert_eq!(encoder.repetitionCount, 2);
    }

    /// 上限を超えるスレッド数は欄の上限で止める
    #[test]
    fn an_unrepresentable_thread_count_is_saturated() {
        let raw = Raw::new(&Config {
            max_threads: u32::MAX,
            ..config()
        })
        .unwrap();
        assert_eq!(unsafe { &*raw.ptr }.maxThreads, c_int::MAX);
    }

    #[test]
    fn the_play_count_becomes_the_number_of_extra_repetitions() {
        assert_eq!(repetition_count(0), AVIF_REPETITION_COUNT_INFINITE);
        assert_eq!(repetition_count(1), 0);
        assert_eq!(repetition_count(2), 1);
        assert_eq!(repetition_count(1000), 999);
    }

    /// 引き算のまま渡すと負へ折り返し、`avifEncoderFinish` が弾く値になる
    #[test]
    fn a_play_count_beyond_the_field_is_saturated() {
        assert_eq!(repetition_count(u32::MAX), c_int::MAX);
        assert_eq!(repetition_count(i32::MAX as u32 + 1), c_int::MAX);
        assert_eq!(repetition_count(i32::MAX as u32), c_int::MAX - 1);
    }

    #[test]
    fn a_still_leaf_always_resolves_to_all_intra() {
        for speed in 0..=10 {
            let point = Config { speed, ..config() }.operating_point(true);
            assert_eq!(point.usage, Usage::AllIntra, "speed {speed}");
            assert_eq!(point.cpu_used, speed.min(9), "speed {speed}");
        }
    }

    #[test]
    fn a_sequence_falls_to_realtime_from_speed_seven() {
        let usage = |speed| Config { speed, ..config() }.operating_point(false).usage;
        for speed in 0..=6 {
            assert_eq!(usage(speed), Usage::GoodQuality, "speed {speed}");
        }
        for speed in 7..=10 {
            assert_eq!(usage(speed), Usage::Realtime, "speed {speed}");
        }
    }

    #[test]
    fn the_cpu_used_stops_at_nine() {
        let cpu_used = |speed| Config { speed, ..config() }.operating_point(false).cpu_used;
        assert_eq!(cpu_used(0), 0);
        assert_eq!(cpu_used(9), 9);
        assert_eq!(cpu_used(10), 9);
    }

    fn encoder(num_frames: u32, config: Config) -> Result<Encoder<Cursor<Vec<u8>>>, Error> {
        Encoder::new(Cursor::new(Vec::new()), 16, 16, num_frames, config)
    }

    #[test]
    fn a_zero_frame_count_is_rejected() {
        assert!(matches!(
            encoder(0, config()),
            Err(Error::InvalidFrameCount)
        ));
    }

    #[test]
    fn a_zero_timescale_is_rejected() {
        assert!(matches!(
            encoder(
                1,
                Config {
                    timescale: 0,
                    ..config()
                }
            ),
            Err(Error::InvalidTimescale)
        ));
    }

    #[test]
    fn a_zero_duration_is_rejected() {
        let mut encoder = encoder(2, config()).unwrap();
        assert!(matches!(
            encoder.add_frame(&[0; 16 * 16 * 3], 0),
            Err(Error::InvalidDuration)
        ));
    }

    #[test]
    fn a_frame_of_another_length_is_rejected() {
        let mut encoder = encoder(2, config()).unwrap();
        assert!(matches!(
            encoder.add_frame(&[0; 16 * 16 * 4], 1),
            Err(Error::FrameSizeMismatch {
                expected: 768,
                actual: 1024
            })
        ));
    }

    #[test]
    fn more_frames_than_declared_are_rejected() {
        let mut encoder = encoder(1, config()).unwrap();
        encoder.add_frame(&[0; 16 * 16 * 3], 1).unwrap();
        assert!(matches!(
            encoder.add_frame(&[0; 16 * 16 * 3], 1),
            Err(Error::FrameCountMismatch {
                expected: 1,
                actual: 2
            })
        ));
    }

    #[test]
    fn fewer_frames_than_declared_are_rejected() {
        let mut encoder = encoder(3, config()).unwrap();
        encoder.add_frame(&[0; 16 * 16 * 3], 1).unwrap();
        assert!(matches!(
            encoder.finish(),
            Err(Error::FrameCountMismatch {
                expected: 3,
                actual: 1
            })
        ));
    }
}
