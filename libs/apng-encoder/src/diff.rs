//! フレーム間差分の外接矩形

/// 画素単位の矩形領域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// 2つのフレームで異なる画素をすべて含む最小の矩形を求める
///
/// `prev` と `curr` は同じ長さで、`stride` バイトの行が隙間なく並んでいること。
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

        let mut diff = p
            .iter()
            .zip(c)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i);
        let Some(first) = diff.next() else {
            continue;
        };
        let last = diff.next_back().unwrap_or(first);

        left = left.min(first / bpp);
        right = right.max(last / bpp);
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
}
