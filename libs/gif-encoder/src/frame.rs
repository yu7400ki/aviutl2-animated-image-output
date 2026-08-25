//! 差分矩形・廃棄方法・透過ランの決定

use crate::block::DISPOSAL_DO_NOT_DISPOSE;
use crate::error::Error;
use crate::layout::{ColorType, Layout};
use crate::normalize::TRANSPARENT;
use crate::table::Palette;
use anim_core::{Rect, dirty_rect};

/// 差分の無いフレームが書く矩形
///
/// 画像記述子は幅・高さ0を表現できないため、1画素だけ書く。その1画素は
/// キャンバスと同じ内容になるので画面は変わらず、宣言したフレーム数と
/// 再生時間だけが保たれる。
const UNCHANGED: Rect = Rect {
    x: 0,
    y: 0,
    width: 1,
    height: 1,
};

/// `base` から `frame` へ変わった画素をすべて含む矩形
///
/// 差分が無い場合は [`UNCHANGED`]。
pub(crate) fn bounding_rect(layout: &Layout, base: &[u8], frame: &[u8]) -> Rect {
    dirty_rect(base, frame, layout.stride, layout.bytes_per_pixel).unwrap_or(UNCHANGED)
}

/// 画素が完全透過の標識か
fn is_transparent(pixel: &[u8]) -> bool {
    u32::from_le_bytes(pixel.try_into().expect("1画素4バイト")) == TRANSPARENT
}

/// 描画後の色の面
///
/// 添字ではなく色で持つ。キャンバスの色が常に現在のカラーテーブルに載って
/// いるとは限らないため、添字の面ではキャンバスを表現できない。
///
/// 入力と同じ画素表現で足りるのは、まだどこも描かれていない状態を
/// [`Canvas::is_empty`] が別に表しているため。先頭フレームの矩形は論理画面
/// 全体なので、それを描いた後は全画素が描かれた色を持つ。以降に現れる透過は
/// すべて素材自身のもので、アルファを持つ入力の表現にそのまま収まる。
pub(crate) struct Canvas {
    layout: Layout,
    /// 論理画面と同じ大きさの画素列。先頭フレームを描くまでは空
    pixels: Vec<u8>,
}

impl Canvas {
    /// 先頭フレームを迎える前のキャンバス
    ///
    /// GIFの論理画面は最初どこも描かれておらず、透過インデックスに当たった
    /// 画素と同じ扱いになる。
    pub(crate) fn new(layout: Layout) -> Self {
        Canvas {
            layout,
            pixels: Vec::new(),
        }
    }

    /// 先頭フレームをまだ描いていないか
    pub(crate) fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }

    /// 投入されたフレームを書き出す矩形
    ///
    /// 先頭フレームは論理画面全体。以降はキャンバスとの差分の外接矩形で、
    /// 差分が無ければ1画素になる。
    pub(crate) fn rect_of(&self, frame: &[u8]) -> Rect {
        if self.is_empty() {
            return self.layout.whole();
        }

        bounding_rect(&self.layout, &self.pixels, frame)
    }

    /// このキャンバスの上で `frame` が表現できるか
    ///
    /// 透過インデックスに当たった画素はキャンバスを書き換えないため、
    /// キャンバスが不透明な位置を透過にすることはできない。
    fn expressible(&self, frame: &[u8]) -> bool {
        // 先頭フレームを迎える前の論理画面はどこも描かれておらず、透過を持たない
        // 入力では透過にする遷移そのものが起きない
        if self.is_empty() || self.layout.color_type != ColorType::Rgba8 {
            return true;
        }

        let bpp = self.layout.bytes_per_pixel;
        self.pixels
            .chunks_exact(bpp)
            .zip(frame.chunks_exact(bpp))
            .all(|(canvas, pixel)| !is_transparent(pixel) || is_transparent(canvas))
    }

    /// 投入されたフレームを描画後の色の面へ写して `out` へ入れる
    ///
    /// 入力が前フレームと変わった画素だけ現在のパレットで写し、変わっていない
    /// 画素は前の描画後の色を持ち越す。持ち越した画素は画面上の色をそのまま
    /// 保つため、パレットが変わっても静止した領域は揺れない。
    ///
    /// `previous` は直前に投入されたフレームの正規化した入力。先頭フレームでは空。
    ///
    /// # Errors
    /// 写す先がテーブルに無い色があるとき [`Error::TooManyColors`]。
    pub(crate) fn render(
        &self,
        previous: &[u8],
        frame: &[u8],
        palette: &Palette,
        out: &mut Vec<u8>,
    ) -> Result<(), Error> {
        let bpp = self.layout.bytes_per_pixel;
        out.clear();
        out.reserve(self.layout.frame_len);

        // 先頭フレームには前が無く、持ち越せる色も無い
        let carried = (!previous.is_empty()).then_some((previous, self.pixels.as_slice()));
        for (at, pixel) in frame.chunks_exact(bpp).enumerate() {
            let at = at * bpp;
            if let Some((previous, drawn)) = carried
                && previous[at..at + bpp] == *pixel
            {
                out.extend_from_slice(&drawn[at..at + bpp]);
                continue;
            }
            // 透過標識は色として写さない。2値透過に中間が無いため、標識のまま
            // 残して廃棄方法の判定へ渡す
            if bpp == 4 && is_transparent(pixel) {
                out.extend_from_slice(pixel);
                continue;
            }
            let index = palette.index_of(pixel, bpp).ok_or(Error::TooManyColors)?;
            out.extend_from_slice(&palette.color_at(index).to_le_bytes()[..bpp]);
        }
        Ok(())
    }

    /// `rect` の添字列を `out` へ追記する
    ///
    /// 透過インデックスを持つテーブルでは、矩形の中でキャンバスと一致する画素を
    /// それへ置き換える。その画素はキャンバスを書き換えないまま、LZWにとって
    /// 同じ値の長いランになる。潰すのが写すより先なので、テーブルに載っていない
    /// 色を持ち越した未変更画素もそのまま潰れる。
    ///
    /// # Errors
    /// 潰されず、テーブルにも載っていない色があるとき [`Error::TooManyColors`]。
    pub(crate) fn append_indices(
        &self,
        frame: &[u8],
        rect: Rect,
        palette: &Palette,
        out: &mut Vec<u8>,
    ) -> Result<(), Error> {
        let stride = self.layout.stride;
        let bpp = self.layout.bytes_per_pixel;
        // 先頭フレームには前が無く、未変更画素そのものが存在しない
        let previous = (!self.is_empty()).then_some(self.pixels.as_slice());
        let transparent = palette.transparent();

        out.reserve(rect.area() as usize);
        for y in 0..rect.height as usize {
            let row = (rect.y as usize + y) * stride + rect.x as usize * bpp;
            for x in 0..rect.width as usize {
                let at = row + x * bpp;
                let pixel = &frame[at..at + bpp];
                let index = match (transparent, previous) {
                    (Some(transparent), Some(previous)) if previous[at..at + bpp] == *pixel => {
                        transparent
                    }
                    _ => palette.index_of(pixel, bpp).ok_or(Error::TooManyColors)?,
                };
                out.push(index);
            }
        }
        Ok(())
    }

    /// 描き終えたフレームでキャンバスを進める
    ///
    /// `rect` の外は投入されたフレームと一致しているため、書き換えるのは中だけで足りる。
    pub(crate) fn advance(&mut self, frame: &[u8], rect: Rect) {
        if self.is_empty() {
            self.pixels.resize(self.layout.frame_len, 0);
        }

        let stride = self.layout.stride;
        let bpp = self.layout.bytes_per_pixel;
        let row_len = rect.width as usize * bpp;
        let head = rect.y as usize * stride + rect.x as usize * bpp;
        for y in 0..rect.height as usize {
            let at = head + y * stride;
            self.pixels[at..at + row_len].copy_from_slice(&frame[at..at + row_len]);
        }
    }
}

/// 保留中のフレームの廃棄方法を選ぶ
///
/// 廃棄方法は次に投入されたフレームが載るキャンバスを決めるため、そのフレームを
/// 表現できるかどうかで選ぶ。表現できる廃棄方法が無ければ `None`。
pub(crate) fn choose_disposal(canvas: &Canvas, frame: &[u8]) -> Option<u8> {
    canvas.expressible(frame).then_some(DISPOSAL_DO_NOT_DISPOSE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::Colors;

    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;

    fn layout(color_type: ColorType) -> Layout {
        Layout::new(WIDTH, HEIGHT, color_type).unwrap()
    }

    /// 全画素が不透明なRGBA8のフレーム
    fn opaque(value: u8) -> Vec<u8> {
        (0..WIDTH * HEIGHT)
            .flat_map(|i| [i as u8, 0x20, value, 0xFF])
            .collect()
    }

    fn palette_of(frames: &[&[u8]], bpp: usize) -> Palette {
        let mut colors = Colors::new();
        for frame in frames {
            colors.observe(frame, bpp);
        }
        Palette::from_colors(colors)
    }

    fn drawn(canvas: &mut Canvas, frame: &[u8]) -> Rect {
        let rect = canvas.rect_of(frame);
        canvas.advance(frame, rect);
        rect
    }

    #[test]
    fn the_first_frame_covers_the_logical_screen() {
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        assert_eq!(
            canvas.rect_of(&opaque(0x10)),
            layout(ColorType::Rgba8).whole()
        );
    }

    /// 差分の無いフレームは1画素の矩形になる
    #[test]
    fn an_identical_frame_becomes_a_unit_rect() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        drawn(&mut canvas, &opaque(0x10));
        assert_eq!(canvas.rect_of(&opaque(0x10)), UNCHANGED);
    }

    /// キャンバスは矩形の中だけを書き換え、外は投入されたフレームと一致したまま
    #[test]
    fn advancing_leaves_the_canvas_equal_to_the_frame() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        drawn(&mut canvas, &opaque(0x10));

        let mut next = opaque(0x10);
        next[4 * 4] = 0x7F;
        drawn(&mut canvas, &next);
        assert_eq!(canvas.pixels, next);
    }

    /// 不透明な画素を透過へ変える遷移は、キャンバスを残す廃棄方法では表現できない
    #[test]
    fn an_opaque_pixel_turning_transparent_has_no_disposal() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        drawn(&mut canvas, &opaque(0x10));

        let mut next = opaque(0x10);
        next[4 * 4..4 * 5].fill(0);
        assert_eq!(choose_disposal(&canvas, &next), None);
    }

    /// 透過のまま留まる画素と、透過から不透明になる画素は表現できる
    #[test]
    fn transitions_that_only_add_paint_keep_the_canvas() {
        let mut first = opaque(0x10);
        first[4 * 4..4 * 6].fill(0);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        drawn(&mut canvas, &first);

        let mut next = first.clone();
        next[4 * 4..4 * 5].copy_from_slice(&[1, 2, 3, 0xFF]);
        assert_eq!(
            choose_disposal(&canvas, &next),
            Some(DISPOSAL_DO_NOT_DISPOSE)
        );
    }

    /// RGB8の入力には透過が存在しないため、遷移の検査は要らない
    #[test]
    fn an_input_without_alpha_is_always_expressible() {
        let frame: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|i| [i as u8, 0, 0]).collect();
        let mut canvas = Canvas::new(layout(ColorType::Rgb8));
        drawn(&mut canvas, &frame);

        let zeros = vec![0u8; frame.len()];
        assert_eq!(
            choose_disposal(&canvas, &zeros),
            Some(DISPOSAL_DO_NOT_DISPOSE)
        );
    }

    /// 矩形の中でキャンバスと一致する画素は透過インデックスになる
    #[test]
    fn unchanged_pixels_inside_the_rect_become_the_transparent_index() {
        let first = opaque(0x10);
        let mut next = opaque(0x10);
        next[0] = 0x7F;
        next[(WIDTH * HEIGHT - 1) as usize * 4] = 0x7E;

        let palette = palette_of(&[&first, &next], 4);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        drawn(&mut canvas, &first);
        let rect = canvas.rect_of(&next);
        assert_eq!(rect, layout(ColorType::Rgba8).whole(), "矩形が全画面でない");

        let mut indices = Vec::new();
        canvas
            .append_indices(&next, rect, &palette, &mut indices)
            .unwrap();
        let last = indices.len() - 1;
        assert_ne!(indices[0], transparent, "変わった画素まで潰れている");
        assert_ne!(indices[last], transparent, "変わった画素まで潰れている");
        assert!(
            indices[1..last].iter().all(|&index| index == transparent),
            "変わっていない画素が潰れていない"
        );
    }

    /// 先頭フレームには未変更画素が無く、全画素がテーブルへ写る
    #[test]
    fn the_first_frame_maps_every_pixel_through_the_table() {
        let frame = opaque(0x10);
        let palette = palette_of(&[&frame], 4);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = canvas.rect_of(&frame);
        let mut indices = Vec::new();
        canvas
            .append_indices(&frame, rect, &palette, &mut indices)
            .unwrap();

        assert!(
            indices.iter().all(|&index| index != transparent),
            "先頭フレームの画素が潰れている"
        );
    }

    /// 透過インデックスを持たないテーブルでは、未変更画素もそのまま写る
    #[test]
    fn a_table_without_a_transparent_index_keeps_every_pixel() {
        let frame: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|i| [i as u8, 0, 0]).collect();
        let full: Vec<u8> = (0..256)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let palette = palette_of(&[&full], 3);
        assert_eq!(palette.transparent(), None);

        let mut canvas = Canvas::new(layout(ColorType::Rgb8));
        drawn(&mut canvas, &frame);

        let mut indices = Vec::new();
        canvas
            .append_indices(&frame, UNCHANGED, &palette, &mut indices)
            .unwrap();
        assert_eq!(indices, [0]);
    }

    /// 素材自身の透過画素は透過インデックスへ写る
    #[test]
    fn a_transparent_pixel_maps_to_the_transparent_index() {
        let mut first = opaque(0x10);
        first[..4].fill(0);

        let palette = palette_of(&[&first], 4);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = canvas.rect_of(&first);
        let mut indices = Vec::new();
        canvas
            .append_indices(&first, rect, &palette, &mut indices)
            .unwrap();

        assert_eq!(indices[0], transparent);
        assert!(indices[1..].iter().all(|&index| index != transparent));
    }

    /// 入力が変わっていない画素は、前の描画後の色を持ち越す
    ///
    /// 持ち越しはパレットを通らないため、写した先がテーブルに無くても写る。
    #[test]
    fn unchanged_pixels_carry_the_color_they_were_drawn_with() {
        let first = opaque(0x10);
        let mut second = opaque(0x10);
        second[0] = 0x7F;

        let palette = palette_of(&[&first, &second], 4);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(&[], &first, &palette, &mut rendered).unwrap();
        assert_eq!(rendered, first, "先頭フレームが写っていない");
        drawn(&mut canvas, &rendered);

        canvas
            .render(&first, &second, &palette, &mut rendered)
            .unwrap();
        assert_eq!(rendered, second);
    }

    /// 透過標識は色として写さず、標識のまま残る
    ///
    /// テーブルが透過ラン用に足したスロットは色を持たないため、標識を写す先が
    /// 無い。残した標識は廃棄方法の判定へ渡る。
    #[test]
    fn the_transparent_marker_passes_through_unmapped() {
        let first = opaque(0x10);
        let mut second = opaque(0x10);
        second[..4].fill(0);

        let palette = palette_of(&[&first], 4);
        assert_eq!(palette.index_of(&[0, 0, 0, 0], 4), None);

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(&[], &first, &palette, &mut rendered).unwrap();
        drawn(&mut canvas, &rendered);

        canvas
            .render(&first, &second, &palette, &mut rendered)
            .unwrap();
        assert_eq!(rendered[..4], [0, 0, 0, 0]);
        assert_eq!(choose_disposal(&canvas, &rendered), None);
    }

    /// 全幅でない矩形は行をまたいで切り出される
    #[test]
    fn a_partial_width_rect_is_cropped_row_by_row() {
        let frame = opaque(0x10);
        let palette = palette_of(&[&frame], 4);

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut indices = Vec::new();
        canvas
            .append_indices(&frame, rect, &palette, &mut indices)
            .unwrap();

        let expected: Vec<u8> = [1u8, 2, 5, 6]
            .iter()
            .map(|&at| {
                palette
                    .table()
                    .bytes()
                    .chunks_exact(3)
                    .position(|color| color == [at, 0x20, 0x10])
                    .unwrap() as u8
            })
            .collect();
        assert_eq!(indices, expected);
    }
}
