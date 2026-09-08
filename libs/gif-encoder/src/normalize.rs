//! 画素の正規化

/// 完全透過へ潰すアルファの上限
///
/// GIFの透過は2値なので、これ未満のアルファを持つ画素を [`TRANSPARENT`] へ潰し、
/// 残りを不透明へ上げる。
const ALPHA_THRESHOLD: u8 = 128;

/// 完全透過の標識 (`R | G<<8 | B<<16 | A<<24` で詰めた値)
pub(crate) const TRANSPARENT: u32 = 0;

/// 2値化で見た目が変わった画素数
///
/// 元から完全透過 (α = 0) と完全不透明 (α = 255) の画素はどちらにも入らない。
/// 前者は潰しても画面に出るものが無く、後者は素通りするため、数えても
/// 失われたものを表さない。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Binarized {
    /// 見えていた画素を完全な透過へ潰した数 (0 < α < [`ALPHA_THRESHOLD`])
    pub(crate) to_transparent: u64,
    /// 透けていた画素を不透明へ上げた数 ([`ALPHA_THRESHOLD`] ≤ α < 255)
    pub(crate) to_opaque: u64,
}

impl std::ops::AddAssign for Binarized {
    fn add_assign(&mut self, other: Self) {
        self.to_transparent += other.to_transparent;
        self.to_opaque += other.to_opaque;
    }
}

/// 画素を `R | G<<8 | B<<16 | A<<24` へ詰める
///
/// `pixel` は1画素 `bpp` バイトが並んでいること。透過を持てない3バイトの画素は
/// アルファを255とみなす。
pub(crate) fn pack(pixel: &[u8], bpp: usize) -> u32 {
    let alpha = if bpp == 4 { pixel[3] } else { u8::MAX };
    u32::from_le_bytes([pixel[0], pixel[1], pixel[2], alpha])
}

/// RGBA8の画素列の透過を2値へ正規化し、見た目が変わった画素数を返す
///
/// `pixels` は1画素4バイトが隙間なく並んでいること。
pub(crate) fn binarize(pixels: &mut [u8]) -> Binarized {
    let mut changed = Binarized::default();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = pixel[3];
        if alpha < ALPHA_THRESHOLD {
            changed.to_transparent += u64::from(alpha != 0);
            pixel.fill(0);
        } else {
            changed.to_opaque += u64::from(alpha != u8::MAX);
            pixel[3] = u8::MAX;
        }
    }
    changed
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
        assert_eq!(
            binarize(&mut pixels),
            Binarized {
                to_transparent: 1,
                to_opaque: 1,
            }
        );
        assert_eq!(
            pixels,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 70, 80, 90, 255, 1, 2, 3, 255]
        );
    }

    /// 元から2値のアルファは、潰しても上げても見た目が変わらないので数えない
    #[test]
    fn already_binary_alpha_counts_as_no_change() {
        let mut pixels = vec![
            0, 0, 0, 0, // 完全透過
            1, 2, 3, 255, // 不透明
        ];
        assert_eq!(binarize(&mut pixels), Binarized::default());
        assert_eq!(pixels, vec![0, 0, 0, 0, 1, 2, 3, 255]);
    }

    /// 潰した画素と不透明にした画素は別々に数える
    #[test]
    fn the_two_directions_are_counted_apart() {
        let mut pixels = vec![
            10, 20, 30, 1, // 見えていたものが消える
            40, 50, 60, 254, // 透けていたものが不透明になる
            70, 80, 90, 200, // 同上
        ];
        assert_eq!(
            binarize(&mut pixels),
            Binarized {
                to_transparent: 1,
                to_opaque: 2,
            }
        );
    }

    #[test]
    fn the_marker_is_the_packed_form_of_the_collapsed_pixel() {
        let mut pixels = vec![10, 20, 30, 0];
        assert_eq!(binarize(&mut pixels), Binarized::default());
        assert_eq!(u32::from_le_bytes([0, 0, 0, 0]), TRANSPARENT);
        assert_eq!(
            u32::from_le_bytes([pixels[0], pixels[1], pixels[2], pixels[3]]),
            TRANSPARENT
        );
    }
}
