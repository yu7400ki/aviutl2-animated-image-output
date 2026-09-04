//! 符号化した矩形の復号と、キャンバスへの合成

use crate::error::{DecodingError, Error};
use crate::frame::Placement;
use anim_core::Rect;
use std::ffi::{c_int, c_void};
use std::ops::Deref;
use std::ptr::NonNull;
use std::slice;
use webp_sys::{WebPDecodeRGBA, WebPFree};

/// キャンバスの1画素のバイト数
const PIXEL: usize = 4;

/// 完全不透明を表すα
const OPAQUE: u8 = u8::MAX;

/// 復号した矩形の画素
///
/// 走査順のRGBAが矩形の面積のぶん並ぶ。画素が生きているのは、この値を持って
/// いる間だけ。
pub(crate) struct Decoded {
    pixels: NonNull<u8>,
    len: usize,
    /// 画素が埋める矩形
    rect: Rect,
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
        rect: Rect {
            width: actual.0,
            height: actual.1,
            ..rect
        },
    };
    if actual != (rect.width, rect.height) {
        return Err(Error::Decode(DecodingError::SizeMismatch {
            expected: (rect.width, rect.height),
            actual,
        }));
    }

    Ok(decoded)
}

/// 復号した矩形をキャンバスへ合成する
///
/// `shown` は前のフレームを表示した面、`disposed` はそこから前のフレームの矩形を
/// 抜いた面で、どちらも `stride` バイトの行が隙間なく並ぶRGBA。
///
/// 戻ったとき `shown` は `placement` のフレームを表示した面、`disposed` は
/// そこから `decoded` の矩形を抜いた面になる。
pub(crate) fn compose(
    shown: &mut [u8],
    disposed: &mut [u8],
    stride: usize,
    placement: Placement,
    decoded: &Decoded,
) {
    debug_assert_eq!(
        placement.rect, decoded.rect,
        "載せる矩形と復号した矩形は同じフレームのもの"
    );

    if placement.dispose {
        shown.copy_from_slice(disposed);
    }

    let rect = decoded.rect;
    let row_len = rect.width as usize * PIXEL;
    let head = rect.y as usize * stride + rect.x as usize * PIXEL;
    for (y, row) in decoded.chunks_exact(row_len).enumerate() {
        let at = head + y * stride;
        let target = &mut shown[at..at + row_len];
        if placement.blend {
            for (base, source) in target.chunks_exact_mut(PIXEL).zip(row.chunks_exact(PIXEL)) {
                base.copy_from_slice(&over(source, base));
            }
        } else {
            target.copy_from_slice(row);
        }
    }

    disposed.copy_from_slice(shown);
    for y in 0..rect.height as usize {
        let at = head + y * stride;
        disposed[at..at + row_len].fill(0);
    }
}

/// `source` を `base` の上へ重ねた画素
///
/// αを乗じていないRGBAどうしを、復号器が画面へ出すのと同じ整数演算で混ぜる。
fn over(source: &[u8], base: &[u8]) -> [u8; PIXEL] {
    match source[3] {
        OPAQUE => return [source[0], source[1], source[2], OPAQUE],
        0 => return [base[0], base[1], base[2], base[3]],
        _ => {}
    }

    let source_alpha = u32::from(source[3]);
    let base_alpha = (u32::from(base[3]) * (256 - source_alpha)) >> 8;
    let alpha = source_alpha + base_alpha;
    let scale = u64::from((1u32 << 24) / alpha);
    let channel = |index: usize| {
        let mixed = u64::from(
            u32::from(source[index]) * source_alpha + u32::from(base[index]) * base_alpha,
        );
        ((mixed * scale) >> 24) as u8
    };

    [channel(0), channel(1), channel(2), alpha as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use crate::codec::{Codec, Job};
    use crate::frame::Canvas;
    use crate::layout::Layout;
    use crate::normalize::normalize;
    use anim_core::{ColorType, crop};
    use std::collections::BTreeSet;

    /// 素材の四角の一辺の長さ
    const SQUARE: u32 = 6;

    fn settings(lossless: bool, quality: f32) -> Config {
        Config {
            color_type: ColorType::Rgba8,
            lossless,
            quality,
            method: 4,
            num_plays: 0,
        }
    }

    fn codec(lossless: bool, quality: f32) -> Codec {
        Codec::new(&settings(lossless, quality)).unwrap()
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

    /// 透過の面に四角を1つ置いたRGBA
    fn sprite(width: u32, height: u32, at: (u32, u32), alpha: u8) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    if x.wrapping_sub(at.0) < SQUARE && y.wrapping_sub(at.1) < SQUARE {
                        [0x20, 0x40, 0x60, alpha]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
            })
            .collect()
    }

    /// 不透明な面に四角を1つ置いたRGBA
    fn patched(width: u32, height: u32, at: (u32, u32)) -> Vec<u8> {
        let mut data = ramp(width, height);
        for y in at.1..at.1 + SQUARE {
            for x in at.0..at.0 + SQUARE {
                let index = (y * width + x) as usize * PIXEL;
                data[index..index + PIXEL].copy_from_slice(&[0x20, 0x40, 0x60, OPAQUE]);
            }
        }
        data
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
        let mut canvas = Canvas::new(&layout, &settings(true, 100.0));
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

    /// 完全透過と完全不透明の画素は、混ぜずにそのまま通る
    #[test]
    fn the_ends_of_the_alpha_range_pass_through_untouched() {
        let base = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(over(&[0x99, 0x88, 0x77, 0], &base), base);
        assert_eq!(
            over(&[0x99, 0x88, 0x77, OPAQUE], &base),
            [0x99, 0x88, 0x77, OPAQUE]
        );
    }

    /// 半透明どうしを重ねるとαが上がり、重ね続けると完全不透明で止まる
    #[test]
    fn blending_a_half_transparent_pixel_raises_the_alpha() {
        let source = [0x40, 0x40, 0x40, 0x80];
        let mut base = [0xC0, 0xC0, 0xC0, 0x80];

        base = over(&source, &base);
        assert_eq!(base, [0x6A, 0x6A, 0x6A, 0xC0]);

        let mut alphas = vec![base[3]];
        for _ in 0..7 {
            base = over(&source, &base);
            alphas.push(base[3]);
        }
        assert_eq!(alphas, [192, 224, 240, 248, 252, 254, 255, 255]);
    }

    /// 土台が完全透過なら、重ねてもαは動かない
    ///
    /// αが1でも同じ規則で通り、割る数は0にならない。
    #[test]
    fn blending_over_a_fully_transparent_base_keeps_the_alpha() {
        for alpha in 1..OPAQUE {
            assert_eq!(
                over(&[0x10, 0x20, 0x30, alpha], &[0, 0, 0, 0])[3],
                alpha,
                "α = {alpha}"
            );
        }
    }

    /// 素材を1枚ずつ符号化し、復号した矩形を合成しながら入力と突き合わせる
    ///
    /// 合成した面は、開ループのキャンバスが持つ2面と同じものになる。
    fn compose_frames(layout: &Layout, frames: &[Vec<u8>]) -> Vec<Placement> {
        let codec = codec(true, 100.0);
        let mut canvas = Canvas::new(layout, &settings(true, 100.0));
        let mut shown = vec![0u8; layout.frame_len];
        let mut disposed = vec![0u8; layout.frame_len];
        let mut placements = Vec::new();

        for (index, frame) in frames.iter().enumerate() {
            canvas.stage(frame, layout.color_type);
            let placement = canvas.place(index > 0).expect("フレームごとに画素が変わる");
            let base = (placement.blend && codec.substitutes_transparency())
                .then(|| canvas.base(placement.dispose));
            let job = Job::crop(canvas.staged(), layout, placement.rect, base, Vec::new());
            let encoded = codec.encode(&job).unwrap();
            canvas
                .commit(placement, || {
                    unreachable!("可逆が符号化した結果を求めている")
                })
                .unwrap();

            let decoded = decode(encoded.still(), placement.rect).unwrap();
            compose(
                &mut shown,
                &mut disposed,
                layout.stride,
                placement,
                &decoded,
            );

            let mut expected = frame.clone();
            normalize(&mut expected);
            assert_eq!(shown, expected, "フレーム{index}を表示した面 {placement:?}");
            assert_eq!(
                disposed,
                canvas.base(true),
                "フレーム{index}の矩形を抜いた面 {placement:?}"
            );
            placements.push(placement);
        }

        placements
    }

    /// 復号した矩形を合成すると、可逆では入力へバイト一致で戻る
    ///
    /// 重ねる形と上書きする形、抜いた面へ載せる形と抜かない面へ載せる形の
    /// 4通りをすべて踏む。
    #[test]
    fn composing_the_decoded_rects_rebuilds_the_input() {
        let layout = Layout::new(28, 22, ColorType::Rgba8).unwrap();
        let (width, height) = (layout.width, layout.height);

        let materials = [
            // 不透明な面を動く四角。重ねる形が採れ、抜くと矩形が広がる
            vec![
                patched(width, height, (2, 2)),
                patched(width, height, (12, 8)),
                patched(width, height, (20, 14)),
            ],
            // 透過の面を離れて動く半透明の四角。抜いた面が狭く、重ねる形は採れない
            vec![
                sprite(width, height, (2, 2), 0x80),
                sprite(width, height, (18, 14), 0x80),
            ],
            // 透過の面を離れて動く不透明な四角。抜いた面が狭く、重ねる形も採れる
            vec![
                sprite(width, height, (2, 2), OPAQUE),
                sprite(width, height, (18, 14), OPAQUE),
            ],
        ];

        let seen: BTreeSet<(bool, bool)> = materials
            .iter()
            .flat_map(|frames| compose_frames(&layout, frames))
            .map(|placement| (placement.blend, placement.dispose))
            .collect();

        assert_eq!(
            seen,
            BTreeSet::from([(false, false), (false, true), (true, false), (true, true)])
        );
    }

    /// αが左から右へ連続して変わり、RGBが位置で決まるRGBA
    ///
    /// 左の4分の1は完全透過、続く4分の1でαが上がり、右の半分は完全不透明になる。
    fn alpha_ramp(width: u32, height: u32) -> Vec<u8> {
        let fade = width / 4;
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    let alpha = (x.saturating_sub(fade) * 255 / fade).min(255) as u8;
                    if alpha == 0 {
                        [0, 0, 0, 0]
                    } else {
                        [(x * 3) as u8, (y * 5) as u8, 0x80, alpha]
                    }
                })
            })
            .collect()
    }

    /// αの連続する素材から切り出した、縦横の違う矩形
    const FADE_RECT: Rect = Rect {
        x: 6,
        y: 5,
        width: 37,
        height: 29,
    };

    /// αの連続する素材から矩形を符号化して復号し、切り出した画素と対で返す
    fn decode_fade(lossless: bool, quality: f32) -> (Vec<u8>, Vec<u8>) {
        let layout = Layout::new(64, 48, ColorType::Rgba8).unwrap();
        let mut canvas = Canvas::new(&layout, &settings(true, 100.0));
        canvas.stage(&alpha_ramp(layout.width, layout.height), ColorType::Rgba8);

        let job = Job::crop(canvas.staged(), &layout, FADE_RECT, None, Vec::new());
        let encoded = codec(lossless, quality).encode(&job).unwrap();
        let decoded = decode(encoded.still(), FADE_RECT).unwrap();

        (
            decoded.to_vec(),
            cropped(canvas.staged(), &layout, FADE_RECT),
        )
    }

    /// 選んだ画素のRGBの、入力からの平均の隔たり
    fn mean_rgb_gap(decoded: &[u8], expected: &[u8], alpha: u8) -> f64 {
        let mut total = 0u64;
        let mut count = 0u64;
        for (decoded, expected) in decoded
            .chunks_exact(PIXEL)
            .zip(expected.chunks_exact(PIXEL))
        {
            if expected[3] != alpha {
                continue;
            }
            total += decoded[..3]
                .iter()
                .zip(&expected[..3])
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum::<u64>();
            count += 3;
        }
        assert!(count > 0, "α = {alpha} の画素が無い");
        total as f64 / count as f64
    }

    /// 品質を振って測る動作点
    const QUALITIES: [f32; 3] = [50.0, 75.0, 90.0];

    /// 非可逆でも、矩形の中のαは入力とバイト一致する
    #[test]
    fn a_lossy_rect_carries_the_alpha_of_the_input() {
        for quality in QUALITIES {
            let (decoded, expected) = decode_fade(false, quality);
            let alpha: Vec<u8> = decoded.iter().skip(3).step_by(PIXEL).copied().collect();
            let want: Vec<u8> = expected.iter().skip(3).step_by(PIXEL).copied().collect();
            assert_eq!(alpha, want, "品質{quality}のα");
        }
    }

    /// 非可逆では、完全透過の下のRGBが動く
    ///
    /// 動く量は品質を上げても縮まず、同じ矩形の不透明な画素より何倍も大きい。
    /// 完全透過の画素をRGBで比べられないのはこのため。
    #[test]
    fn the_color_under_a_fully_transparent_pixel_moves_when_it_is_lossy() {
        for quality in QUALITIES {
            let (decoded, expected) = decode_fade(false, quality);
            let hidden = mean_rgb_gap(&decoded, &expected, 0);
            let visible = mean_rgb_gap(&decoded, &expected, OPAQUE);
            assert!(hidden > 16.0, "品質{quality}の完全透過の下 {hidden}");
            assert!(
                hidden > visible * 4.0,
                "品質{quality}: 完全透過の下 {hidden}、不透明 {visible}"
            );

            let (decoded, expected) = decode_fade(true, quality);
            assert_eq!(decoded, expected, "可逆の品質{quality}");
        }
    }
}
