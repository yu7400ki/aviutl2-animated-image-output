//! キャンバスの追跡と、フレームの載せ方の決定

use crate::layout::{ColorType, Layout};
use crate::normalize::normalize;
use anim_core::{Rect, dirty_rect};

/// キャンバスの1画素のバイト数
const PIXEL: usize = 4;

/// 完全不透明を表すα
const OPAQUE: u8 = u8::MAX;

/// 差分の無いキャンバスへ載せる最小の矩形
const SINGLE_PIXEL: Rect = Rect {
    x: 0,
    y: 0,
    width: 1,
    height: 1,
};

/// 写した入力の載せ方と、保留中のフレームの廃棄方法
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    /// キャンバス上の矩形。`x` と `y` は偶数
    pub(crate) rect: Rect,
    /// 透過画素を下のキャンバスへ重ねるか
    pub(crate) blend: bool,
    /// 保留中のフレームを表示した後にその矩形を抜くか
    pub(crate) dispose: bool,
}

/// 前のフレームまでを描いたキャンバス
///
/// 正規化後の入力をRGBAで持つ。廃棄方法が2値なので、前のフレームを描いた後の
/// 画素と、そこから前のフレームの矩形を抜いた画素の2面を追う。
pub(crate) struct Canvas {
    /// 前のフレームを描いた画素。まだ何も描いていなければ空
    drawn: Vec<u8>,
    /// [`Self::drawn`] から前のフレームの矩形を抜いた画素
    disposed: Vec<u8>,
    /// 正規化して写した入力
    staged: Vec<u8>,
    /// キャンバス全体を覆う矩形
    whole: Rect,
    /// 1行のバイト数
    stride: usize,
}

impl Canvas {
    /// `layout` の大きさの、まだ何も描いていないキャンバスを作る
    pub(crate) fn new(layout: &Layout) -> Self {
        let stride = layout.width as usize * PIXEL;
        Canvas {
            drawn: Vec::new(),
            disposed: Vec::new(),
            staged: Vec::with_capacity(stride * layout.height as usize),
            whole: layout.whole(),
            stride,
        }
    }

    /// 入力を正規化してRGBAへ写す
    ///
    /// `data` は `color_type` の画素が [`Layout`] のとおりに並んでいること。
    pub(crate) fn stage(&mut self, data: &[u8], color_type: ColorType) {
        self.staged.clear();
        match color_type {
            ColorType::Rgba8 => {
                self.staged.extend_from_slice(data);
                normalize(&mut self.staged);
            }
            ColorType::Rgb8 => {
                self.staged.reserve(data.len() / 3 * PIXEL);
                for pixel in data.chunks_exact(3) {
                    self.staged
                        .extend_from_slice(&[pixel[0], pixel[1], pixel[2], OPAQUE]);
                }
            }
        }
    }

    /// 写した入力 (RGBA)
    pub(crate) fn staged(&self) -> &[u8] {
        &self.staged
    }

    /// 写した入力を重ねる先のキャンバス
    ///
    /// `dispose` は保留中のフレームの矩形を抜くかどうか。
    pub(crate) fn base(&self, dispose: bool) -> &[u8] {
        if dispose { &self.disposed } else { &self.drawn }
    }

    /// 写した入力の載せ方を決める
    ///
    /// `disposable` は、矩形を抜く廃棄方法を載せられるフレームが保留されて
    /// いること。抜いた側の矩形が狭ければそちらを採る。写した入力がキャンバスと
    /// 一致していれば `None`。
    pub(crate) fn place(&self, disposable: bool) -> Option<Placement> {
        if self.drawn.is_empty() {
            return Some(Placement {
                rect: self.whole,
                blend: false,
                dispose: false,
            });
        }

        let kept = self.dirty(&self.drawn)?;
        let cleared = disposable.then(|| self.dirty(&self.disposed).unwrap_or(SINGLE_PIXEL));
        let (rect, dispose) = match cleared {
            Some(cleared) if cleared.area() < kept.area() => (cleared, true),
            _ => (kept, false),
        };

        Some(Placement {
            rect,
            blend: self.blendable(self.base(dispose), rect),
            dispose,
        })
    }

    /// 写した入力をキャンバスへ据える
    ///
    /// `rect` は据えたフレームの矩形。抜いた後のキャンバスはここを透過にしたものになる。
    pub(crate) fn commit(&mut self, rect: Rect) {
        std::mem::swap(&mut self.drawn, &mut self.staged);
        self.disposed.clear();
        self.disposed.extend_from_slice(&self.drawn);

        let row_len = rect.width as usize * PIXEL;
        let head = rect.y as usize * self.stride + rect.x as usize * PIXEL;
        for y in 0..rect.height as usize {
            let at = head + y * self.stride;
            self.disposed[at..at + row_len].fill(0);
        }
    }

    /// 写した入力と `base` の差分矩形。オフセットは偶数へ寄る
    fn dirty(&self, base: &[u8]) -> Option<Rect> {
        dirty_rect(base, &self.staged, self.stride, PIXEL).map(snap_to_even)
    }

    /// `rect` の中の写した入力を `base` の上へ重ねられるか
    ///
    /// 完全不透明な画素は重ねても入力そのものになり、`base` と一致する画素は
    /// 重ねる先がその値なので入力へ戻る。
    fn blendable(&self, base: &[u8], rect: Rect) -> bool {
        let row_len = rect.width as usize * PIXEL;
        let head = rect.y as usize * self.stride + rect.x as usize * PIXEL;
        (0..rect.height as usize).all(|y| {
            let at = head + y * self.stride;
            self.staged[at..at + row_len]
                .chunks_exact(PIXEL)
                .zip(base[at..at + row_len].chunks_exact(PIXEL))
                .all(|(staged, base)| staged[3] == OPAQUE || staged == base)
        })
    }
}

/// 矩形のオフセットを偶数へ寄せる
///
/// 奇数分を幅・高さへ吸収して左・上へ広げる。右端と下端は動かない。
fn snap_to_even(rect: Rect) -> Rect {
    let x = rect.x & !1;
    let y = rect.y & !1;
    Rect {
        x,
        y,
        width: rect.width + (rect.x - x),
        height: rect.height + (rect.y - y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x3のキャンバスを覆う配置
    fn layout(color_type: ColorType) -> Layout {
        Layout::new(4, 3, color_type).unwrap()
    }

    /// 画素ごとに値の違う不透明なRGBA
    fn ramp(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| [(x * 7) as u8, (y * 11) as u8, (x + y) as u8, 0xFF])
            })
            .collect()
    }

    /// 透過の面に不透明な四角を1つ置いたRGBA
    fn sprite(width: u32, height: u32, at: (u32, u32), size: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    if x.wrapping_sub(at.0) < size && y.wrapping_sub(at.1) < size {
                        [0x20, 0x40, 0x60, 0xFF]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
            })
            .collect()
    }

    /// フレームを1つ据えたキャンバス
    fn canvas_with(layout: &Layout, data: &[u8]) -> Canvas {
        let mut canvas = Canvas::new(layout);
        canvas.stage(data, layout.color_type);
        let placement = canvas.place(false).expect("先頭フレームは全面を持つ");
        canvas.commit(placement.rect);
        canvas
    }

    #[test]
    fn an_odd_offset_grows_the_rect_towards_the_origin() {
        for (rect, expected) in [
            ((1, 1, 4, 4), (0, 0, 5, 5)),
            ((3, 6, 2, 2), (2, 6, 3, 2)),
            ((6, 3, 2, 2), (6, 2, 2, 3)),
            ((0, 0, 7, 5), (0, 0, 7, 5)),
            ((2, 4, 1, 1), (2, 4, 1, 1)),
        ] {
            let (x, y, width, height) = rect;
            let (ex, ey, ewidth, eheight) = expected;
            assert_eq!(
                snap_to_even(Rect {
                    x,
                    y,
                    width,
                    height
                }),
                Rect {
                    x: ex,
                    y: ey,
                    width: ewidth,
                    height: eheight
                },
                "{rect:?}"
            );
        }
    }

    /// 広げても右端・下端は動かないので、キャンバスからはみ出さない
    #[test]
    fn snapping_keeps_the_far_edges_where_they_were() {
        for x in 0..8u32 {
            for y in 0..8u32 {
                let rect = Rect {
                    x,
                    y,
                    width: 8 - x,
                    height: 8 - y,
                };
                let snapped = snap_to_even(rect);
                assert_eq!(snapped.x % 2, 0, "{rect:?}");
                assert_eq!(snapped.y % 2, 0, "{rect:?}");
                assert_eq!(snapped.x + snapped.width, rect.x + rect.width, "{rect:?}");
                assert_eq!(snapped.y + snapped.height, rect.y + rect.height, "{rect:?}");
            }
        }
    }

    /// 先頭のフレームは比べる相手が無いので全面を上書きする
    #[test]
    fn the_first_frame_covers_the_whole_canvas() {
        let layout = layout(ColorType::Rgba8);
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&vec![0; layout.frame_len], ColorType::Rgba8);

        assert_eq!(
            canvas.place(false),
            Some(Placement {
                rect: layout.whole(),
                blend: false,
                dispose: false,
            })
        );
    }

    #[test]
    fn a_frame_equal_to_the_canvas_has_no_placement() {
        let layout = layout(ColorType::Rgba8);
        let data = ramp(layout.width, layout.height);
        let mut canvas = canvas_with(&layout, &data);

        canvas.stage(&data, ColorType::Rgba8);
        assert_eq!(canvas.place(true), None);
    }

    /// 透過下のRGBだけが違うフレームは、正規化を経て差分が無くなる
    #[test]
    fn a_frame_differing_only_under_transparency_has_no_placement() {
        let layout = layout(ColorType::Rgba8);
        let mut first = ramp(layout.width, layout.height);
        first[4..8].copy_from_slice(&[10, 20, 30, 0]);
        let mut second = first.clone();
        second[4..8].copy_from_slice(&[90, 80, 70, 0]);

        let mut canvas = canvas_with(&layout, &first);

        canvas.stage(&second, ColorType::Rgba8);
        assert_eq!(canvas.place(true), None);
    }

    /// 差分は変わった画素を囲み、オフセットは偶数へ寄る
    #[test]
    fn a_changed_pixel_yields_the_rect_around_it() {
        let layout = Layout::new(8, 6, ColorType::Rgba8).unwrap();
        let data = ramp(layout.width, layout.height);
        let mut canvas = canvas_with(&layout, &data);

        let mut changed = data.clone();
        let at = (3 * 8 + 5) * 4;
        changed[at..at + 4].copy_from_slice(&[1, 2, 3, 255]);
        canvas.stage(&changed, ColorType::Rgba8);

        let placement = canvas.place(true).expect("変わった画素がある");
        assert_eq!(
            placement.rect,
            Rect {
                x: 4,
                y: 2,
                width: 2,
                height: 2
            }
        );
    }

    /// RGBの入力はα = 255 のRGBAとしてキャンバスに載る
    #[test]
    fn an_rgb_frame_is_carried_as_opaque_rgba() {
        let layout = layout(ColorType::Rgb8);
        let data: Vec<u8> = (0..layout.frame_len as u8).collect();
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&data, ColorType::Rgb8);

        let expected: Vec<u8> = data
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 0xFF])
            .collect();
        assert_eq!(canvas.staged(), expected);
    }

    /// 据えたフレームが次のフレームの比較相手になる
    #[test]
    fn the_committed_frame_becomes_the_canvas() {
        let layout = layout(ColorType::Rgba8);
        let first = ramp(layout.width, layout.height);
        let mut second = first.clone();
        second[0] ^= 0xFF;

        let mut canvas = canvas_with(&layout, &first);
        canvas.stage(&second, ColorType::Rgba8);
        let placement = canvas.place(false).expect("画素が1つ変わっている");
        canvas.commit(placement.rect);

        canvas.stage(&second, ColorType::Rgba8);
        assert_eq!(canvas.place(false), None);
        canvas.stage(&first, ColorType::Rgba8);
        assert!(canvas.place(false).is_some());
    }

    /// 離れた場所へ動く不透明な四角は、前の場所を抜いた方が矩形が狭くなる
    #[test]
    fn clearing_the_previous_rect_wins_when_it_shrinks_the_next_one() {
        let layout = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        let mut canvas = canvas_with(&layout, &sprite(16, 16, (0, 0), 4));

        canvas.stage(&sprite(16, 16, (10, 10), 4), ColorType::Rgba8);
        assert_eq!(
            canvas.place(true),
            Some(Placement {
                rect: Rect {
                    x: 10,
                    y: 10,
                    width: 4,
                    height: 4
                },
                blend: true,
                dispose: true,
            })
        );
    }

    /// 抜ける保留中のフレームが無ければ、抜かない仮定だけが候補になる
    #[test]
    fn without_a_pending_frame_the_rect_is_taken_from_the_drawn_canvas() {
        let layout = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        let mut canvas = canvas_with(&layout, &sprite(16, 16, (0, 0), 4));

        canvas.stage(&sprite(16, 16, (10, 10), 4), ColorType::Rgba8);
        assert_eq!(
            canvas.place(false),
            Some(Placement {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 14,
                    height: 14
                },
                blend: false,
                dispose: false,
            })
        );
    }

    /// 抜いた跡そのものになるフレームは、原点の1画素だけで表せる
    #[test]
    fn a_frame_equal_to_the_disposed_canvas_takes_a_single_pixel() {
        let layout = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&sprite(16, 16, (0, 0), 16), ColorType::Rgba8);
        canvas.commit(layout.whole());
        canvas.stage(&sprite(16, 16, (4, 4), 4), ColorType::Rgba8);
        let placement = canvas.place(false).expect("四角が縮んでいる");
        canvas.commit(placement.rect);

        canvas.stage(&vec![0; layout.frame_len], ColorType::Rgba8);
        assert_eq!(
            canvas.place(true),
            Some(Placement {
                rect: SINGLE_PIXEL,
                blend: true,
                dispose: true,
            })
        );
    }

    /// 矩形の中の透過画素がキャンバスと食い違うと、重ねる形では表せない
    #[test]
    fn a_transparent_pixel_over_a_different_canvas_forbids_blending() {
        let layout = Layout::new(8, 8, ColorType::Rgba8).unwrap();
        let opaque = ramp(8, 8);
        let mut canvas = canvas_with(&layout, &opaque);

        let mut punched = opaque.clone();
        let at = (3 * 8 + 3) * 4;
        punched[at..at + 4].fill(0);
        canvas.stage(&punched, ColorType::Rgba8);

        let placement = canvas.place(true).expect("画素が1つ変わっている");
        assert!(!placement.blend);
    }

    /// 不透明な画素だけの矩形は、そのまま重ねても入力になる
    #[test]
    fn an_opaque_rect_can_always_be_blended() {
        let layout = Layout::new(8, 8, ColorType::Rgba8).unwrap();
        let opaque = ramp(8, 8);
        let mut canvas = canvas_with(&layout, &opaque);

        let mut repainted = opaque.clone();
        let at = (3 * 8 + 3) * 4;
        repainted[at..at + 4].copy_from_slice(&[9, 9, 9, 0xFF]);
        canvas.stage(&repainted, ColorType::Rgba8);

        let placement = canvas.place(true).expect("画素が1つ変わっている");
        assert!(placement.blend);
    }
}
