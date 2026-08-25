//! キャンバスの大きさと、入力フレームのバイト並び

use crate::error::Error;
use anim_core::Rect;

/// 論理画面と画像記述子が持てる寸法の上限
const MAX_DIMENSION: u32 = u16::MAX as u32;

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

/// キャンバスの大きさと、入力フレームのバイト並び
#[derive(Debug, Clone, Copy)]
pub(crate) struct Layout {
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// 入力の色種別
    pub(crate) color_type: ColorType,
    /// 入力の1画素あたりのバイト数
    pub(crate) bytes_per_pixel: usize,
    /// 入力の1行のバイト数
    pub(crate) stride: usize,
    /// 入力の1フレームのバイト数
    pub(crate) frame_len: usize,
}

impl Layout {
    /// `width` x `height` の `color_type` を並べる配置を作る
    ///
    /// # Errors
    /// 寸法が0か65535を超えるとき [`Error::InvalidDimensions`]。1フレームの
    /// バイト数が `usize` で表現できないとき [`Error::ImageTooLarge`]。
    pub(crate) fn new(width: u32, height: u32, color_type: ColorType) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(Error::InvalidDimensions { width, height });
        }

        let bytes_per_pixel = color_type.bytes_per_pixel();
        let stride = (width as usize)
            .checked_mul(bytes_per_pixel)
            .ok_or(Error::ImageTooLarge { width, height })?;
        let frame_len = stride
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge { width, height })?;

        Ok(Layout {
            width: width as u16,
            height: height as u16,
            color_type,
            bytes_per_pixel,
            stride,
            frame_len,
        })
    }

    /// 論理画面全体を覆う矩形
    pub(crate) fn whole(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: u32::from(self.width),
            height: u32::from(self.height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_oversized_dimensions_are_rejected() {
        for (width, height) in [(0, 1), (1, 0), (65536, 1), (1, 65536), (70000, 70000)] {
            assert!(
                matches!(
                    Layout::new(width, height, ColorType::Rgb8),
                    Err(Error::InvalidDimensions { .. })
                ),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn the_largest_screen_is_accepted() {
        let layout = Layout::new(65535, 65535, ColorType::Rgba8).unwrap();
        assert_eq!((layout.width, layout.height), (65535, 65535));
        assert_eq!(layout.stride, 65535 * 4);
        assert_eq!(layout.frame_len, 65535 * 65535 * 4);
    }

    #[test]
    fn the_whole_rect_covers_the_logical_screen() {
        let layout = Layout::new(7, 5, ColorType::Rgb8).unwrap();
        assert_eq!(
            layout.whole(),
            Rect {
                x: 0,
                y: 0,
                width: 7,
                height: 5
            }
        );
    }
}
