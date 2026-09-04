//! `JxlEncoder` と並列実行器のRAII、および全体の駆動

use crate::delta::{Delta, Pending, Region};
use crate::error::{EncodingError, Error};
use crate::layout::Layout;
use crate::{Config, EFFORT_RANGE, QUALITY_RANGE};
use jxl_sys::{
    JXL_ENC_ERR_OOM, JXL_ENC_ERROR, JXL_ENC_FRAME_SETTING_EFFORT, JXL_ENC_NEED_MORE_OUTPUT,
    JXL_ENC_SUCCESS, JXL_FALSE, JXL_NATIVE_ENDIAN, JXL_TRUE, JXL_TYPE_UINT8, JxlBasicInfo,
    JxlColorEncoding, JxlColorEncodingSetToSRGB, JxlEncoder, JxlEncoderAddImageFrame,
    JxlEncoderCloseInput, JxlEncoderCreate, JxlEncoderDestroy, JxlEncoderDistanceFromQuality,
    JxlEncoderFrameSettings, JxlEncoderFrameSettingsCreate, JxlEncoderFrameSettingsSetOption,
    JxlEncoderGetError, JxlEncoderInitBasicInfo, JxlEncoderInitFrameHeader,
    JxlEncoderProcessOutput, JxlEncoderSetBasicInfo, JxlEncoderSetColorEncoding,
    JxlEncoderSetFrameDistance, JxlEncoderSetFrameHeader, JxlEncoderSetFrameLossless,
    JxlEncoderSetParallelRunner, JxlFrameHeader, JxlPixelFormat, JxlThreadParallelRunner,
    JxlThreadParallelRunnerCreate, JxlThreadParallelRunnerDestroy,
};
use std::ffi::{c_int, c_void};
use std::io::Write;
use std::mem::MaybeUninit;
use std::ops::RangeInclusive;
use std::ptr;

/// 1度の排水で受け取るバイト数
const OUTPUT_CHUNK: usize = 64 * 1024;

/// フレームが土台にし、合成後のキャンバスを置く参照スロット
const REFERENCE_SLOT: u32 = 1;

/// `JxlAnimationHeader::tps_numerator` が採れる値
const TPS_NUMERATOR_RANGE: RangeInclusive<u32> = 1..=(1 << 30);

/// `JxlAnimationHeader::tps_denominator` が採れる値
const TPS_DENOMINATOR_RANGE: RangeInclusive<u32> = 1..=1024;

/// `JXL_BOOL` へ写す
fn jxl_bool(value: bool) -> c_int {
    if value { JXL_TRUE } else { JXL_FALSE }
}

/// 確保に失敗したときのエラー
fn allocation_failed() -> Error {
    Error::Encode(EncodingError::new(JXL_ENC_ERROR, JXL_ENC_ERR_OOM))
}

/// 最大公約数
fn gcd(a: u32, b: u32) -> u32 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// アニメーションヘッダへ書ける1秒あたりのtick数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tps {
    numerator: u32,
    denominator: u32,
}

impl Tps {
    /// 最大公約数で約した比を作る
    ///
    /// 約分は比を変えないので、1 tick の長さも表示時間の意味も動かない。
    ///
    /// # Errors
    /// 0を含むか、約した比がヘッダの値域に収まらないとき [`Error::InvalidTps`]。
    fn new(numerator: u32, denominator: u32) -> Result<Self, Error> {
        let invalid = || Error::InvalidTps {
            numerator,
            denominator,
        };
        if numerator == 0 || denominator == 0 {
            return Err(invalid());
        }

        let divisor = gcd(numerator, denominator);
        let tps = Tps {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        };

        if !TPS_NUMERATOR_RANGE.contains(&tps.numerator) {
            return Err(invalid());
        }
        if !TPS_DENOMINATOR_RANGE.contains(&tps.denominator) {
            return Err(invalid());
        }
        Ok(tps)
    }
}

impl Config {
    /// 可逆で符号化するか
    ///
    /// 品質の上限は距離0に写り、libjxlが可逆へ倒す。
    pub(crate) fn is_lossless(&self) -> bool {
        self.quality >= *QUALITY_RANGE.end()
    }
}

/// `JxlEncoder` と、それが参照する並列実行器のRAII
struct Raw {
    enc: *mut JxlEncoder,
    runner: *mut c_void,
}

impl Raw {
    /// `max_threads` の作業スレッドを持つ符号化器を作る
    fn new(max_threads: u32) -> Result<Self, Error> {
        let runner = unsafe { JxlThreadParallelRunnerCreate(ptr::null(), max_threads as usize) };
        if runner.is_null() {
            return Err(allocation_failed());
        }
        let enc = unsafe { JxlEncoderCreate(ptr::null()) };
        if enc.is_null() {
            unsafe { JxlThreadParallelRunnerDestroy(runner) };
            return Err(allocation_failed());
        }

        let raw = Raw { enc, runner };
        raw.check(unsafe {
            JxlEncoderSetParallelRunner(raw.enc, Some(JxlThreadParallelRunner), raw.runner)
        })?;
        Ok(raw)
    }

    /// 失敗した `JxlEncoderStatus` に、符号化器が保持する内訳を添えて写す
    fn check(&self, status: c_int) -> Result<(), Error> {
        if status == JXL_ENC_SUCCESS {
            return Ok(());
        }
        let code = unsafe { JxlEncoderGetError(self.enc) };
        Err(Error::Encode(EncodingError::new(status, code)))
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            JxlEncoderDestroy(self.enc);
            JxlThreadParallelRunnerDestroy(self.runner);
        }
    }
}

/// JPEG XLのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
///
/// フレーム数が2以上ならアニメーションになり、1なら静止画になる。
/// `writer` への書き出しは1フレーム遅れる。
///
/// キャンバス2面ぶんまでの画素を抱える (1920x1080のRGBA8で約16.6MB)。
pub struct Encoder<W: Write> {
    writer: W,
    raw: Raw,
    /// 所有は [`Raw::enc`] にある
    settings: *mut JxlEncoderFrameSettings,
    format: JxlPixelFormat,
    layout: Layout,
    /// 書き出しを待っているフレームとキャンバスの追跡
    delta: Delta,
    num_frames: u32,
    /// [`Self::add_frame`] が受け付けたフレーム数
    frames_accepted: u32,
    /// 排水の受け皿
    chunk: Vec<u8>,
}

// SAFETY: 抱える生ポインタは唯一の所有で別名を持たず、libjxl の符号化経路は
// スレッド固有の状態を持たない。同時アクセスは Sync を付けないことで防ぐ。
unsafe impl<W: Write + Send> Send for Encoder<W> {}

impl<W: Write> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// # Errors
    /// フレーム数が0のとき [`Error::InvalidFrameCount`]。1秒あたりのtick数が
    /// 0を含むか、約した比がヘッダの値域に収まらないとき
    /// [`Error::InvalidTps`]。品質が [`QUALITY_RANGE`]
    /// の外のとき [`Error::InvalidQuality`]。均衡が [`EFFORT_RANGE`] の外のとき
    /// [`Error::InvalidEffort`]。寸法が0のとき [`Error::InvalidDimensions`]。
    /// 符号化器を組み立てられないとき [`Error::Encode`]。
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
        let tps = Tps::new(config.tps_numerator, config.tps_denominator)?;
        if !QUALITY_RANGE.contains(&config.quality) {
            return Err(Error::InvalidQuality {
                quality: config.quality,
            });
        }
        if !EFFORT_RANGE.contains(&config.effort) {
            return Err(Error::InvalidEffort {
                effort: config.effort,
            });
        }
        let layout = Layout::new(width, height, config.color_type)?;
        let raw = Raw::new(config.max_threads)?;
        let lossless = config.is_lossless();

        let mut info = MaybeUninit::<JxlBasicInfo>::zeroed();
        unsafe { JxlEncoderInitBasicInfo(info.as_mut_ptr()) };
        let mut info = unsafe { info.assume_init() };
        info.xsize = layout.width;
        info.ysize = layout.height;
        info.bits_per_sample = 8;
        info.num_color_channels = 3;
        info.uses_original_profile = jxl_bool(lossless);
        if layout.color_type.has_alpha() {
            info.num_extra_channels = 1;
            info.alpha_bits = 8;
            info.alpha_exponent_bits = 0;
            info.alpha_premultiplied = JXL_FALSE;
        }
        if num_frames > 1 {
            info.have_animation = JXL_TRUE;
            info.animation.tps_numerator = tps.numerator;
            info.animation.tps_denominator = tps.denominator;
            info.animation.num_loops = config.num_plays;
            info.animation.have_timecodes = JXL_FALSE;
        }
        raw.check(unsafe { JxlEncoderSetBasicInfo(raw.enc, &info) })?;

        let mut color = MaybeUninit::<JxlColorEncoding>::zeroed();
        unsafe { JxlColorEncodingSetToSRGB(color.as_mut_ptr(), JXL_FALSE) };
        let color = unsafe { color.assume_init() };
        raw.check(unsafe { JxlEncoderSetColorEncoding(raw.enc, &color) })?;

        let settings = unsafe { JxlEncoderFrameSettingsCreate(raw.enc, ptr::null()) };
        if settings.is_null() {
            return Err(allocation_failed());
        }
        raw.check(unsafe {
            JxlEncoderFrameSettingsSetOption(
                settings,
                JXL_ENC_FRAME_SETTING_EFFORT,
                i64::from(config.effort),
            )
        })?;
        raw.check(unsafe {
            JxlEncoderSetFrameDistance(settings, JxlEncoderDistanceFromQuality(config.quality))
        })?;
        raw.check(unsafe { JxlEncoderSetFrameLossless(settings, jxl_bool(lossless)) })?;

        Ok(Encoder {
            writer,
            raw,
            settings,
            format: JxlPixelFormat {
                num_channels: layout.color_type.num_channels(),
                data_type: JXL_TYPE_UINT8,
                endianness: JXL_NATIVE_ENDIAN,
                align: 0,
            },
            layout,
            delta: Delta::new(),
            num_frames,
            frames_accepted: 0,
            chunk: vec![0; OUTPUT_CHUNK],
        })
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく並んで
    /// いること。`duration` は [`Config::tps_numerator`] と
    /// [`Config::tps_denominator`] が決める tick 数の表示時間で、静止画では
    /// 書かれない。
    ///
    /// 投入されたフレームはその場では書き出さず、書き直す範囲が決まる次の
    /// 呼び出し、または [`Encoder::finish`] で書き出す。直前に投入されたフレームと
    /// 一致する内容は、そちらの表示時間へ畳まれる。
    ///
    /// # Errors
    /// 宣言したフレーム数を超えたとき [`Error::FrameCountMismatch`]。表示時間が
    /// 0のとき [`Error::InvalidDuration`]。バイト数が寸法と色種別から決まる長さと
    /// 違うとき [`Error::FrameSizeMismatch`]。符号化に失敗したとき
    /// [`Error::Encode`]。書き出しに失敗したとき [`Error::Io`]。
    ///
    /// 符号化と書き出しの失敗は1つ前に投入されたフレームのものになる。最後に
    /// 投入したフレームの書き出しは [`Encoder::finish`] で報告される。
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
        self.layout.check_frame(data)?;

        self.frames_accepted += 1;
        if let Some(pending) = self.delta.advance(&self.layout, data, duration) {
            self.write(pending, false)?;
        }
        Ok(())
    }

    /// 取り出したフレームを符号化し、書き出せる分を `writer` へ流す
    ///
    /// 矩形へ割れたフレームは、表示時間を持つ1枚が最後に来るよう順に書く。
    /// `last` はこれがストリームの最後のフレームであることを表す。
    fn write(&mut self, pending: Pending, last: bool) -> Result<(), Error> {
        let mut offset = 0;
        let mut frames = pending.frames().peekable();
        while let Some((region, duration)) = frames.next() {
            let mut header = MaybeUninit::<JxlFrameHeader>::zeroed();
            unsafe { JxlEncoderInitFrameHeader(header.as_mut_ptr()) };
            let mut header = unsafe { header.assume_init() };
            header.duration = duration;
            header.layer_info.blend_info.source = REFERENCE_SLOT;
            header.layer_info.save_as_reference = REFERENCE_SLOT;
            if let Region::Part(rect) = region {
                header.layer_info.have_crop = JXL_TRUE;
                header.layer_info.crop_x0 = rect.x as i32;
                header.layer_info.crop_y0 = rect.y as i32;
                header.layer_info.xsize = rect.width;
                header.layer_info.ysize = rect.height;
            }
            self.raw
                .check(unsafe { JxlEncoderSetFrameHeader(self.settings, &header) })?;

            let len = region.byte_len(&self.layout);
            let pixels = &self.delta.pixels()[offset..offset + len];
            self.raw.check(unsafe {
                JxlEncoderAddImageFrame(
                    self.settings,
                    &self.format,
                    pixels.as_ptr().cast::<c_void>(),
                    pixels.len(),
                )
            })?;
            offset += len;

            // 最後のフレームかどうかは排水した時点の状態で焼き込まれる
            if last && frames.peek().is_none() {
                unsafe { JxlEncoderCloseInput(self.raw.enc) };
            }
            self.drain()?;
        }
        Ok(())
    }

    /// 書き出せる分を `writer` へ流し切る
    fn drain(&mut self) -> Result<(), Error> {
        loop {
            let mut next_out = self.chunk.as_mut_ptr();
            let mut avail_out = self.chunk.len();
            let status =
                unsafe { JxlEncoderProcessOutput(self.raw.enc, &mut next_out, &mut avail_out) };
            let written = self.chunk.len() - avail_out;
            self.writer.write_all(&self.chunk[..written])?;

            match status {
                JXL_ENC_SUCCESS => return Ok(()),
                JXL_ENC_NEED_MORE_OUTPUT => continue,
                status => return self.raw.check(status),
            }
        }
    }

    /// 最後に投入されたフレームを書き出し、`writer` を返す
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。このとき、投入されたフレームがあれば
    /// 書き出してストリームを閉じてから返す。符号化に失敗したとき
    /// [`Error::Encode`]。書き出しに失敗したとき [`Error::Io`]。
    pub fn finish(mut self) -> Result<W, Error> {
        let mismatch =
            (self.frames_accepted != self.num_frames).then_some(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        if let Some(pending) = self.delta.take(&self.layout) {
            self.write(pending, true)?;
        }
        if let Some(mismatch) = mismatch {
            return Err(mismatch);
        }

        let Encoder { mut writer, .. } = self;
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
            quality: 90.0,
            effort: 3,
            num_plays: 3,
            tps_numerator: 30000,
            tps_denominator: 1001,
            max_threads: 2,
        }
    }

    fn encoder(num_frames: u32, config: Config) -> Result<Encoder<Cursor<Vec<u8>>>, Error> {
        Encoder::new(Cursor::new(Vec::new()), 16, 16, num_frames, config)
    }

    /// `index` 番目の画素だけが白いRGB8のフレーム
    ///
    /// 隣り合う `index` どうしの差分の外接矩形はキャンバスの一部に収まる。
    fn dot(index: usize) -> Vec<u8> {
        let mut frame = vec![0u8; 16 * 16 * 3];
        let at = (index % (16 * 16)) * 3;
        frame[at..at + 3].fill(0xFF);
        frame
    }

    /// 渡された長さと `flush` の回数を控える writer
    #[derive(Default)]
    struct Recorder {
        writes: Vec<usize>,
        flushes: u32,
    }

    impl Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.push(buf.len());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    /// 位置から決まる雑音。圧縮が効かないので出力が受け皿を超える
    fn noise_rgba(width: u32, height: u32) -> Vec<u8> {
        let mut state = 0x1234_5678u32;
        (0..(width as usize * height as usize * 4))
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect()
    }

    /// 受け皿に収まらない出力は、埋まるたびに渡して何周も排水する
    #[test]
    fn a_large_frame_drains_in_several_rounds() {
        let side = 256;
        let config = Config {
            color_type: ColorType::Rgba8,
            quality: *QUALITY_RANGE.end(),
            ..config()
        };
        let mut encoder = Encoder::new(Recorder::default(), side, side, 1, config).unwrap();
        encoder.add_frame(&noise_rgba(side, side), 1).unwrap();
        let recorder = encoder.finish().unwrap();

        let filled = recorder
            .writes
            .iter()
            .filter(|len| **len == OUTPUT_CHUNK)
            .count();
        assert!(
            filled >= 2,
            "受け皿が埋まったのは {filled} 回だけ: {:?}",
            recorder.writes
        );
    }

    #[test]
    fn finishing_flushes_the_writer() {
        let mut encoder = Encoder::new(Recorder::default(), 16, 16, 1, config()).unwrap();
        encoder.add_frame(&[0; 16 * 16 * 3], 1).unwrap();
        assert_eq!(encoder.writer.flushes, 0, "投入の途中で flush している");
        assert_eq!(encoder.finish().unwrap().flushes, 1);
    }

    /// 符号化を作業スレッドへ渡せる
    #[test]
    fn the_encoder_moves_across_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<Encoder<BufWriter<File>>>();
    }

    /// 共有はできない。渡せるのは所有権だけで、同時アクセスは型が拒む
    ///
    /// 固有の実装は `Sync` な型にだけ当たり、外れるとトレイトの既定へ落ちる。
    /// `u32` の判定がこの振り分け自体を見張る。
    #[test]
    fn the_encoder_is_not_shared_between_threads() {
        trait NotSync {
            const IS_SYNC: bool = false;
        }
        impl<T: ?Sized> NotSync for T {}
        struct Probe<T: ?Sized>(std::marker::PhantomData<T>);
        impl<T: ?Sized + Sync> Probe<T> {
            const IS_SYNC: bool = true;
        }
        const { assert!(!<Probe<Encoder<BufWriter<File>>>>::IS_SYNC) };
        const { assert!(<Probe<u32>>::IS_SYNC, "Sync な型を Sync と見抜けていない") };
    }

    /// 可逆になるのは品質の上限だけ
    #[test]
    fn only_the_top_of_the_quality_range_is_lossless() {
        assert!(
            Config {
                quality: *QUALITY_RANGE.end(),
                ..config()
            }
            .is_lossless()
        );
        assert!(
            !Config {
                quality: 99.9,
                ..config()
            }
            .is_lossless()
        );
    }

    #[test]
    fn a_zero_frame_count_is_rejected() {
        assert!(matches!(
            encoder(0, config()),
            Err(Error::InvalidFrameCount)
        ));
    }

    #[test]
    fn a_zero_tps_is_rejected() {
        for (numerator, denominator) in [(0, 1001), (30000, 0), (0, 0)] {
            assert!(
                matches!(
                    encoder(
                        2,
                        Config {
                            tps_numerator: numerator,
                            tps_denominator: denominator,
                            ..config()
                        }
                    ),
                    Err(Error::InvalidTps { .. })
                ),
                "{numerator}/{denominator}"
            );
        }
    }

    /// 分母の値域には、NTSCの分母1001が収まる
    #[test]
    fn the_writable_tps_denominators_are_accepted() {
        for denominator in [1, 2, 1000, 1001, 1002, 1024] {
            assert!(Tps::new(1, denominator).is_ok(), "1/{denominator}");
        }
    }

    #[test]
    fn a_tps_denominator_outside_the_writable_values_is_rejected() {
        for denominator in [1025, 1026, 2002, u32::MAX] {
            assert!(
                matches!(Tps::new(1, denominator), Err(Error::InvalidTps { .. })),
                "1/{denominator}"
            );
        }
    }

    #[test]
    fn the_tps_numerator_stops_at_the_writable_bound() {
        assert!(Tps::new(1 << 30, 1).is_ok());
        assert!(matches!(
            Tps::new((1 << 30) + 1, 1),
            Err(Error::InvalidTps { .. })
        ));
    }

    /// 値域を見るのは約した後の比
    #[test]
    fn the_writable_values_are_checked_after_the_reduction() {
        assert_eq!(
            Tps::new(60000, 2002).unwrap(),
            Tps {
                numerator: 30000,
                denominator: 1001
            }
        );
        assert_eq!(
            Tps::new(2048, 2048).unwrap(),
            Tps {
                numerator: 1,
                denominator: 1
            }
        );
        // 約しても 1/1025 にしかならず、値域の外に残る
        assert!(matches!(Tps::new(2, 2050), Err(Error::InvalidTps { .. })));
    }

    /// 約した比が渡るので、そのままなら書けない比でも符号化まで通る
    #[test]
    fn a_reducible_tps_reaches_the_encoder() {
        let mut encoder = encoder(
            2,
            Config {
                tps_numerator: 60000,
                tps_denominator: 2002,
                ..config()
            },
        )
        .unwrap();
        encoder.add_frame(&dot(0), 1).unwrap();
        encoder.add_frame(&dot(1), 1).unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        assert!(matches!(
            Encoder::new(Cursor::new(Vec::new()), 0, 16, 1, config()),
            Err(Error::InvalidDimensions { .. })
        ));
    }

    /// 値域の外はNaNも含めて弾く
    #[test]
    fn a_quality_outside_the_range_is_rejected() {
        for quality in [-0.1, 100.1, f32::NAN] {
            assert!(
                matches!(
                    encoder(
                        1,
                        Config {
                            quality,
                            ..config()
                        }
                    ),
                    Err(Error::InvalidQuality { .. })
                ),
                "{quality}"
            );
        }
        for quality in [0.0, 50.0, 100.0] {
            assert!(
                encoder(
                    1,
                    Config {
                        quality,
                        ..config()
                    }
                )
                .is_ok(),
                "{quality}"
            );
        }
    }

    #[test]
    fn an_effort_outside_the_range_is_rejected() {
        for effort in [0, 11] {
            assert!(
                matches!(
                    encoder(1, Config { effort, ..config() }),
                    Err(Error::InvalidEffort { .. })
                ),
                "{effort}"
            );
        }
        for effort in 1..=10 {
            assert!(
                encoder(1, Config { effort, ..config() }).is_ok(),
                "{effort}"
            );
        }
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

    /// フレームは1枚遅れて writer へ流れ、ファイル全体が溜まらない
    #[test]
    fn each_frame_reaches_the_writer_one_frame_behind() {
        let mut encoder = encoder(3, config()).unwrap();
        let mut written = Vec::new();
        for index in 0..3 {
            encoder.add_frame(&dot(index), 1).unwrap();
            written.push(encoder.writer.position());
        }
        assert_eq!(written[0], 0, "1枚目が投入と同時に書き出されている");
        assert!(written[1] > 0, "1枚目が2枚目の投入で流れていない");
        assert!(written[2] > written[1], "2枚目が溜め込まれている");
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
