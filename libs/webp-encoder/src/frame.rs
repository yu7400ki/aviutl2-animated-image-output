//! キャンバスの追跡と差分矩形の決定

use crate::layout::{ColorType, Layout};
use crate::normalize::normalize;
use anim_core::{Rect, dirty_rect};

/// キャンバスの1画素のバイト数
const PIXEL: usize = 4;

/// 前のフレームまでを描いたキャンバス
///
/// 正規化後の入力をRGBAで持つ。非可逆でもデコーダの復号結果ではなく入力で
/// 追う。未変更の入力に対して前回の復号結果が画面に残るのは望んだ挙動で、
/// 復号結果を追いかける理由が無い。
pub(crate) struct Canvas {
    /// 前のフレームまでを描いた画素。まだ何も描いていなければ空
    pixels: Vec<u8>,
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
            pixels: Vec::new(),
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
                        .extend_from_slice(&[pixel[0], pixel[1], pixel[2], u8::MAX]);
                }
            }
        }
    }

    /// 写した入力 (RGBA)
    pub(crate) fn staged(&self) -> &[u8] {
        &self.staged
    }

    /// 写した入力とキャンバスの差分矩形
    ///
    /// 返る矩形のx, yは偶数。まだ何も描いていなければ全面を返し、差分が
    /// 無ければ `None`。
    pub(crate) fn dirty(&self) -> Option<Rect> {
        if self.pixels.is_empty() {
            return Some(self.whole);
        }
        dirty_rect(&self.pixels, &self.staged, self.stride, PIXEL).map(snap_to_even)
    }

    /// 写した入力をキャンバスへ据える
    pub(crate) fn commit(&mut self) {
        std::mem::swap(&mut self.pixels, &mut self.staged);
    }
}

/// 矩形のオフセットを偶数へ寄せる
///
/// ANMFは矩形のx, yを値の半分で格納するため奇数を表せない。奇数分を
/// 幅・高さへ吸収して左・上へ広げる。広げた列は未変更の画素になる。
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

    /// 画素ごとに値の違うRGBA
    fn ramp(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| [(x * 7) as u8, (y * 11) as u8, (x + y) as u8, 0xFF])
            })
            .collect()
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

    /// 先頭のフレームは比べる相手が無いので全面になる
    #[test]
    fn the_first_frame_covers_the_whole_canvas() {
        let layout = layout(ColorType::Rgba8);
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&vec![0; layout.frame_len], ColorType::Rgba8);

        assert_eq!(canvas.dirty(), Some(layout.whole()));
    }

    #[test]
    fn a_frame_equal_to_the_canvas_has_no_rect() {
        let layout = layout(ColorType::Rgba8);
        let data = ramp(layout.width, layout.height);
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&data, ColorType::Rgba8);
        canvas.commit();

        canvas.stage(&data, ColorType::Rgba8);
        assert_eq!(canvas.dirty(), None);
    }

    /// 透過下のRGBだけが違うフレームは、正規化を経て差分が無くなる
    #[test]
    fn a_frame_differing_only_under_transparency_has_no_rect() {
        let layout = layout(ColorType::Rgba8);
        let mut first = ramp(layout.width, layout.height);
        first[4..8].copy_from_slice(&[10, 20, 30, 0]);
        let mut second = first.clone();
        second[4..8].copy_from_slice(&[90, 80, 70, 0]);

        let mut canvas = Canvas::new(&layout);
        canvas.stage(&first, ColorType::Rgba8);
        canvas.commit();

        canvas.stage(&second, ColorType::Rgba8);
        assert_eq!(canvas.dirty(), None);
    }

    /// 差分は変わった画素を囲み、オフセットは偶数へ寄る
    #[test]
    fn a_changed_pixel_yields_the_rect_around_it() {
        let layout = Layout::new(8, 6, ColorType::Rgba8).unwrap();
        let data = ramp(layout.width, layout.height);
        let mut canvas = Canvas::new(&layout);
        canvas.stage(&data, ColorType::Rgba8);
        canvas.commit();

        let mut changed = data.clone();
        let at = (3 * 8 + 5) * 4;
        changed[at..at + 4].copy_from_slice(&[1, 2, 3, 255]);
        canvas.stage(&changed, ColorType::Rgba8);

        assert_eq!(
            canvas.dirty(),
            Some(Rect {
                x: 4,
                y: 2,
                width: 2,
                height: 2
            })
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

        let mut canvas = Canvas::new(&layout);
        canvas.stage(&first, ColorType::Rgba8);
        canvas.commit();
        canvas.stage(&second, ColorType::Rgba8);
        canvas.commit();

        canvas.stage(&second, ColorType::Rgba8);
        assert_eq!(canvas.dirty(), None);
        canvas.stage(&first, ColorType::Rgba8);
        assert!(canvas.dirty().is_some());
    }
}
