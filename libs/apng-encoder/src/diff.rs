//! フレーム間差分の外接矩形

/// 画素単位の矩形領域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Rect {
    /// 含む画素の数
    pub(crate) fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// 2つのフレームで異なる画素をすべて含む最小の矩形を求める
///
/// `prev` と `curr` は同じ長さで、`stride` バイトの行が隙間なく並んでいること。
/// `stride` は `bpp` の整数倍で、`stride / bpp` が画像の幅と一致すること
/// (行末に画素以外のバイトがあると、幅を超える `x` を返す)。
/// 差分がまったく無い場合は `None`。
pub(crate) fn dirty_rect(prev: &[u8], curr: &[u8], stride: usize, bpp: usize) -> Option<Rect> {
    let mut top = None;
    let mut bottom = 0usize;
    let mut left = usize::MAX;
    let mut right = 0usize;

    let rows = prev.chunks_exact(stride).zip(curr.chunks_exact(stride));
    for (y, (p, c)) in rows.enumerate() {
        if p == c {
            continue;
        }

        // 行が一致しない以上、前後の一致部分を除いた範囲は空にならない
        let head = p.iter().zip(c).take_while(|(a, b)| a == b).count();
        let tail = p.iter().zip(c).rev().take_while(|(a, b)| a == b).count();

        left = left.min(head / bpp);
        right = right.max((stride - 1 - tail) / bpp);
        top.get_or_insert(y);
        bottom = y;
    }

    let top = top?;
    Some(Rect {
        x: left as u32,
        y: top as u32,
        width: (right - left + 1) as u32,
        height: (bottom - top + 1) as u32,
    })
}

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
    const TRANSPARENT: [u8; RGBA] = [0; RGBA];

    let start = out.len();
    out.reserve(rect.width as usize * rect.height as usize * RGBA);

    let mut collapsed = false;
    for (prev, curr) in region_rows(prev, curr, stride, rect) {
        for (p, c) in prev.chunks_exact(RGBA).zip(curr.chunks_exact(RGBA)) {
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
#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 7;
    const HEIGHT: usize = 5;

    fn canvas(bpp: usize) -> Vec<u8> {
        vec![0x40; WIDTH * HEIGHT * bpp]
    }

    fn set_pixel(frame: &mut [u8], x: usize, y: usize, bpp: usize, value: u8) {
        let start = (y * WIDTH + x) * bpp;
        frame[start..start + bpp].fill(value);
    }

    fn rect_of(prev: &[u8], curr: &[u8], bpp: usize) -> Option<Rect> {
        dirty_rect(prev, curr, WIDTH * bpp, bpp)
    }

    fn expect(x: u32, y: u32, width: u32, height: u32) -> Option<Rect> {
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }

    #[test]
    fn identical_frames_have_no_rect() {
        for bpp in [3, 4] {
            let frame = canvas(bpp);
            assert_eq!(rect_of(&frame, &frame, bpp), None, "bpp={bpp}");
        }
    }

    #[test]
    fn a_single_pixel_yields_a_unit_rect() {
        for bpp in [3, 4] {
            let prev = canvas(bpp);
            let mut curr = prev.clone();
            set_pixel(&mut curr, 3, 2, bpp, 0xFF);

            assert_eq!(rect_of(&prev, &curr, bpp), expect(3, 2, 1, 1), "bpp={bpp}");
        }
    }

    /// 1画素のうち1チャンネルだけが違っても矩形は1画素に収まる
    #[test]
    fn a_single_channel_yields_a_unit_rect() {
        for bpp in [3, 4] {
            let prev = canvas(bpp);
            let mut curr = prev.clone();
            curr[(2 * WIDTH + 5) * bpp + bpp - 1] ^= 0xFF;

            assert_eq!(rect_of(&prev, &curr, bpp), expect(5, 2, 1, 1), "bpp={bpp}");
        }
    }

    #[test]
    fn each_corner_yields_a_unit_rect() {
        let corners = [
            (0, 0),
            (WIDTH - 1, 0),
            (0, HEIGHT - 1),
            (WIDTH - 1, HEIGHT - 1),
        ];
        for bpp in [3, 4] {
            for (x, y) in corners {
                let prev = canvas(bpp);
                let mut curr = prev.clone();
                set_pixel(&mut curr, x, y, bpp, 0xFF);

                assert_eq!(
                    rect_of(&prev, &curr, bpp),
                    expect(x as u32, y as u32, 1, 1),
                    "bpp={bpp} ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn an_edge_row_yields_a_full_width_rect() {
        for bpp in [3, 4] {
            let prev = canvas(bpp);
            let mut curr = prev.clone();
            for x in 0..WIDTH {
                set_pixel(&mut curr, x, HEIGHT - 1, bpp, 0xFF);
            }

            assert_eq!(
                rect_of(&prev, &curr, bpp),
                expect(0, HEIGHT as u32 - 1, WIDTH as u32, 1),
                "bpp={bpp}"
            );
        }
    }

    #[test]
    fn an_edge_column_yields_a_full_height_rect() {
        for bpp in [3, 4] {
            let prev = canvas(bpp);
            let mut curr = prev.clone();
            for y in 0..HEIGHT {
                set_pixel(&mut curr, WIDTH - 1, y, bpp, 0xFF);
            }

            assert_eq!(
                rect_of(&prev, &curr, bpp),
                expect(WIDTH as u32 - 1, 0, 1, HEIGHT as u32),
                "bpp={bpp}"
            );
        }
    }

    #[test]
    fn a_full_change_yields_the_whole_canvas() {
        for bpp in [3, 4] {
            let prev = canvas(bpp);
            let curr = vec![0x80; prev.len()];

            assert_eq!(
                rect_of(&prev, &curr, bpp),
                expect(0, 0, WIDTH as u32, HEIGHT as u32),
                "bpp={bpp}"
            );
        }
    }

    /// 離れた2画素の外接矩形は、変更されていない画素を含む
    #[test]
    fn disjoint_changes_span_a_bounding_rect() {
        for bpp in [3, 4] {
            let prev = canvas(bpp);
            let mut curr = prev.clone();
            set_pixel(&mut curr, 5, 1, bpp, 0xFF);
            set_pixel(&mut curr, 1, 3, bpp, 0xFF);

            assert_eq!(rect_of(&prev, &curr, bpp), expect(1, 1, 5, 3), "bpp={bpp}");
        }
    }

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
