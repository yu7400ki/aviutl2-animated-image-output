//! 差分矩形・廃棄方法・透過ランの決定

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

/// 投入されたフレームが載る画面
///
/// 廃棄方法を適用した後のキャンバスで、差分矩形も透過ランもこの上で求める。
#[derive(Clone, Copy)]
pub(crate) struct Screen<'a> {
    layout: &'a Layout,
    /// 描かれた画素。先頭フレームを描く前は `None`
    pixels: Option<&'a [u8]>,
}

impl Screen<'_> {
    /// 投入されたフレームを書き出す矩形
    ///
    /// 先頭フレームは論理画面全体。以降は画面との差分の外接矩形で、
    /// 差分が無ければ1画素になる。
    pub(crate) fn rect_of(&self, frame: &[u8]) -> Rect {
        match self.pixels {
            None => self.layout.whole(),
            Some(pixels) => bounding_rect(self.layout, pixels, frame),
        }
    }

    /// この画面の上で `frame` が表現できるか
    ///
    /// 透過インデックスに当たった画素は画面を書き換えないため、画面が不透明な
    /// 位置を透過にすることはできない。
    pub(crate) fn expressible(&self, frame: &[u8]) -> bool {
        // 先頭フレームを迎える前の論理画面はどこも描かれておらず、透過を持たない
        // 入力では透過にする遷移そのものが起きない
        let Some(pixels) = self.pixels else {
            return true;
        };
        if self.layout.color_type != ColorType::Rgba8 {
            return true;
        }

        let bpp = self.layout.bytes_per_pixel;
        pixels
            .chunks_exact(bpp)
            .zip(frame.chunks_exact(bpp))
            .all(|(screen, pixel)| !is_transparent(pixel) || is_transparent(screen))
    }

    /// `rect` の添字列を `out` へ追記する
    ///
    /// 透過インデックスを持つテーブルでは、矩形の中で画面と一致する画素を
    /// それへ置き換える。その画素は画面を書き換えないまま、LZWにとって
    /// 同じ値の長いランになる。潰すのが写すより先なので、テーブルに載って
    /// いない色を持ち越した未変更画素もそのまま潰れる。
    ///
    /// `frame` は [`Canvas::render`] が写した描画後の色。
    pub(crate) fn append_indices(
        &self,
        frame: &[u8],
        rect: Rect,
        palette: &mut Palette,
        out: &mut Vec<u8>,
    ) {
        let stride = self.layout.stride;
        let bpp = self.layout.bytes_per_pixel;
        let transparent = palette.transparent();

        out.reserve(rect.area() as usize);
        for y in 0..rect.height as usize {
            let row = (rect.y as usize + y) * stride + rect.x as usize * bpp;
            for x in 0..rect.width as usize {
                let at = row + x * bpp;
                let pixel = &frame[at..at + bpp];
                let index = match (transparent, self.pixels) {
                    (Some(transparent), Some(screen)) if screen[at..at + bpp] == *pixel => {
                        transparent
                    }
                    _ => palette.index_of(pixel, bpp),
                };
                out.push(index);
            }
        }
    }
}

/// 描画後の色の面
///
/// 添字ではなく色で持つ。キャンバスの色が常に現在のカラーテーブルに載って
/// いるとは限らないため、添字の面ではキャンバスを表現できない。
///
/// 保留中のフレームを挟んで2面を持つ。[`Self::before`] はそのフレームを描く
/// 直前の画面、[`Self::after`] は描いた後の画面で、廃棄方法はこの2面から
/// 投入されたフレームが載る画面を作る。
pub(crate) struct Canvas {
    layout: Layout,
    /// 保留中のフレームを描く直前の画面
    ///
    /// 透過を持てない RGB8 の入力では廃棄方法が Do Not Dispose に固定され、
    /// 戻す先が要らないため空のまま。
    before: Vec<u8>,
    /// 保留中のフレームを描いた後の画面
    after: Vec<u8>,
    /// 先頭フレームを描いたか
    drawn: bool,
}

impl Canvas {
    /// 先頭フレームを迎える前のキャンバス
    ///
    /// GIFの論理画面は最初どこも描かれておらず、透過インデックスに当たった
    /// 画素と同じ扱いになる。
    pub(crate) fn new(layout: Layout) -> Self {
        let before = match layout.color_type {
            ColorType::Rgba8 => vec![0; layout.frame_len],
            ColorType::Rgb8 => Vec::new(),
        };
        Canvas {
            after: vec![0; layout.frame_len],
            before,
            layout,
            drawn: false,
        }
    }

    /// 廃棄方法を適用した後、投入されたフレームが載る画面
    pub(crate) fn screen(&self) -> Screen<'_> {
        Screen {
            layout: &self.layout,
            pixels: self.drawn.then_some(self.after.as_slice()),
        }
    }

    /// 投入されたフレームを描画後の色の面へ写して `out` へ入れる
    ///
    /// 入力が前フレームと変わった画素だけ現在のパレットで写し、変わっていない
    /// 画素は前の描画後の色を持ち越す。持ち越した画素は画面上の色をそのまま
    /// 保つため、パレットが変わっても静止した領域は揺れない。
    ///
    /// `previous` は直前に投入されたフレームの正規化した入力。先頭フレームでは空。
    pub(crate) fn render(
        &self,
        previous: &[u8],
        frame: &[u8],
        palette: &mut Palette,
        out: &mut Vec<u8>,
    ) {
        let bpp = self.layout.bytes_per_pixel;
        out.clear();
        out.reserve(self.layout.frame_len);

        // 先頭フレームには前が無く、持ち越せる色も無い
        let carried = self.drawn.then_some((previous, self.after.as_slice()));
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
            let index = palette.index_of(pixel, bpp);
            out.extend_from_slice(&palette.color_at(index).to_le_bytes()[..bpp]);
        }
    }

    /// 先頭フレームを描く
    ///
    /// 矩形が論理画面全体なので、描いた後の画面は写したフレームそのものになる。
    pub(crate) fn start(&mut self, frame: &[u8]) {
        self.after.copy_from_slice(frame);
        self.drawn = true;
    }

    /// 保留中のフレームを廃棄し、投入されたフレームで進める
    ///
    /// `disposed` は保留中のフレームの矩形、`rect` は投入されたフレームの矩形。
    /// 2面はそれぞれの矩形の中でしか変わらないため、書き換えるのは中だけで足りる。
    pub(crate) fn advance(&mut self, disposed: Rect, frame: &[u8], rect: Rect) {
        // 描いた後の面を潰す前に、戻す先を廃棄後の画面へ進める
        if !self.before.is_empty() {
            copy_rect(&mut self.before, &self.after, disposed, &self.layout);
        }
        copy_rect(&mut self.after, frame, rect, &self.layout);
    }
}

/// `rect` の中だけを `from` から `to` へ写す
fn copy_rect(to: &mut [u8], from: &[u8], rect: Rect, layout: &Layout) {
    let stride = layout.stride;
    let row_len = rect.width as usize * layout.bytes_per_pixel;
    let head = rect.y as usize * stride + rect.x as usize * layout.bytes_per_pixel;
    for y in 0..rect.height as usize {
        let at = head + y * stride;
        to[at..at + row_len].copy_from_slice(&from[at..at + row_len]);
    }
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

    /// 先頭フレームを描き、その矩形を返す
    fn start(canvas: &mut Canvas, frame: &[u8]) -> Rect {
        let rect = canvas.screen().rect_of(frame);
        canvas.start(frame);
        rect
    }

    /// 保留中のフレームをそのまま残して `frame` を描き、その矩形を返す
    fn draw(canvas: &mut Canvas, pending: Rect, frame: &[u8]) -> Rect {
        let rect = canvas.screen().rect_of(frame);
        canvas.advance(pending, frame, rect);
        rect
    }

    #[test]
    fn the_first_frame_covers_the_logical_screen() {
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        assert_eq!(
            canvas.screen().rect_of(&opaque(0x10)),
            layout(ColorType::Rgba8).whole()
        );
    }

    /// 差分の無いフレームは1画素の矩形になる
    #[test]
    fn an_identical_frame_becomes_a_unit_rect() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        start(&mut canvas, &opaque(0x10));
        assert_eq!(canvas.screen().rect_of(&opaque(0x10)), UNCHANGED);
    }

    /// キャンバスは矩形の中だけを書き換え、外は投入されたフレームと一致したまま
    #[test]
    fn advancing_leaves_the_canvas_equal_to_the_frame() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let first = start(&mut canvas, &opaque(0x10));

        let mut next = opaque(0x10);
        next[4 * 4] = 0x7F;
        draw(&mut canvas, first, &next);
        assert_eq!(canvas.after, next);
    }

    /// 戻す先の面は、保留中のフレームを描く直前の画面を持つ
    #[test]
    fn the_earlier_plane_holds_the_screen_before_the_pending_frame() {
        let first = opaque(0x10);
        let mut second = opaque(0x10);
        second[4] = 0x7F;
        let mut third = second.clone();
        third[8] = 0x7E;

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        assert_eq!(canvas.before, vec![0; first.len()], "論理画面が空でない");

        let rect = start(&mut canvas, &first);
        let rect = draw(&mut canvas, rect, &second);
        assert_eq!(canvas.before, first);

        draw(&mut canvas, rect, &third);
        assert_eq!(canvas.before, second);
        assert_eq!(canvas.after, third);
    }

    /// 透過を持てない入力では、戻す先の面を持たない
    #[test]
    fn an_input_without_alpha_keeps_no_earlier_plane() {
        let canvas = Canvas::new(layout(ColorType::Rgb8));
        assert!(canvas.before.is_empty());
    }

    /// 不透明な画素を透過へ変える遷移は、キャンバスを残す画面では表現できない
    #[test]
    fn an_opaque_pixel_turning_transparent_is_not_expressible() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        start(&mut canvas, &opaque(0x10));

        let mut next = opaque(0x10);
        next[16..20].fill(0);
        assert!(!canvas.screen().expressible(&next));
    }

    /// 透過のまま留まる画素と、透過から不透明になる画素は表現できる
    #[test]
    fn transitions_that_only_add_paint_keep_the_canvas() {
        let mut first = opaque(0x10);
        first[16..24].fill(0);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        start(&mut canvas, &first);

        let mut next = first.clone();
        next[16..20].copy_from_slice(&[1, 2, 3, 0xFF]);
        assert!(canvas.screen().expressible(&next));
    }

    /// RGB8の入力には透過が存在しないため、遷移の検査は要らない
    #[test]
    fn an_input_without_alpha_is_always_expressible() {
        let frame: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|i| [i as u8, 0, 0]).collect();
        let mut canvas = Canvas::new(layout(ColorType::Rgb8));
        start(&mut canvas, &frame);

        let zeros = vec![0u8; frame.len()];
        assert!(canvas.screen().expressible(&zeros));
    }

    /// 矩形の中で画面と一致する画素は透過インデックスになる
    #[test]
    fn unchanged_pixels_inside_the_rect_become_the_transparent_index() {
        let first = opaque(0x10);
        let mut next = opaque(0x10);
        next[0] = 0x7F;
        next[(WIDTH * HEIGHT - 1) as usize * 4] = 0x7E;

        let mut palette = palette_of(&[&first, &next], 4);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        start(&mut canvas, &first);
        let rect = canvas.screen().rect_of(&next);
        assert_eq!(rect, layout(ColorType::Rgba8).whole(), "矩形が全画面でない");

        let mut indices = Vec::new();
        canvas
            .screen()
            .append_indices(&next, rect, &mut palette, &mut indices);
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
        let mut palette = palette_of(&[&frame], 4);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = canvas.screen().rect_of(&frame);
        let mut indices = Vec::new();
        canvas
            .screen()
            .append_indices(&frame, rect, &mut palette, &mut indices);

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
        let mut palette = palette_of(&[&full], 3);
        assert_eq!(palette.transparent(), None);

        let mut canvas = Canvas::new(layout(ColorType::Rgb8));
        start(&mut canvas, &frame);

        let mut indices = Vec::new();
        canvas
            .screen()
            .append_indices(&frame, UNCHANGED, &mut palette, &mut indices);
        assert_eq!(indices, [0]);
    }

    /// 素材自身の透過画素は透過インデックスへ写る
    #[test]
    fn a_transparent_pixel_maps_to_the_transparent_index() {
        let mut first = opaque(0x10);
        first[..4].fill(0);

        let mut palette = palette_of(&[&first], 4);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = canvas.screen().rect_of(&first);
        let mut indices = Vec::new();
        canvas
            .screen()
            .append_indices(&first, rect, &mut palette, &mut indices);

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

        let mut palette = palette_of(&[&first, &second], 4);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(&[], &first, &mut palette, &mut rendered);
        assert_eq!(rendered, first, "先頭フレームが写っていない");
        start(&mut canvas, &rendered);

        canvas.render(&first, &second, &mut palette, &mut rendered);
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

        let mut palette = palette_of(&[&first], 4);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(&[], &first, &mut palette, &mut rendered);
        start(&mut canvas, &rendered);

        canvas.render(&first, &second, &mut palette, &mut rendered);
        assert_eq!(rendered[..4], [0, 0, 0, 0]);
        assert!(!canvas.screen().expressible(&rendered));
    }

    /// 全幅でない矩形は行をまたいで切り出される
    #[test]
    fn a_partial_width_rect_is_cropped_row_by_row() {
        let frame = opaque(0x10);
        let mut palette = palette_of(&[&frame], 4);

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut indices = Vec::new();
        canvas
            .screen()
            .append_indices(&frame, rect, &mut palette, &mut indices);

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
