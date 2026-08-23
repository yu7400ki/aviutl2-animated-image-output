//! 入力フレームの並びと、選べる画素表現

use crate::diff::Rect;
use crate::error::Error;

/// 画素の色種別 (ビット深度8固定)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    /// 8bit/chのRGB
    Rgb8,
    /// 8bit/chのRGBA
    Rgba8,
}

impl ColorType {
    /// 1画素あたりのバイト数
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            ColorType::Rgb8 => 3,
            ColorType::Rgba8 => 4,
        }
    }
}

/// 出力の画素表現 (ビット深度8固定)
///
/// 入力に取れる色種別のほか、パレットを引く添字を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    /// 8bit/chのRGB
    Rgb8,
    /// 8bit/chのRGBA
    Rgba8,
    /// PLTEを引く1バイトの添字
    Indexed8,
}

impl Output {
    /// 1画素あたりのバイト数
    pub(crate) fn bytes_per_pixel(self) -> usize {
        match self {
            Output::Rgb8 => 3,
            Output::Rgba8 => 4,
            Output::Indexed8 => 1,
        }
    }

    /// IHDRのcolour type
    pub(crate) fn code(self) -> u8 {
        match self {
            Output::Rgb8 => 2,
            Output::Rgba8 => 6,
            Output::Indexed8 => 3,
        }
    }
}

impl From<ColorType> for Output {
    fn from(color_type: ColorType) -> Self {
        match color_type {
            ColorType::Rgb8 => Output::Rgb8,
            ColorType::Rgba8 => Output::Rgba8,
        }
    }
}

/// キャンバスの大きさと、入力フレームのバイト並び
#[derive(Debug, Clone, Copy)]
pub(crate) struct Layout {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// 入力の色種別
    pub(crate) input: ColorType,
    /// 入力の1画素あたりのバイト数
    pub(crate) bytes_per_pixel: usize,
    /// 入力の1行のバイト数
    pub(crate) stride: usize,
    /// 入力の1フレームのバイト数
    pub(crate) frame_len: usize,
}

impl Layout {
    /// `width` x `height` の `input` を並べる配置を作る
    ///
    /// # Errors
    /// 1フレームのバイト数が `usize` で表現できないとき。
    pub(crate) fn new(width: u32, height: u32, input: ColorType) -> Result<Self, Error> {
        let bytes_per_pixel = input.bytes_per_pixel();
        let stride = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(Error::ImageTooLarge { width, height })?;
        let frame_len = stride
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge { width, height })?;

        Ok(Layout {
            width,
            height,
            input,
            bytes_per_pixel,
            stride,
            frame_len,
        })
    }

    /// キャンバス全体を覆う矩形
    pub(crate) fn whole(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }
}
