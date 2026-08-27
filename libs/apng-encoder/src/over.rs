//! blend_op=OVERで合成する画素の詰め方

use anim_core::Rect;

/// RGBA8の1画素のバイト数
const RGBA: usize = 4;

/// 矩形の内側の行を `prev` と `curr` の組で上から順に返す
fn region_rows<'a>(
    prev: &'a [u8],
    curr: &'a [u8],
    stride: usize,
    rect: Rect,
) -> impl Iterator<Item = (&'a [u8], &'a [u8])> {
    let head = rect.y as usize * stride + rect.x as usize * RGBA;
    let row_len = rect.width as usize * RGBA;
    (0..rect.height as usize).map(move |y| {
        let start = head + y * stride;
        (&prev[start..start + row_len], &curr[start..start + row_len])
    })
}

/// 行の組を上から順に潰して `out` へ追記する
fn pack_over_rows<'a>(rows: impl Iterator<Item = (&'a [u8], &'a [u8])>, out: &mut Vec<u8>) -> bool {
    const TRANSPARENT: [u8; RGBA] = [0; RGBA];

    let start = out.len();
    let mut collapsed = false;
    for (prev_row, curr_row) in rows {
        for (p, c) in prev_row.chunks_exact(RGBA).zip(curr_row.chunks_exact(RGBA)) {
            if p == c {
                collapsed = true;
                out.extend_from_slice(&TRANSPARENT);
            } else if c[RGBA - 1] == u8::MAX {
                out.extend_from_slice(c);
            } else {
                out.truncate(start);
                return false;
            }
        }
    }

    if !collapsed {
        out.truncate(start);
    }
    collapsed
}

/// 矩形を切り出し、`prev` と一致する画素を完全な透明にして `out` へ追記する
///
/// `prev` と `curr` はRGBA8で、`stride` バイトの行が隙間なく並んでいること。
/// 追記した領域をblend_op=OVERで合成すると、矩形の中は `curr` と一致する。
///
/// 次のどちらかに当たる矩形はOVERの候補にならず、`out` を変えずに偽を返す。
/// - 変化した画素に不透明でないものがある。完全に透明な画素はキャンバスに埋もれ、
///   半透明の画素はキャンバスと混ざるため、どちらも元の値に戻らない
/// - `prev` と一致する画素が1つも無い。潰す先が無く、切り出した結果が
///   blend_op=SOURCEの候補と同じバイト列になる
pub(crate) fn pack_over(
    prev: &[u8],
    curr: &[u8],
    stride: usize,
    rect: Rect,
    out: &mut Vec<u8>,
) -> bool {
    out.reserve(rect.width as usize * rect.height as usize * RGBA);
    pack_over_rows(region_rows(prev, curr, stride, rect), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 7;
    const HEIGHT: usize = 5;
    const RGBA_STRIDE: usize = WIDTH * RGBA;

    /// 一様なRGBA8のキャンバス
    fn rgba_canvas(color: [u8; RGBA]) -> Vec<u8> {
        color.repeat(WIDTH * HEIGHT)
    }

    fn set_rgba(frame: &mut [u8], x: usize, y: usize, color: [u8; RGBA]) {
        let start = (y * WIDTH + x) * RGBA;
        frame[start..start + RGBA].copy_from_slice(&color);
    }

    /// キャンバス全体
    const WHOLE: Rect = Rect {
        x: 0,
        y: 0,
        width: WIDTH as u32,
        height: HEIGHT as u32,
    };

    fn packed(prev: &[u8], curr: &[u8], rect: Rect) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        pack_over(prev, curr, RGBA_STRIDE, rect, &mut out).then_some(out)
    }

    /// 変化の無い矩形は、丸ごと完全な透明へ潰れる
    #[test]
    fn an_unchanged_region_collapses_to_transparent() {
        let frame = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);

        let out = packed(&frame, &frame, WHOLE).expect("候補が立つ");
        assert_eq!(out, vec![0u8; WIDTH * HEIGHT * RGBA]);
    }

    /// 不透明に変わった画素だけがそのまま残る
    #[test]
    fn an_opaque_change_is_kept_while_the_rest_collapses() {
        let prev = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        let mut curr = prev.clone();
        set_rgba(&mut curr, 3, 2, [0xFF, 0x00, 0x00, 0xFF]);

        let out = packed(&prev, &curr, WHOLE).expect("候補が立つ");
        let mut expected = vec![0u8; WIDTH * HEIGHT * RGBA];
        let at = (2 * WIDTH + 3) * RGBA;
        expected[at..at + RGBA].copy_from_slice(&[0xFF, 0x00, 0x00, 0xFF]);
        assert_eq!(out, expected);
    }

    /// 変化していない画素は、アルファがいくつでも潰れる
    #[test]
    fn unchanged_pixels_collapse_whatever_their_alpha() {
        let mut prev = rgba_canvas([0x10, 0x20, 0x30, 0x80]);
        set_rgba(&mut prev, 1, 1, [0x05, 0x06, 0x07, 0x00]);
        let mut curr = prev.clone();
        set_rgba(&mut curr, 3, 2, [0xFF, 0x00, 0x00, 0xFF]);

        let out = packed(&prev, &curr, WHOLE).expect("候補が立つ");
        let at = (WIDTH + 1) * RGBA;
        assert_eq!(&out[at..at + RGBA], &[0, 0, 0, 0]);
    }

    /// 不透明でない画素へ変わった矩形は候補にならない
    #[test]
    fn a_change_that_is_not_opaque_is_rejected() {
        let prev = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        for alpha in [0x00, 0x01, 0x80, 0xFE] {
            let mut curr = prev.clone();
            set_rgba(&mut curr, 3, 2, [0x11, 0x22, 0x33, alpha]);

            assert_eq!(packed(&prev, &curr, WHOLE), None, "alpha={alpha}");
        }
    }

    /// 不透明な変化に紛れた1画素でも、不透明でなければ候補にならない
    #[test]
    fn a_single_pixel_that_is_not_opaque_rejects_the_region() {
        let prev = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        let mut curr = prev.clone();
        for x in 0..WIDTH {
            set_rgba(&mut curr, x, 2, [0xFF, 0xFF, 0x00, 0xFF]);
        }
        set_rgba(&mut curr, 4, 2, [0xFF, 0xFF, 0x00, 0x80]);

        assert_eq!(packed(&prev, &curr, WHOLE), None);
    }

    /// 潰す先が1画素も無い矩形は候補にならない
    #[test]
    fn a_region_without_a_matching_pixel_is_rejected() {
        let prev = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        let curr = rgba_canvas([0x40, 0x50, 0x60, 0xFF]);

        assert_eq!(packed(&prev, &curr, WHOLE), None);
    }

    /// 矩形の外の変化は見ない
    #[test]
    fn changes_outside_the_rect_are_ignored() {
        let prev = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        let mut curr = prev.clone();
        set_rgba(&mut curr, 0, 0, [0x11, 0x22, 0x33, 0x80]);
        set_rgba(&mut curr, 3, 2, [0xFF, 0x00, 0x00, 0xFF]);
        let rect = Rect {
            x: 2,
            y: 1,
            width: 3,
            height: 3,
        };

        let out = packed(&prev, &curr, rect).expect("候補が立つ");
        assert_eq!(out.len(), 3 * 3 * RGBA);
        let at = (rect.width as usize + 1) * RGBA;
        assert_eq!(&out[at..at + RGBA], &[0xFF, 0x00, 0x00, 0xFF]);
    }

    /// 候補が立たなかった矩形は、既にある内容を残す
    #[test]
    fn a_rejected_region_leaves_the_buffer_untouched() {
        let prev = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        let mut curr = prev.clone();
        set_rgba(&mut curr, 3, 2, [0x11, 0x22, 0x33, 0x80]);

        let mut out = vec![0xAA, 0xBB];
        assert!(!pack_over(&prev, &curr, RGBA_STRIDE, WHOLE, &mut out));
        assert_eq!(out, [0xAA, 0xBB]);
    }

    /// 切り出した領域は、既にある内容の後ろへ追記される
    #[test]
    fn the_region_is_appended_after_the_existing_content() {
        let frame = rgba_canvas([0x10, 0x20, 0x30, 0xFF]);
        let rect = Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };

        let mut out = vec![0xAA];
        assert!(pack_over(&frame, &frame, RGBA_STRIDE, rect, &mut out));
        assert_eq!(out, [0xAA, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
}
