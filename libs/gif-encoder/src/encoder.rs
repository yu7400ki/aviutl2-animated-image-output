//! GIFのストリーミング書き出し

use crate::block::{self, DISPOSAL_DO_NOT_DISPOSE};
use crate::error::Error;
use crate::layout::{ColorType, Layout};
use crate::normalize::{self, TRANSPARENT};
use crate::table::ColorTable;
use anim_core::{Colors, FrameDelay, Indexed};
use std::io::Write;

/// 遅延時間の下限 (1/100秒)
///
/// 0と1は多くのデコーダが10へ引き上げるため、それを避ける下限を置く。
const MIN_DELAY: u64 = 2;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。
    pub color_type: ColorType,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
}

/// 既定は無限ループするRGB8
impl Default for Config {
    fn default() -> Self {
        Config {
            color_type: ColorType::Rgb8,
            num_plays: 0,
        }
    }
}

/// GIFのエンコーダ
///
/// [`Encoder::new`] で寸法とフレーム数を宣言し、[`Encoder::add_frame`] で
/// フレームを投入し、[`Encoder::finish`] で閉じる。
pub struct Encoder<W: Write> {
    writer: W,
    layout: Layout,
    num_frames: u32,
    num_plays: u32,
    frames_accepted: u32,
    poisoned: bool,
}

impl<W: Write> Encoder<W> {
    /// `width` x `height` の `num_frames` フレームを `writer` へ書き出す
    ///
    /// # Errors
    /// 寸法が0か65535を超えるとき [`Error::InvalidDimensions`]。フレーム数が0の
    /// とき [`Error::InvalidFrameCount`]、1でないとき
    /// [`Error::UnsupportedFrameCount`]。1フレームのバイト数が `usize` で
    /// 表現できないとき [`Error::ImageTooLarge`]。
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
        if num_frames != 1 {
            return Err(Error::UnsupportedFrameCount(num_frames));
        }

        Ok(Encoder {
            writer,
            layout: Layout::new(width, height, config.color_type)?,
            num_frames,
            num_plays: config.num_plays,
            frames_accepted: 0,
            poisoned: false,
        })
    }

    /// フレームを1つ投入する
    ///
    /// `data` は [`Config::color_type`] の画素が左上から右下へ隙間なく
    /// 並んでいること。
    ///
    /// # Errors
    /// バイト数が寸法と色種別から決まる長さと違うとき
    /// [`Error::FrameSizeMismatch`]。宣言したフレーム数を超えたとき
    /// [`Error::FrameCountMismatch`]。フレームの色がカラーテーブルに収まらない
    /// とき [`Error::TooManyColors`]。
    pub fn add_frame(&mut self, data: &[u8], delay: FrameDelay) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if data.len() != self.layout.frame_len {
            return Err(Error::FrameSizeMismatch {
                expected: self.layout.frame_len,
                actual: data.len(),
            });
        }
        if self.frames_accepted == self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted + 1,
            });
        }

        let mut pixels = data.to_vec();
        if self.layout.color_type == ColorType::Rgba8 {
            normalize::binarize(&mut pixels);
        }

        let mut colors = Colors::new();
        colors.observe(&pixels, self.layout.bytes_per_pixel);
        if colors.exceeded() {
            return Err(Error::TooManyColors);
        }
        // GIFは添字の局所性に無関心なので、見つけた順のまま添字を振る
        let indexed = colors.into_indexed(|_| ());

        self.write_frame(&pixels, &indexed, delay)
            .inspect_err(|_| self.poisoned = true)?;
        self.frames_accepted += 1;
        Ok(())
    }

    /// 書き出しを終え、終端を書いて `writer` を返す
    ///
    /// # Errors
    /// 投入されたフレーム数が宣言したフレーム数に満たないとき
    /// [`Error::FrameCountMismatch`]。
    pub fn finish(mut self) -> Result<W, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if self.frames_accepted != self.num_frames {
            return Err(Error::FrameCountMismatch {
                expected: self.num_frames,
                actual: self.frames_accepted,
            });
        }

        block::trailer(&mut self.writer)?;
        self.writer.flush()?;
        Ok(self.writer)
    }

    /// ヘッダから画像データまでを書く
    fn write_frame(
        &mut self,
        pixels: &[u8],
        indexed: &Indexed,
        delay: FrameDelay,
    ) -> Result<(), Error> {
        let table = ColorTable::new(indexed.colors());
        // 透過標識が和集合にあるなら、そのエントリがそのまま透過インデックスになる
        let transparent = indexed
            .colors()
            .iter()
            .position(|&color| color == TRANSPARENT)
            .map(|index| index as u8);

        let mut indices = Vec::new();
        indexed.append_indices(pixels, self.layout.bytes_per_pixel, &mut indices);

        block::header(&mut self.writer)?;
        block::logical_screen_descriptor(
            &mut self.writer,
            self.layout.width,
            self.layout.height,
            table.size_field(),
        )?;
        block::color_table(&mut self.writer, table.bytes())?;
        block::netscape(&mut self.writer, self.num_plays)?;
        block::graphic_control(
            &mut self.writer,
            DISPOSAL_DO_NOT_DISPOSE,
            hundredths(delay),
            transparent,
        )?;
        block::image_descriptor(
            &mut self.writer,
            0,
            0,
            self.layout.width,
            self.layout.height,
        )?;
        block::image_data(&mut self.writer, table.min_code_size(), &indices)?;
        Ok(())
    }
}

/// フレーム遅延を1/100秒へ丸める
fn hundredths(delay: FrameDelay) -> u16 {
    let numerator = u64::from(delay.numerator()) * 100;
    let denominator = u64::from(delay.denominator());
    let rounded = (numerator + denominator / 2) / denominator;
    rounded.clamp(MIN_DELAY, u64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_is_rounded_to_hundredths_of_a_second() {
        for (numerator, denominator, expected) in [
            (1, 30, 3),
            (2, 30, 7),
            (1, 1, 100),
            (1001, 30000, 3),
            (1, 10, 10),
            (3, 40, 8),
        ] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            assert_eq!(hundredths(delay), expected, "{numerator}/{denominator}");
        }
    }

    /// 0と1へ丸まる遅延は下限まで切り上げる
    #[test]
    fn the_delay_is_raised_to_the_lower_bound() {
        for (numerator, denominator) in [(0, 30), (1, 1000), (1, 100), (1, 60)] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            assert_eq!(
                u64::from(hundredths(delay)),
                MIN_DELAY,
                "{numerator}/{denominator}"
            );
        }
    }

    #[test]
    fn a_long_delay_saturates_at_the_field_width() {
        let delay = FrameDelay::new(1000, 1).unwrap();
        assert_eq!(hundredths(delay), u16::MAX);
    }
}
