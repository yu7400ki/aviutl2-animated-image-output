//! キャンバスの大きさと、入力フレームのバイト並び

use crate::color::ColorType;
use crate::diff::Rect;
use crate::error::InputError;

/// キャンバスの大きさと、入力フレームのバイト並び
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// 入力の色種別
    pub color_type: ColorType,
    /// 入力の1画素あたりのバイト数
    pub bytes_per_pixel: usize,
    /// 入力の1行のバイト数
    pub stride: usize,
    /// 入力の1フレームのバイト数
    pub frame_len: usize,
}

impl Layout {
    /// `width` x `height` の `color_type` を隙間なく並べる配置を作る
    ///
    /// 寸法の上限は画像フォーマットごとに違うため、呼び出し側が締める。
    ///
    /// # Errors
    /// 寸法が0のとき、1フレームのバイト数が `usize` に収まらないとき
    /// [`InputError::InvalidDimensions`]。
    pub fn new(width: u32, height: u32, color_type: ColorType) -> Result<Self, InputError> {
        if width == 0 || height == 0 {
            return Err(InputError::InvalidDimensions { width, height });
        }

        let bytes_per_pixel = color_type.bytes_per_pixel();
        let stride = width as usize * bytes_per_pixel;
        let Some(frame_len) = stride.checked_mul(height as usize) else {
            return Err(InputError::InvalidDimensions { width, height });
        };
        Ok(Layout {
            width,
            height,
            color_type,
            bytes_per_pixel,
            stride,
            frame_len,
        })
    }

    /// キャンバス全体を覆う矩形
    pub fn whole(&self) -> Rect {
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
    /// 長さが違うとき [`InputError::FrameSizeMismatch`]。
    pub fn check_frame(&self, data: &[u8]) -> Result<(), InputError> {
        if data.len() == self.frame_len {
            Ok(())
        } else {
            Err(InputError::FrameSizeMismatch {
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
                    Err(InputError::InvalidDimensions { .. })
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

    /// 1フレームのバイト数がusizeに収まらない寸法は弾く
    ///
    /// 溢れた積は0や小さな値へ化けるため、長さの検査がそれを期待値として通す。
    #[test]
    fn a_canvas_whose_length_overflows_is_rejected() {
        assert!(matches!(
            Layout::new(1 << 31, 1 << 31, ColorType::Rgba8),
            Err(InputError::InvalidDimensions {
                width: 2147483648,
                height: 2147483648
            })
        ));
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
            Err(InputError::FrameSizeMismatch {
                expected: 48,
                actual: 36
            })
        ));
        assert!(matches!(
            layout.check_frame(&[0; 4 * 3 * 4 + 1]),
            Err(InputError::FrameSizeMismatch {
                expected: 48,
                actual: 49
            })
        ));
    }
}
