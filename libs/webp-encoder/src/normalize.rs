//! 画素の正規化

/// RGBA8の画素列の完全透過を1つの値へ集約する
///
/// α = 0 の画素を `0x00000000` へ潰す。半透明 (0 < α < 255) は触らない。
/// `pixels` は1画素4バイトが隙間なく並んでいること。
pub(crate) fn normalize(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            pixel.fill(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fully_transparent_pixel_collapses_onto_a_single_value() {
        let mut pixels = vec![
            10, 20, 30, 0, // 完全透過
            40, 50, 60, 1, // 透過の直上
            70, 80, 90, 254, // 不透明の直下
            1, 2, 3, 255, // 不透明
        ];
        normalize(&mut pixels);

        assert_eq!(
            pixels,
            vec![0, 0, 0, 0, 40, 50, 60, 1, 70, 80, 90, 254, 1, 2, 3, 255]
        );
    }

    /// 透過下のRGBが違うだけの画素列は、正規化すると一致する
    #[test]
    fn transparent_pixels_of_different_colors_become_equal() {
        let mut left = vec![10, 20, 30, 0, 1, 2, 3, 255];
        let mut right = vec![90, 80, 70, 0, 1, 2, 3, 255];
        normalize(&mut left);
        normalize(&mut right);

        assert_eq!(left, right);
    }
}
