//! 入力フレームの並び

use crate::error::Error;
use anim_core::{ColorType, Rect};

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

    /// 入力の色種別が対応するIHDRのcolour type
    pub(crate) fn color_type_code(&self) -> u8 {
        match self.input {
            ColorType::Rgb8 => 2,
            ColorType::Rgba8 => 6,
        }
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
