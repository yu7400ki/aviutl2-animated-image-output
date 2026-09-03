//! キャンバスの大きさと、入力フレームのバイト並び

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

    /// 1画素あたりのチャネル数
    pub(crate) fn num_channels(self) -> u32 {
        match self {
            ColorType::Rgb8 => 3,
            ColorType::Rgba8 => 4,
        }
    }

    /// αを持つか
    pub(crate) fn has_alpha(self) -> bool {
        match self {
            ColorType::Rgb8 => false,
            ColorType::Rgba8 => true,
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
    /// `width` x `height` の `color_type` を隙間なく並べる配置を作る
    ///
    /// 寸法の上限はlibjxlの検査に委ねる。
    ///
    /// # Errors
    /// 寸法が0のとき [`Error::InvalidDimensions`]。
    pub(crate) fn new(width: u32, height: u32, color_type: ColorType) -> Result<Self, Error> {
        if width == 0 || height == 0 {
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
    fn a_zero_dimension_is_rejected() {
        for (width, height) in [(0, 1), (1, 0), (0, 0)] {
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
    fn the_color_type_decides_the_channels_and_the_alpha() {
        assert_eq!(ColorType::Rgb8.bytes_per_pixel(), 3);
        assert_eq!(ColorType::Rgb8.num_channels(), 3);
        assert!(!ColorType::Rgb8.has_alpha());
        assert_eq!(ColorType::Rgba8.bytes_per_pixel(), 4);
        assert_eq!(ColorType::Rgba8.num_channels(), 4);
        assert!(ColorType::Rgba8.has_alpha());
    }

    /// 長さの計算はusizeで行う。u32演算なら折り返して小さな期待値を通してしまう
    #[test]
    fn a_large_canvas_keeps_its_length_in_usize() {
        let layout = Layout::new(100_000, 100_000, ColorType::Rgba8).unwrap();
        assert_eq!(layout.stride, 400_000);
        assert_eq!(layout.frame_len, 40_000_000_000);
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
