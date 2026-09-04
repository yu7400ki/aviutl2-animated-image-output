//! 符号化した矩形の復号と、キャンバスへの合成

use crate::error::{DecodingError, Error};
use anim_core::Rect;
use std::ffi::{c_int, c_void};
use std::ops::Deref;
use std::ptr::NonNull;
use std::slice;
use webp_sys::{WebPDecodeRGBA, WebPFree};

/// キャンバスの1画素のバイト数
const PIXEL: usize = 4;

/// 復号した矩形の画素
///
/// 走査順のRGBAが矩形の面積のぶん並ぶ。画素が生きているのは、この値を持って
/// いる間だけ。
pub(crate) struct Decoded {
    pixels: NonNull<u8>,
    len: usize,
}

impl Deref for Decoded {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.pixels.as_ptr(), self.len) }
    }
}

impl Drop for Decoded {
    fn drop(&mut self) {
        unsafe { WebPFree(self.pixels.as_ptr().cast::<c_void>()) };
    }
}

/// 単葉の .webp を `rect` の矩形として復号する
///
/// # Errors
/// 画素を取り出せないとき、または復号した寸法が `rect` と違うとき
/// [`Error::Decode`]。
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) fn decode(still: &[u8], rect: Rect) -> Result<Decoded, Error> {
    let (mut width, mut height): (c_int, c_int) = (0, 0);
    let pixels = unsafe { WebPDecodeRGBA(still.as_ptr(), still.len(), &mut width, &mut height) };
    let Some(pixels) = NonNull::new(pixels) else {
        return Err(Error::Decode(DecodingError::Refused));
    };

    let actual = (
        u32::try_from(width).unwrap_or(0),
        u32::try_from(height).unwrap_or(0),
    );
    let decoded = Decoded {
        pixels,
        len: actual.0 as usize * actual.1 as usize * PIXEL,
    };
    if actual != (rect.width, rect.height) {
        return Err(Error::Decode(DecodingError::SizeMismatch {
            expected: (rect.width, rect.height),
            actual,
        }));
    }

    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use crate::codec::{Codec, Job};
    use crate::frame::Canvas;
    use crate::layout::Layout;
    use anim_core::{ColorType, crop};

    /// 完全不透明を表すα
    const OPAQUE: u8 = u8::MAX;

    fn codec(lossless: bool, quality: f32) -> Codec {
        Codec::new(&Config {
            color_type: ColorType::Rgba8,
            lossless,
            quality,
            method: 4,
            num_plays: 0,
        })
        .unwrap()
    }

    /// 画素の値が縦横で別々に決まる不透明なRGBA
    ///
    /// 行と列を取り違えると値が食い違う。
    fn ramp(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    [
                        (x * 7 + y * 13) as u8,
                        (y * 11 + x * 3) as u8,
                        (x ^ y) as u8,
                        OPAQUE,
                    ]
                })
            })
            .collect()
    }

    /// `data` から `rect` を切り出す
    fn cropped(data: &[u8], layout: &Layout, rect: Rect) -> Vec<u8> {
        let mut out = Vec::new();
        crop(data, rect, layout.stride, PIXEL, PIXEL, &mut out);
        out
    }

    /// 縦横の違う矩形
    const RECT: Rect = Rect {
        x: 3,
        y: 5,
        width: 11,
        height: 7,
    };

    /// 可逆で符号化した矩形は、切り出した画素へバイト一致で戻る
    #[test]
    fn a_lossless_rect_decodes_back_to_the_pixels_it_carried() {
        let layout = Layout::new(23, 17, ColorType::Rgba8).unwrap();
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&ramp(layout.width, layout.height), ColorType::Rgba8);

        let job = Job::crop(canvas.staged(), &layout, RECT, None, Vec::new());
        let encoded = codec(true, 100.0).encode(&job).unwrap();

        assert_eq!(
            *decode(encoded.still(), RECT).unwrap(),
            *cropped(canvas.staged(), &layout, RECT)
        );
    }

    /// 求めた矩形と違う寸法で返ったら、画素を渡さない
    #[test]
    fn a_rect_of_another_shape_is_refused() {
        let layout = Layout::new(23, 17, ColorType::Rgba8).unwrap();
        let data = ramp(layout.width, layout.height);
        let job = Job::crop(&data, &layout, RECT, None, Vec::new());
        let encoded = codec(true, 100.0).encode(&job).unwrap();

        let transposed = Rect {
            width: RECT.height,
            height: RECT.width,
            ..RECT
        };
        assert!(matches!(
            decode(encoded.still(), transposed),
            Err(Error::Decode(DecodingError::SizeMismatch {
                expected: (7, 11),
                actual: (11, 7),
            }))
        ));
    }

    /// 単葉として読めないバイト列は復号の失敗になる
    #[test]
    fn bytes_that_are_not_a_still_are_refused() {
        for bytes in [b"".as_slice(), b"RIFF".as_slice(), &[0xFF; 64]] {
            assert!(matches!(
                decode(bytes, RECT),
                Err(Error::Decode(DecodingError::Refused))
            ));
        }
    }
}
