//! RGBA8の画素列に含まれるアルファの判定

/// 2画素ぶんのアルファだけを取り出すマスク
const ALPHA_MASK: u64 = 0xFF00_0000_FF00_0000;

/// RGBA8の画素列に不透明でない画素が1つでもあるか調べる
///
/// `pixels` は1画素4バイトのR,G,B,Aが隙間なく並んでいること。
pub fn has_transparency(pixels: &[u8]) -> bool {
    // 論理積は255を保つので、全画素が不透明なときに限り累積値のアルファが255で残る
    let mut acc = u64::MAX;
    let mut pairs = pixels.chunks_exact(8);
    for pair in &mut pairs {
        let bytes: [u8; 8] = pair.try_into().expect("chunks_exactが返すのは8バイト");
        acc &= u64::from_le_bytes(bytes);
    }

    let mut opaque = (acc & ALPHA_MASK) == ALPHA_MASK;
    for pixel in pairs.remainder().chunks_exact(4) {
        opaque &= pixel[3] == u8::MAX;
    }
    !opaque
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全画素不透明のRGBA8列を作る
    fn opaque(pixels: usize) -> Vec<u8> {
        (0..pixels)
            .flat_map(|i| [i as u8, (i >> 8) as u8, (i >> 16) as u8, 0xFF])
            .collect()
    }

    #[test]
    fn an_empty_slice_has_no_transparency() {
        assert!(!has_transparency(&[]));
    }

    #[test]
    fn fully_opaque_pixels_are_detected_at_every_length() {
        for pixels in 0..40 {
            assert!(!has_transparency(&opaque(pixels)), "{pixels} 画素");
        }
    }

    /// 端数の画素も含めて、どの位置のアルファでも見つかる
    #[test]
    fn a_single_transparent_pixel_is_found_at_any_position() {
        for pixels in 1..40 {
            for index in 0..pixels {
                let mut data = opaque(pixels);
                data[index * 4 + 3] = 0xFE;
                assert!(has_transparency(&data), "{pixels} 画素の {index} 番目");
            }
        }
    }

    /// アルファ以外のチャンネルが0でも不透明と判定する
    #[test]
    fn zero_color_channels_do_not_imply_transparency() {
        let data = vec![0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF];
        assert!(!has_transparency(&data));
    }

    #[test]
    fn a_fully_transparent_pixel_is_found() {
        let mut data = opaque(9);
        data[4 * 4 + 3] = 0;
        assert!(has_transparency(&data));
    }
}
