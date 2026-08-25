//! 画素の正規化

/// 完全透過へ潰すアルファの上限
///
/// GIFの透過は2値なので、これ未満のアルファを持つ画素を [`TRANSPARENT`] へ潰し、
/// 残りを不透明へ上げる。
const ALPHA_THRESHOLD: u8 = 128;

/// 完全透過の標識 (`R | G<<8 | B<<16 | A<<24` で詰めた値)
pub(crate) const TRANSPARENT: u32 = 0;

/// 画素を `R | G<<8 | B<<16 | A<<24` へ詰める
///
/// `pixel` は1画素 `bpp` バイトが並んでいること。透過を持てない3バイトの画素は
/// アルファを255とみなす。
pub(crate) fn pack(pixel: &[u8], bpp: usize) -> u32 {
    let alpha = if bpp == 4 { pixel[3] } else { u8::MAX };
    u32::from_le_bytes([pixel[0], pixel[1], pixel[2], alpha])
}

/// RGBA8の画素列の透過を2値へ正規化し、完全透過へ潰した画素数を返す
///
/// `pixels` は1画素4バイトが隙間なく並んでいること。
pub(crate) fn binarize(pixels: &mut [u8]) -> u64 {
    let mut squashed = 0;
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] < ALPHA_THRESHOLD {
            pixel.fill(0);
            squashed += 1;
        } else {
            pixel[3] = u8::MAX;
        }
    }
    squashed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_below_the_threshold_collapse_onto_a_single_value() {
        let mut pixels = vec![
            10, 20, 30, 0, // 完全透過
            40, 50, 60, 127, // 閾値の直下
            70, 80, 90, 128, // 閾値
            1, 2, 3, 255, // 不透明
        ];
        assert_eq!(binarize(&mut pixels), 2);
        assert_eq!(
            pixels,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 70, 80, 90, 255, 1, 2, 3, 255]
        );
    }

    #[test]
    fn the_marker_is_the_packed_form_of_the_collapsed_pixel() {
        let mut pixels = vec![10, 20, 30, 0];
        assert_eq!(binarize(&mut pixels), 1);
        assert_eq!(u32::from_le_bytes([0, 0, 0, 0]), TRANSPARENT);
        assert_eq!(
            u32::from_le_bytes([pixels[0], pixels[1], pixels[2], pixels[3]]),
            TRANSPARENT
        );
    }
}
