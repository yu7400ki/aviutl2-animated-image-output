//! 矩形領域の切り出しと貼り戻し、出力の画素表現への詰め替え

use crate::diff::Rect;

/// 画素列を出力の画素表現へ直しながら `out` へ追記する
///
/// `in_bpp` と `out_bpp` が等しければそのまま複製し、`out_bpp` が小さければ
/// 各画素の先頭 `out_bpp` バイトだけを残す。
pub(crate) fn append_pixels(pixels: &[u8], in_bpp: usize, out_bpp: usize, out: &mut Vec<u8>) {
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
pub(crate) fn crop(
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

/// 連続バッファ `region` を `rect` の位置へ書き戻す
///
/// [`crop`] の逆で、`data` は `stride` バイトの行が隙間なく並んでいること。
/// `region` は画素表現を変えずに切り出したものであること。
pub(crate) fn paste(data: &mut [u8], region: &[u8], rect: Rect, stride: usize, bpp: usize) {
    let row_len = rect.width as usize * bpp;
    let head = rect.y as usize * stride + rect.x as usize * bpp;
    for (y, row) in region.chunks_exact(row_len).enumerate() {
        let start = head + y * stride;
        data[start..start + row_len].copy_from_slice(row);
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

    /// 切り出した領域を貼り戻すと元のキャンバスに戻り、矩形の外は動かない
    #[test]
    fn pasting_a_cropped_region_restores_the_canvas() {
        let original = canvas();
        let rect = Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };

        let mut region = Vec::new();
        crop(&original, rect, STRIDE, 4, 4, &mut region);

        let mut target = vec![0xAA; original.len()];
        paste(&mut target, &region, rect, STRIDE, 4);

        for (index, byte) in target.iter().enumerate() {
            let pixel = index / 4;
            let (x, y) = (pixel % 4, pixel / 4);
            let inside = (1..3).contains(&x) && (1..3).contains(&y);
            let expected = if inside { original[index] } else { 0xAA };
            assert_eq!(*byte, expected, "({x}, {y}) の {} バイト目", index % 4);
        }
    }
}
