//! フレーム間差分の外接矩形

/// 画素単位の矩形領域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// 含む画素の数
    pub fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// まとめて比べるバイト数
const STEP: usize = 32;

/// `at` から始まる、2つのフレームで一致する画素が続く終端のバイト位置
///
/// `previous` と `frame` は同じ長さで、`at` は画素の境界にあること。返す位置も
/// 画素の境界で、`at` の画素が異なれば `at` をそのまま返す。
pub fn unchanged_run<const BPP: usize>(previous: &[u8], frame: &[u8], at: usize) -> usize {
    let len = frame.len();
    let mut end = at;
    while end + STEP <= len && previous[end..end + STEP] == frame[end..end + STEP] {
        end += STEP;
    }
    // 塊の境界は画素の途中に落ちうるので、戻してから1画素ずつ詰める
    end -= (end - at) % BPP;
    while end + BPP <= len && previous[end..end + BPP] == frame[end..end + BPP] {
        end += BPP;
    }
    end
}

/// 2つのフレームで異なる画素をすべて含む最小の矩形を求める
///
/// `prev` と `curr` は同じ長さで、`stride` バイトの行が隙間なく並んでいること。
/// `stride` は `bpp` の整数倍で、`stride / bpp` が画像の幅と一致すること
/// (行末に画素以外のバイトがあると、幅を超える `x` を返す)。
/// 差分がまったく無い場合は `None`。
pub fn dirty_rect(prev: &[u8], curr: &[u8], stride: usize, bpp: usize) -> Option<Rect> {
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

    /// 塊の境界が画素の途中に落ちても、一致する画素を終端に含む
    ///
    /// 塊で比べる段は画素の境界を跨ぐ。境界へ戻したあと1画素ずつ詰め直すので、
    /// 塊の末尾に収まった一致画素も終端に入る。
    #[test]
    fn a_run_keeps_the_pixels_that_straddle_a_block_boundary() {
        const BPP: usize = 3;
        // 塊を1つ跨いだ先で画素の境界に揃う長さ。塊の終端はその手前の画素の途中に落ちる
        const MATCHED: usize = (STEP / BPP + 1) * BPP;

        let previous = vec![0x11; MATCHED + STEP];
        let mut frame = previous.clone();
        frame[MATCHED..].fill(0x22);

        assert_eq!(unchanged_run::<BPP>(&previous, &frame, 0), MATCHED);
        assert_eq!(
            unchanged_run::<BPP>(&previous, &frame, MATCHED),
            MATCHED,
            "食い違う画素から始めたのに進んでいる"
        );
    }

    /// すべて一致するときは画素の境界に収まる終端を返す
    #[test]
    fn a_run_over_identical_frames_stops_on_a_pixel_boundary() {
        const BPP: usize = 4;
        let frame = vec![0x33; 100];
        assert_eq!(unchanged_run::<BPP>(&frame, &frame, 0), 100);

        // 端数のある長さでは、収まる画素までで止まる
        let frame = vec![0x33; 102];
        assert_eq!(unchanged_run::<BPP>(&frame, &frame, 0), 100);
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
