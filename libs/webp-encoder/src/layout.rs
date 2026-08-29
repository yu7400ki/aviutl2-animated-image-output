//! キャンバスの大きさと、入力フレームのバイト並び

use crate::error::Error;
use anim_core::Rect;

/// キャンバスが取りうる幅・高さの上限
const MAX_DIMENSION: u32 = webp_sys::WEBP_MAX_DIMENSION as u32;

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
    pub(crate) width: u32,
    pub(crate) height: u32,
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
    /// 寸法が0か16383を超えるとき [`Error::InvalidDimensions`]。
    pub(crate) fn new(width: u32, height: u32, color_type: ColorType) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(Error::InvalidDimensions { width, height });
        }

        let bytes_per_pixel = color_type.bytes_per_pixel();
        let stride = width as usize * bytes_per_pixel;

        Ok(Layout {
            width,
            height,
            color_type,
            bytes_per_pixel,
            stride,
            frame_len: stride * height as usize,
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

    /// `data` が1フレームぶんの長さか検める
    ///
    /// # Errors
    /// 長さが違うとき [`Error::FrameSizeMismatch`]。
    pub(crate) fn check_frame(&self, data: &[u8]) -> Result<(), Error> {
        if data.len() == self.frame_len {
            Ok(())
        } else {
            Err(Error::FrameSizeMismatch {
                expected: self.frame_len,
                actual: data.len(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_oversized_dimensions_are_rejected() {
        for (width, height) in [(0, 1), (1, 0), (16384, 1), (1, 16384), (70000, 70000)] {
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
    fn the_largest_canvas_is_accepted() {
        let layout = Layout::new(16383, 16383, ColorType::Rgba8).unwrap();
        assert_eq!((layout.width, layout.height), (16383, 16383));
        assert_eq!(layout.stride, 16383 * 4);
        assert_eq!(layout.frame_len, 16383 * 16383 * 4);
    }

    #[test]
    fn the_whole_rect_covers_the_canvas() {
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

    /// 長さは過不足のどちらでも弾く
    ///
    /// 長すぎる入力を通すと、はみ出したぶんが黙って捨てられる。
    #[test]
    fn a_frame_of_another_length_is_rejected() {
        let layout = Layout::new(4, 3, ColorType::Rgba8).unwrap();
        assert!(layout.check_frame(&[0; 4 * 3 * 4]).is_ok());
        assert!(matches!(
            layout.check_frame(&[0; 4 * 3 * 3]),
            Err(Error::FrameSizeMismatch {
                expected: 48,
                actual: 36
            })
        ));
        assert!(matches!(
            layout.check_frame(&[0; 4 * 3 * 4 + 1]),
            Err(Error::FrameSizeMismatch {
                expected: 48,
                actual: 49
            })
        ));
    }
}
