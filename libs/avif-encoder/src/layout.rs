//! キャンバスの大きさと、入力フレームのバイト並び

use crate::error::Error;
use anim_core::ColorType;

/// キャンバスの大きさと、入力フレームのバイト並び
#[derive(Debug, Clone, Copy)]
pub(crate) struct Layout {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// 入力の色種別
    pub(crate) color_type: ColorType,
    /// 入力の1行のバイト数
    pub(crate) stride: u32,
    /// 入力の1フレームのバイト数
    pub(crate) frame_len: usize,
}

impl Layout {
    /// `width` x `height` の `color_type` を並べる配置を作る
    ///
    /// 寸法の上限はlibavifの検査に委ね、ここでは行間が `avifRGBImage::rowBytes`
    /// に収まることまでを見る。
    ///
    /// # Errors
    /// 寸法が0か、1行のバイト数がu32に収まらないとき [`Error::InvalidDimensions`]。
    pub(crate) fn new(width: u32, height: u32, color_type: ColorType) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidDimensions { width, height });
        }

        let stride = width as usize * color_type.bytes_per_pixel();
        let Ok(stride) = u32::try_from(stride) else {
            return Err(Error::InvalidDimensions { width, height });
        };

        Ok(Layout {
            width,
            height,
            color_type,
            stride,
            frame_len: stride as usize * height as usize,
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

    /// 長さの計算はusizeで行う。u32演算なら折り返して小さな期待値を通してしまう
    #[test]
    fn a_large_canvas_keeps_its_length_in_usize() {
        let layout = Layout::new(100_000, 100_000, ColorType::Rgba8).unwrap();
        assert_eq!(layout.stride, 400_000);
        assert_eq!(layout.frame_len, 40_000_000_000);
    }

    #[test]
    fn a_row_beyond_the_row_bytes_field_is_rejected() {
        assert!(matches!(
            Layout::new(u32::MAX, 1, ColorType::Rgb8),
            Err(Error::InvalidDimensions { .. })
        ));
        let layout = Layout::new(u32::MAX / 4, 1, ColorType::Rgba8).unwrap();
        assert_eq!(layout.stride, (u32::MAX / 4) * 4);
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
