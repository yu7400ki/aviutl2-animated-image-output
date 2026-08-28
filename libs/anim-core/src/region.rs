//! 矩形領域の切り出しと、1画素あたりのバイト数の詰め替え

use crate::diff::Rect;

/// 画素列を1画素 `out_bpp` バイトへ直しながら `out` へ追記する
///
/// `in_bpp` と `out_bpp` が等しければそのまま複製し、`out_bpp` が小さければ
/// 各画素の先頭 `out_bpp` バイトだけを残す。
pub fn append_pixels(pixels: &[u8], in_bpp: usize, out_bpp: usize, out: &mut Vec<u8>) {
    if in_bpp == out_bpp {
        out.extend_from_slice(pixels);
        return;
    }

    out.reserve(pixels.len() / in_bpp * out_bpp);
    for pixel in pixels.chunks_exact(in_bpp) {
        out.extend_from_slice(&pixel[..out_bpp]);
    }
}

/// `rect` の領域を連続バッファとして `out` へ追記する
///
/// `data` は `stride` バイトの行が隙間なく並んでいること。
pub fn crop(
    data: &[u8],
    rect: Rect,
    stride: usize,
    in_bpp: usize,
    out_bpp: usize,
    out: &mut Vec<u8>,
) {
    let row_len = rect.width as usize * in_bpp;
    let head = rect.y as usize * stride + rect.x as usize * in_bpp;
    out.reserve(rect.width as usize * rect.height as usize * out_bpp);
    for y in 0..rect.height as usize {
        let start = head + y * stride;
        append_pixels(&data[start..start + row_len], in_bpp, out_bpp, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x3のRGBA8キャンバス (画素の値は x, y, 0x10, アルファ)
    fn canvas() -> Vec<u8> {
        (0..3u8)
            .flat_map(|y| (0..4u8).flat_map(move |x| [x, y, 0x10, 0x80 + x]))
            .collect()
    }

    const STRIDE: usize = 4 * 4;

    #[test]
    fn a_sub_rect_is_packed_without_the_surrounding_pixels() {
        let mut out = Vec::new();
        let rect = Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        crop(&canvas(), rect, STRIDE, 4, 4, &mut out);

        assert_eq!(
            out,
            [
                1, 1, 0x10, 0x81, 2, 1, 0x10, 0x82, 1, 2, 0x10, 0x81, 2, 2, 0x10, 0x82
            ]
        );
    }

    #[test]
    fn dropping_alpha_keeps_the_color_channels_in_order() {
        let mut out = Vec::new();
        let rect = Rect {
            x: 2,
            y: 0,
            width: 2,
            height: 1,
        };
        crop(&canvas(), rect, STRIDE, 4, 3, &mut out);

        assert_eq!(out, [2, 0, 0x10, 3, 0, 0x10]);
    }

    #[test]
    fn appending_is_the_same_as_dropping_the_fourth_byte_of_each_pixel() {
        let pixels = canvas();
        let mut out = Vec::new();
        append_pixels(&pixels, 4, 3, &mut out);

        let expected: Vec<u8> = pixels
            .chunks_exact(4)
            .flat_map(|p| p[..3].to_vec())
            .collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn output_is_appended_after_the_existing_content() {
        let mut out = vec![0xAA];
        append_pixels(&[1, 2, 3, 4], 4, 4, &mut out);
        assert_eq!(out, [0xAA, 1, 2, 3, 4]);
    }
}
