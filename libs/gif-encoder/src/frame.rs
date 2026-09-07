//! 差分矩形・廃棄方法・透過ランの決定

use crate::block::{DISPOSAL_DO_NOT_DISPOSE, DISPOSAL_RESTORE_TO_BACKGROUND};
use crate::layout::Layout;
use crate::normalize::TRANSPARENT;
use crate::table::{Fit, Palette};
use anim_core::{ColorType, Rect, dirty_rect, unchanged_run};

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

    /// `rect` の添字列を `out` へ追記し、書いた色を `frame` へ戻す
    ///
    /// 透過インデックスを持つテーブルでは、矩形の中で画面と一致する画素を
    /// それへ置き換える。その画素は画面を書き換えないまま、LZWにとって
    /// 同じ値の長いランになる。潰すのが写すより先なので、テーブルに載って
    /// いない色を持ち越した未変更画素もそのまま潰れる。
    ///
    /// 潰れずに写した画素は、テーブルに完全一致が無ければ最近傍か埋め草へずれる。
    /// 書いた添字が指す色を `frame` へ戻すので、これを [`Canvas::advance`] へ
    /// 渡せばキャンバスはデコーダが見る色をそのまま持つ。
    ///
    /// `frame` は [`Canvas::render`] が写した描画後の色。
    pub(crate) fn append_indices(
        &self,
        frame: &mut [u8],
        rect: Rect,
        palette: &mut Palette,
        out: &mut Vec<u8>,
    ) -> Written {
        match self.layout.color_type {
            ColorType::Rgb8 => self.append_indices_bpp::<3>(frame, rect, palette, out),
            ColorType::Rgba8 => self.append_indices_bpp::<4>(frame, rect, palette, out),
        }
    }

    fn append_indices_bpp<const BPP: usize>(
        &self,
        frame: &mut [u8],
        rect: Rect,
        palette: &mut Palette,
        out: &mut Vec<u8>,
    ) -> Written {
        let stride = self.layout.stride;
        let mut approximated = 0;
        let mut substituted = 0;

        let base = out.len();
        out.resize(base + rect.area() as usize, 0);
        // 画面と一致する画素を潰せるのは、透過添字と描かれた画面が揃うときだけ
        let collapsible = palette.transparent().zip(self.pixels);

        let mut wrote = base;
        for y in 0..rect.height as usize {
            let row = (rect.y as usize + y) * stride + rect.x as usize * BPP;
            let end = row + rect.width as usize * BPP;
            let mut at = row;
            while at < end {
                if let Some((transparent, screen)) = collapsible {
                    let stop = unchanged_run::<BPP>(&screen[..end], &frame[..end], at);
                    if stop > at {
                        let run = (stop - at) / BPP;
                        out[wrote..wrote + run].fill(transparent);
                        palette.mark_used(transparent);
                        wrote += run;
                        at = stop;
                        if at == end {
                            break;
                        }
                    }
                }
                let pixel: [u8; BPP] = frame[at..at + BPP].try_into().expect("1画素");
                let mapped = palette.map(&pixel, BPP);
                let counter = match mapped.fit {
                    Fit::Exact => None,
                    Fit::Approximated { .. } => Some(&mut approximated),
                    Fit::Substituted { .. } => Some(&mut substituted),
                };
                if let Some(counter) = counter {
                    *counter += 1;
                    let color = palette.color_at(mapped.index).to_le_bytes();
                    frame[at..at + BPP].copy_from_slice(&color[..BPP]);
                }
                palette.mark_used(mapped.index);
                out[wrote] = mapped.index;
                wrote += 1;
                at += BPP;
            }
        }
        Written {
            approximated,
            substituted,
        }
    }
}

/// 添字を書くときにテーブルへ写した画素の集計
pub(crate) struct Written {
    /// 完全一致が無く最近傍へ写した画素数
    pub(crate) approximated: u64,
    /// 写す先が無く埋め草へ置いた画素数
    pub(crate) substituted: u64,
}

/// 入力が変わった画素を写した結果の集計
///
/// 持ち越した画素と透過標識はどちらにも数えない。パレットを通らないため。
pub(crate) struct Rendered {
    /// 完全一致が無く最近傍へ写した画素数
    pub(crate) approximated: u64,
    /// 写す先が無く埋め草へ置いた画素数
    pub(crate) substituted: u64,
    /// テーブルへ写した画素数
    pub(crate) mapped: u64,
    /// 写した画素とその写し先の二乗距離の総和
    pub(crate) error: u64,
    /// 写し先との二乗距離が外れの閾値を超えた画素数
    pub(crate) strayed: u64,
}

impl Rendered {
    /// 写した画素1つあたりの二乗距離の平均が `floor` を超えるか
    ///
    /// 1画素も写していないフレームは超えない。
    pub(crate) fn mean_error_exceeds(&self, floor: u64) -> bool {
        self.error > floor * self.mapped
    }

    /// 外れた画素が、写した画素の `permille` 千分率を超えるか
    ///
    /// 1画素も写していないフレームは超えない。
    pub(crate) fn strayed_ratio_exceeds(&self, permille: u64) -> bool {
        self.strayed * 1000 > permille * self.mapped
    }
}

/// 保留中のフレームを廃棄した後の画面
///
/// 「不透明 → 透過」の遷移を含むフレームだけが使う。どちらの画面もキャンバスから
/// 画素を抜き、抜いた画素は投入されたフレームが書き直すことになる。
///
/// [`Self::previous`] が抜く画素は [`Self::background`] でも抜けるため、
/// 表現できるかどうかは後者だけで決まる。
pub(crate) struct Disposed<'a> {
    /// 矩形を透過へ抜いた画面
    background: Screen<'a>,
    /// 保留中のフレームを描く直前へ戻した画面
    previous: Screen<'a>,
}

impl<'a> Disposed<'a> {
    /// 矩形を透過へ抜いた画面
    pub(crate) fn background(&self) -> Screen<'a> {
        self.background
    }

    /// 保留中のフレームを描く直前へ戻した画面
    pub(crate) fn previous(&self) -> Screen<'a> {
        self.previous
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
    /// 保留中のフレームを描く直前の画面。透過を持てない入力では空
    before: Vec<u8>,
    /// 保留中のフレームを描いた後の画面
    after: Vec<u8>,
    /// 保留中のフレームの矩形を透過へ抜いた画面
    ///
    /// 抜く矩形はフレームごとに変わるため、[`AlphaCanvas::dispose`] が組み立て直す。
    cleared: Vec<u8>,
    /// 直前に投入されたフレームの正規化した入力。先頭フレームを描く前は `None`
    ///
    /// 描いた後の画面と対になっていて、この2つが揃って初めて未変更画素の色を
    /// 持ち越せる。画面を進める [`Canvas::start`] と [`Canvas::advance`] が
    /// 同時に更新する。
    previous: Option<Vec<u8>>,
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
            cleared: Vec::new(),
            layout,
            previous: None,
        }
    }

    /// 保留中のフレームをそのまま残した画面
    pub(crate) fn kept(&self) -> Screen<'_> {
        self.screen(&self.after)
    }

    /// 直前に投入されたフレームの正規化した入力
    pub(crate) fn previous(&self) -> Option<&[u8]> {
        self.previous.as_deref()
    }

    /// 透過を読み書きできる面として借りる
    ///
    /// 透過標識を持てるのはαの欄がある入力だけで、標識を抜いたり戻したりする
    /// 操作はそのときにしか成り立たない。
    pub(crate) fn alpha(&mut self) -> Option<AlphaCanvas<'_>> {
        match self.layout.color_type {
            ColorType::Rgba8 => Some(AlphaCanvas { canvas: self }),
            ColorType::Rgb8 => None,
        }
    }

    /// 論理画面と同じ大きさの画素列を画面として見る
    fn screen<'a>(&'a self, pixels: &'a [u8]) -> Screen<'a> {
        Screen {
            layout: &self.layout,
            pixels: self.previous.is_some().then_some(pixels),
        }
    }

    /// 投入されたフレームを描画後の色の面へ写して `out` へ入れる
    ///
    /// 入力が前フレームと変わった画素だけ現在のパレットで写し、変わっていない
    /// 画素は前の描画後の色を持ち越す。持ち越した画素は画面上の色をそのまま
    /// 保つため、パレットが変わっても静止した領域は揺れない。
    ///
    /// `stray_floor` は外れた画素と呼ぶ二乗距離で、写し先がこれより遠い画素を
    /// 別に数える。
    pub(crate) fn render(
        &self,
        frame: &[u8],
        palette: &mut Palette,
        stray_floor: u64,
        out: &mut Vec<u8>,
    ) -> Rendered {
        match self.layout.color_type {
            ColorType::Rgb8 => self.render_bpp::<3>(frame, palette, stray_floor, out),
            ColorType::Rgba8 => self.render_bpp::<4>(frame, palette, stray_floor, out),
        }
    }

    fn render_bpp<const BPP: usize>(
        &self,
        frame: &[u8],
        palette: &mut Palette,
        stray_floor: u64,
        out: &mut Vec<u8>,
    ) -> Rendered {
        let len = self.layout.frame_len;
        // 全画素を書き直すので、長さが揃っていれば中身は問わない
        if out.len() != len {
            out.clear();
            out.resize(len, 0);
        }
        let dst = &mut out[..len];
        let frame = &frame[..len];

        let mut rendered = Rendered {
            approximated: 0,
            substituted: 0,
            mapped: 0,
            error: 0,
            strayed: 0,
        };
        // 先頭フレームには前が無く、持ち越せる色も無い
        let carried = self
            .previous
            .as_ref()
            .map(|previous| (previous.as_slice(), self.after.as_slice()));
        let mut at = 0;
        while at + BPP <= len {
            if let Some((previous, drawn)) = carried {
                let run = unchanged_run::<BPP>(&previous[..len], frame, at);
                if run > at {
                    dst[at..run].copy_from_slice(&drawn[at..run]);
                    at = run;
                    if at + BPP > len {
                        break;
                    }
                }
            }
            let pixel: &[u8; BPP] = frame[at..at + BPP].try_into().expect("1画素");
            // 透過標識は色として写さない。2値透過に中間が無いため、標識のまま
            // 残して廃棄方法の判定へ渡す
            if BPP == 4 && is_transparent(pixel) {
                dst[at..at + BPP].copy_from_slice(pixel);
                at += BPP;
                continue;
            }
            let mapped = palette.map(pixel, BPP);
            rendered.mapped += 1;
            let error = match mapped.fit {
                Fit::Exact => 0,
                Fit::Approximated { error } => {
                    rendered.approximated += 1;
                    u64::from(error)
                }
                Fit::Substituted { error } => {
                    rendered.substituted += 1;
                    u64::from(error)
                }
            };
            rendered.error += error;
            if error > stray_floor {
                rendered.strayed += 1;
            }
            dst[at..at + BPP].copy_from_slice(&palette.color_at(mapped.index).to_le_bytes()[..BPP]);
            at += BPP;
        }
        rendered
    }

    /// 先頭フレームを描く
    ///
    /// 矩形が論理画面全体なので、描いた後の画面は写したフレームそのものになる。
    ///
    /// `rendered` は投入されたフレームを写した描画後の色、`frame` はその
    /// 正規化した入力。
    pub(crate) fn start(&mut self, rendered: &[u8], frame: &[u8]) {
        self.after.copy_from_slice(rendered);
        self.keep(frame);
    }

    /// 保留中のフレームを `disposal` で廃棄し、投入されたフレームで進める
    ///
    /// `disposed` は保留中のフレームの矩形、`rect` は投入されたフレームの矩形。
    /// 2面が変わるのはこの2つの矩形の中だけなので、書き換えるのも中だけで足りる。
    ///
    /// `rendered` は投入されたフレームを写した描画後の色、`frame` はその
    /// 正規化した入力。
    pub(crate) fn advance(
        &mut self,
        disposal: u8,
        disposed: Rect,
        rendered: &[u8],
        rect: Rect,
        frame: &[u8],
    ) {
        // 描いた後の面を潰す前に、戻す先を廃棄後の画面へ進める
        if !self.before.is_empty() {
            match disposal {
                DISPOSAL_DO_NOT_DISPOSE => {
                    copy_rect(&mut self.before, &self.after, disposed, &self.layout)
                }
                DISPOSAL_RESTORE_TO_BACKGROUND => {
                    fill_rect(&mut self.before, disposed, &self.layout)
                }
                // 戻す先そのものが廃棄後の画面になる
                _ => {}
            }
        }
        if disposal != DISPOSAL_DO_NOT_DISPOSE {
            copy_rect(&mut self.after, rendered, disposed, &self.layout);
        }
        copy_rect(&mut self.after, rendered, rect, &self.layout);
        self.keep(frame);
    }

    /// 投入されたフレームの正規化した入力を、次のフレームの前として覚える
    fn keep(&mut self, frame: &[u8]) {
        let previous = self.previous.get_or_insert_with(Vec::new);
        previous.clear();
        previous.extend_from_slice(frame);
    }
}

/// 透過を読み書きできるキャンバス
///
/// [`Canvas::alpha`] からだけ取れる。透過標識を抜いた画面も、標識を書き戻す先も、
/// αの欄がある入力でしか作れない。
pub(crate) struct AlphaCanvas<'a> {
    canvas: &'a mut Canvas,
}

impl AlphaCanvas<'_> {
    /// 保留中のフレームをそのまま残した画面
    pub(crate) fn kept(&self) -> Screen<'_> {
        self.canvas.kept()
    }

    /// 保留中のフレームを、描く直前の画面とその上に描いた色の組で借りる
    ///
    /// そのフレームを符号化し直す経路が使う。書いた色を描いた後の面へ戻せる。
    pub(crate) fn pending_frame(&mut self) -> (Screen<'_>, &mut [u8]) {
        let Canvas {
            layout,
            before,
            after,
            previous,
            ..
        } = &mut *self.canvas;
        (
            Screen {
                layout,
                pixels: previous.is_some().then_some(before.as_slice()),
            },
            after,
        )
    }

    /// `frame` が透過にしたい画素をすべて含むまで `rect` を広げる
    ///
    /// 矩形を丸ごと抜く候補は、この矩形の中しか抜けない。返した矩形が `rect` と
    /// 同じなら、抜きたい画素はすべて中にあってその候補で表現できる。
    pub(crate) fn widen(&self, rect: Rect, frame: &[u8]) -> Rect {
        let canvas = &*self.canvas;
        let bpp = canvas.layout.bytes_per_pixel;
        let width = usize::from(canvas.layout.width);
        let (mut left, mut right) = (usize::MAX, 0usize);
        let (mut top, mut bottom) = (usize::MAX, 0usize);

        let pixels = canvas.after.chunks_exact(bpp).zip(frame.chunks_exact(bpp));
        for (at, (screen, pixel)) in pixels.enumerate() {
            if !is_transparent(pixel) || is_transparent(screen) {
                continue;
            }
            left = left.min(at % width);
            right = right.max(at % width);
            top = top.min(at / width);
            bottom = bottom.max(at / width);
        }
        if left > right {
            return rect;
        }

        let clearing = Rect {
            x: left as u32,
            y: top as u32,
            width: (right - left + 1) as u32,
            height: (bottom - top + 1) as u32,
        };
        union(rect, clearing)
    }

    /// 保留中のフレームをキャンバスから廃棄した画面を組み立てる
    ///
    /// `rect` は保留中のフレームの矩形。
    pub(crate) fn dispose(&mut self, rect: Rect) -> Disposed<'_> {
        let canvas = &mut *self.canvas;
        canvas.cleared.clear();
        canvas.cleared.extend_from_slice(&canvas.after);
        fill_rect(&mut canvas.cleared, rect, &canvas.layout);

        Disposed {
            background: canvas.screen(&canvas.cleared),
            previous: canvas.screen(&canvas.before),
        }
    }
}

/// 2つの矩形をどちらも含む最小の矩形
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}

/// `rect` の中だけを `from` から `to` へ写す
fn copy_rect(to: &mut [u8], from: &[u8], rect: Rect, layout: &Layout) {
    for_each_row(rect, layout, |at, row_len| {
        to[at..at + row_len].copy_from_slice(&from[at..at + row_len])
    });
}

/// `rect` の中だけを完全透過の標識で埋める
fn fill_rect(plane: &mut [u8], rect: Rect, layout: &Layout) {
    for_each_row(rect, layout, |at, row_len| {
        plane[at..at + row_len].fill(0);
    });
}

/// `rect` が覆う各行の先頭とバイト数を渡す
fn for_each_row(rect: Rect, layout: &Layout, mut row: impl FnMut(usize, usize)) {
    let stride = layout.stride;
    let row_len = rect.width as usize * layout.bytes_per_pixel;
    let head = rect.y as usize * stride + rect.x as usize * layout.bytes_per_pixel;
    for y in 0..rect.height as usize {
        row(head + y * stride, row_len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::DISPOSAL_RESTORE_TO_PREVIOUS;

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

    /// フレーム列の色を見つけた順に受け入れて閉じたテーブル
    fn palette_of(frames: &[&[u8]], color_type: ColorType) -> Palette {
        let mut palette = Palette::new();
        let mut previous: Option<&[u8]> = None;
        for frame in frames {
            assert!(
                palette.admit(color_type, previous, frame),
                "色が上限に収まらない"
            );
            previous = Some(frame);
        }
        palette.settle(&[]);
        palette
    }

    /// 先頭フレームを描き、その矩形を返す
    fn start(canvas: &mut Canvas, frame: &[u8]) -> Rect {
        let rect = canvas.kept().rect_of(frame);
        canvas.start(frame, frame);
        rect
    }

    /// 透過を読み書きできる面として借りる
    fn alpha(canvas: &mut Canvas) -> AlphaCanvas<'_> {
        canvas.alpha().expect("αを持つ入力")
    }

    /// 保留中のフレームをそのまま残して `frame` を描き、その矩形を返す
    fn draw(canvas: &mut Canvas, pending: Rect, frame: &[u8]) -> Rect {
        let rect = canvas.kept().rect_of(frame);
        canvas.advance(DISPOSAL_DO_NOT_DISPOSE, pending, frame, rect, frame);
        rect
    }

    #[test]
    fn the_first_frame_covers_the_logical_screen() {
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        assert_eq!(
            canvas.kept().rect_of(&opaque(0x10)),
            layout(ColorType::Rgba8).whole()
        );
    }

    /// 2枚目以降は、変わった画素をすべて含む矩形になる
    #[test]
    fn later_frames_are_cropped_against_the_screen() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let first = opaque(0x10);
        start(&mut canvas, &first);

        let mut next = first.clone();
        next[(WIDTH as usize + 2) * 4] = 0xFF;
        assert_eq!(
            canvas.kept().rect_of(&next),
            Rect {
                x: 2,
                y: 1,
                width: 1,
                height: 1
            }
        );
    }

    /// 差分の無いフレームは1画素の矩形になる
    #[test]
    fn an_identical_frame_becomes_a_unit_rect() {
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        start(&mut canvas, &opaque(0x10));
        assert_eq!(canvas.kept().rect_of(&opaque(0x10)), UNCHANGED);
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

    /// 矩形を透過へ抜く廃棄では、戻す先もその矩形を抜いた画面になる
    ///
    /// 戻す先が抜いた画素を持ったままだと、以降のフレームで描く直前へ戻す候補が
    /// 抜けない画素を抜けると読む。
    #[test]
    fn a_background_disposal_clears_the_earlier_plane() {
        let first = opaque(0x10);
        let mut second = first.clone();
        second[4] = 0x7F;
        let third = opaque(0x20);

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = start(&mut canvas, &first);
        let rect = draw(&mut canvas, rect, &second);
        assert_eq!(
            rect,
            Rect {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            }
        );
        assert_eq!(canvas.before, first, "描く直前の画面が違う");

        let cleared = alpha(&mut canvas)
            .dispose(rect)
            .background()
            .rect_of(&third);
        canvas.advance(
            DISPOSAL_RESTORE_TO_BACKGROUND,
            rect,
            &third,
            cleared,
            &third,
        );

        let mut expected = first.clone();
        expected[4..8].fill(0);
        assert_eq!(canvas.before, expected, "抜いた画素が戻す先に残っている");
        assert_eq!(canvas.after, third);
    }

    /// 描く直前へ戻す廃棄では、戻す先はそのまま残る
    #[test]
    fn a_previous_disposal_keeps_the_earlier_plane() {
        let first = opaque(0x10);
        let mut second = first.clone();
        second[4] = 0x7F;

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = start(&mut canvas, &first);
        let rect = draw(&mut canvas, rect, &second);
        assert_eq!(canvas.before, first, "描く直前の画面が違う");

        let restored = alpha(&mut canvas).dispose(rect).previous().rect_of(&first);
        assert_eq!(restored, UNCHANGED, "戻した画面が投入されたフレームと違う");
        canvas.advance(DISPOSAL_RESTORE_TO_PREVIOUS, rect, &first, restored, &first);

        assert_eq!(canvas.before, first, "戻す先が書き換わっている");
        assert_eq!(canvas.after, first);
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
        assert!(!canvas.kept().expressible(&next));
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
        assert!(canvas.kept().expressible(&next));
    }

    /// RGB8の入力には透過が存在しないため、遷移の検査は要らない
    #[test]
    fn an_input_without_alpha_is_always_expressible() {
        let frame: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|i| [i as u8, 0, 0]).collect();
        let mut canvas = Canvas::new(layout(ColorType::Rgb8));
        start(&mut canvas, &frame);

        let zeros = vec![0u8; frame.len()];
        assert!(canvas.kept().expressible(&zeros));
    }

    /// 矩形の中で画面と一致する画素は透過インデックスになる
    #[test]
    fn unchanged_pixels_inside_the_rect_become_the_transparent_index() {
        let first = opaque(0x10);
        let mut next = opaque(0x10);
        next[0] = 0x7F;
        next[(WIDTH * HEIGHT - 1) as usize * 4] = 0x7E;

        let mut palette = palette_of(&[&first, &next], ColorType::Rgba8);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        start(&mut canvas, &first);
        let rect = canvas.kept().rect_of(&next);
        assert_eq!(rect, layout(ColorType::Rgba8).whole(), "矩形が全画面でない");

        let mut indices = Vec::new();
        canvas
            .kept()
            .append_indices(&mut next, rect, &mut palette, &mut indices);
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
        let mut frame = opaque(0x10);
        let mut palette = palette_of(&[&frame], ColorType::Rgba8);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = canvas.kept().rect_of(&frame);
        let mut indices = Vec::new();
        canvas
            .kept()
            .append_indices(&mut frame, rect, &mut palette, &mut indices);

        assert!(
            indices.iter().all(|&index| index != transparent),
            "先頭フレームの画素が潰れている"
        );
    }

    /// 透過インデックスを持たないテーブルでは、未変更画素もそのまま写る
    #[test]
    fn a_table_without_a_transparent_index_keeps_every_pixel() {
        let mut frame: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|i| [i as u8, 0, 0]).collect();
        let full: Vec<u8> = (0..256)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let mut palette = palette_of(&[&full], ColorType::Rgb8);
        assert_eq!(palette.transparent(), None);

        let mut canvas = Canvas::new(layout(ColorType::Rgb8));
        start(&mut canvas, &frame);

        let mut indices = Vec::new();
        canvas
            .kept()
            .append_indices(&mut frame, UNCHANGED, &mut palette, &mut indices);
        assert_eq!(indices, [0]);
    }

    /// 素材自身の透過画素は透過インデックスへ写る
    #[test]
    fn a_transparent_pixel_maps_to_the_transparent_index() {
        let mut first = opaque(0x10);
        first[..4].fill(0);

        let mut palette = palette_of(&[&first], ColorType::Rgba8);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = canvas.kept().rect_of(&first);
        let mut indices = Vec::new();
        canvas
            .kept()
            .append_indices(&mut first, rect, &mut palette, &mut indices);

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

        let mut palette = palette_of(&[&first, &second], ColorType::Rgba8);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(&first, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(rendered, first, "先頭フレームが写っていない");
        canvas.start(&rendered, &first);

        canvas.render(&second, &mut palette, STRAY_FLOOR, &mut rendered);
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

        let mut palette = palette_of(&[&first], ColorType::Rgba8);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(&first, &mut palette, STRAY_FLOOR, &mut rendered);
        canvas.start(&rendered, &first);

        canvas.render(&second, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(rendered[..4], [0, 0, 0, 0]);
        assert!(!canvas.kept().expressible(&rendered));
    }

    /// 書いた添字が指す色が、そのままキャンバスへ入る
    ///
    /// テーブルから落ちた色を持ち越した画素は、廃棄方法が抜いた矩形の中で
    /// 書き直されて最近傍へずれる。持ち越した色のまま進めると、キャンバスが
    /// 持つ色とデコーダが見る色が離れる。
    #[test]
    fn the_canvas_takes_the_color_that_was_written() {
        /// 先に据えたテーブルにだけある色
        const CARRIED: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];
        /// 逃げた色表が持つ唯一の非透過色
        const SETTLED: [u8; 4] = [0x10, 0x20, 0x30, 0xFF];

        let first: Vec<u8> = CARRIED.repeat((WIDTH * HEIGHT) as usize);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(
            &first,
            &mut palette_of(&[&first], ColorType::Rgba8),
            STRAY_FLOOR,
            &mut rendered,
        );
        canvas.start(&rendered, &first);

        // 入力が変わらない画素は、色表が入れ替わっても持ち越される
        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        canvas.render(&first, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(rendered, first, "持ち越しがテーブルを通っている");

        // 矩形を透過へ抜いた画面では、持ち越した画素も書き直す
        let rect = layout(ColorType::Rgba8).whole();
        let mut indices = Vec::new();
        alpha(&mut canvas)
            .dispose(rect)
            .background()
            .append_indices(&mut rendered, rect, &mut palette, &mut indices);

        let written = palette.color_at(indices[0]).to_le_bytes();
        assert_ne!(written, CARRIED, "持ち越した色がテーブルに残っている");
        assert_eq!(rendered[..4], written, "書いた色がキャンバスへ渡っていない");

        canvas.advance(
            DISPOSAL_RESTORE_TO_BACKGROUND,
            rect,
            &rendered,
            rect,
            &first,
        );
        assert_eq!(canvas.after[..4], written, "キャンバスが書いた色を持たない");
    }

    /// 添字を書いた集計は、テーブルへ入れずに返り値で渡す
    ///
    /// 廃棄方法は候補を複数符号化して1つだけ採るため、符号化した時点で数えると
    /// 採らなかった候補の画素まで載る。
    #[test]
    fn writing_indices_hands_the_counts_back_instead_of_noting_them() {
        /// 先に据えたテーブルにだけある色
        const CARRIED: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];

        let first: Vec<u8> = CARRIED.repeat((WIDTH * HEIGHT) as usize);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();
        canvas.render(
            &first,
            &mut palette_of(&[&first], ColorType::Rgba8),
            STRAY_FLOOR,
            &mut rendered,
        );
        canvas.start(&rendered, &first);

        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        canvas.render(&first, &mut palette, STRAY_FLOOR, &mut rendered);

        let rect = layout(ColorType::Rgba8).whole();
        let mut indices = Vec::new();
        let written = alpha(&mut canvas)
            .dispose(rect)
            .background()
            .append_indices(&mut rendered, rect, &mut palette, &mut indices);

        assert_eq!(
            written.approximated,
            u64::from(WIDTH * HEIGHT),
            "書き直した画素を数えていない"
        );
        assert_eq!(
            palette.approximated(),
            0,
            "符号化した時点でテーブルが数えている"
        );
    }

    /// テーブルが持つ唯一の非透過色
    const SETTLED: [u8; 4] = [0x10, 0x20, 0x30, 0xFF];
    /// 緑だけ 20 離れた色。写した先との二乗距離は 400
    const OFF_BY: [u8; 4] = [0x10, 0x34, 0x30, 0xFF];
    /// [`OFF_BY`] を [`SETTLED`] へ写した二乗距離
    const OFF_BY_ERROR: u64 = 20 * 20;
    /// 写した結果を数えるときの外れの閾値
    ///
    /// [`OFF_BY`] を写した二乗距離ちょうどに置く。
    const STRAY_FLOOR: u64 = OFF_BY_ERROR;
    /// [`OFF_BY`] から赤へさらに 1 離れた色
    ///
    /// 写した先との二乗距離は [`STRAY_FLOOR`] をちょうど1超える。
    const BEYOND: [u8; 4] = [0x11, 0x34, 0x30, 0xFF];

    /// 平均がちょうど床の誤差は、床を超えたものに数えない
    ///
    /// この平均は逃げるかどうかを決める。境目を含めると、ちょうどの誤差で
    /// 逃げへ倒れる。
    #[test]
    fn a_mean_error_equal_to_the_floor_does_not_exceed_it() {
        let pixels = u64::from(WIDTH * HEIGHT);
        let frame: Vec<u8> = OFF_BY.repeat(pixels as usize);
        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();

        let counted = canvas.render(&frame, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(counted.approximated, pixels, "最近傍へ写していない");
        assert_eq!(counted.mapped, pixels);
        assert_eq!(counted.error, OFF_BY_ERROR * pixels);

        assert!(
            !counted.mean_error_exceeds(OFF_BY_ERROR),
            "床ちょうどの平均を超えたものに数えている"
        );
        assert!(
            counted.mean_error_exceeds(OFF_BY_ERROR - 1),
            "床を超えた平均を数えていない"
        );
    }

    /// 持ち越した画素と透過標識は平均の分母に入らない
    ///
    /// 分母に入れると、静止した領域の広いフレームほど平均が薄まる。
    #[test]
    fn carried_pixels_and_markers_stay_out_of_the_mean() {
        let pixels = (WIDTH * HEIGHT) as usize;
        let previous: Vec<u8> = SETTLED.repeat(pixels);
        let mut frame = previous.clone();
        frame[..4].copy_from_slice(&OFF_BY);
        frame[4..8].copy_from_slice(&[0, 0, 0, 0]);

        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        canvas.start(&previous, &previous);
        let mut rendered = Vec::new();

        let counted = canvas.render(&frame, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(counted.mapped, 1, "写していない画素を分母に入れている");
        assert_eq!(counted.error, OFF_BY_ERROR);
        assert!(counted.mean_error_exceeds(OFF_BY_ERROR - 1));
    }

    /// [`SETTLED`] を埋め草の黒へ置いた二乗距離
    const SUBSTITUTED_ERROR: u64 = 0x10 * 0x10 + 0x20 * 0x20 + 0x30 * 0x30;

    /// 写す先が無い画素は、埋め草までの隔たりを平均へ持ち込む
    ///
    /// 分母だけ増やして誤差を0で足すと、最も遠い画素が平均を下げる。
    #[test]
    fn substituted_pixels_carry_their_distance_into_the_mean() {
        let pixels = (WIDTH * HEIGHT) as usize;
        let transparent = vec![0u8; pixels * 4];
        let frame: Vec<u8> = SETTLED.repeat(pixels);

        let mut palette = palette_of(&[&transparent], ColorType::Rgba8);
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();

        let counted = canvas.render(&frame, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(counted.substituted, pixels as u64, "埋め草へ置いていない");
        assert_eq!(counted.approximated, 0, "近似に数えている");
        assert_eq!(counted.error, SUBSTITUTED_ERROR * pixels as u64);
        assert!(
            counted.mean_error_exceeds(SUBSTITUTED_ERROR - 1),
            "埋め草へ置いた画素が平均を薄めている"
        );
    }

    /// 1画素も写していないフレームは、どの床も超えない
    #[test]
    fn a_frame_that_maps_nothing_never_exceeds_the_floor() {
        let pixels = (WIDTH * HEIGHT) as usize;
        let previous: Vec<u8> = SETTLED.repeat(pixels);
        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        let mut canvas = Canvas::new(layout(ColorType::Rgba8));
        canvas.start(&previous, &previous);
        let mut rendered = Vec::new();

        let counted = canvas.render(&previous, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(counted.mapped, 0);
        assert!(!counted.mean_error_exceeds(0));
        assert!(!counted.strayed_ratio_exceeds(0));
    }

    /// 閾値ちょうどの二乗距離は外れた画素に数えない
    ///
    /// 外れた画素の割合も逃げるかどうかを決める。境目を含めると、ちょうどの
    /// 誤差で逃げへ倒れる。
    #[test]
    fn an_error_equal_to_the_stray_floor_is_not_a_stray() {
        let pixels = u64::from(WIDTH * HEIGHT);
        let frame: Vec<u8> = OFF_BY.repeat(pixels as usize);
        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();

        let counted = canvas.render(&frame, &mut palette, OFF_BY_ERROR, &mut rendered);
        assert_eq!(counted.mapped, pixels);
        assert_eq!(counted.strayed, 0, "閾値ちょうどを外れに数えている");

        let counted = canvas.render(&frame, &mut palette, OFF_BY_ERROR - 1, &mut rendered);
        assert_eq!(counted.strayed, pixels, "閾値を超えた画素を数えていない");

        let exact: Vec<u8> = SETTLED.repeat(pixels as usize);
        let counted = canvas.render(&exact, &mut palette, 0, &mut rendered);
        assert_eq!(counted.mapped, pixels);
        assert_eq!(counted.strayed, 0, "完全一致した画素を外れに数えている");
    }

    /// 外れた画素がちょうど千分率のフレームは、割合を超えたものに数えない
    #[test]
    fn a_stray_ratio_equal_to_the_permille_does_not_exceed_it() {
        let pixels = (WIDTH * HEIGHT) as usize;
        let mut frame: Vec<u8> = SETTLED.repeat(pixels);
        frame[..4].copy_from_slice(&BEYOND);

        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();

        let counted = canvas.render(&frame, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(counted.mapped, pixels as u64);
        assert_eq!(counted.strayed, 1, "閾値を1超えた画素を数えていない");

        let permille = 1000 / pixels as u64;
        assert!(
            !counted.strayed_ratio_exceeds(permille),
            "ちょうどの割合を超えたものに数えている"
        );
        assert!(
            counted.strayed_ratio_exceeds(permille - 1),
            "割合を超えたフレームを数えていない"
        );
    }

    /// 写した画素がすべて外れても、千分率が 1000 なら割合を超えない
    #[test]
    fn a_permille_of_one_thousand_is_never_exceeded() {
        let pixels = (WIDTH * HEIGHT) as usize;
        let frame: Vec<u8> = BEYOND.repeat(pixels);

        let mut palette = palette_of(&[&SETTLED[..]], ColorType::Rgba8);
        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let mut rendered = Vec::new();

        let counted = canvas.render(&frame, &mut palette, STRAY_FLOOR, &mut rendered);
        assert_eq!(counted.strayed, pixels as u64, "全画素が外れていない");
        assert!(!counted.strayed_ratio_exceeds(1000));
    }

    /// 全幅でない矩形は行をまたいで切り出される
    #[test]
    fn a_partial_width_rect_is_cropped_row_by_row() {
        let mut frame = opaque(0x10);
        let mut palette = palette_of(&[&frame], ColorType::Rgba8);

        let canvas = Canvas::new(layout(ColorType::Rgba8));
        let rect = Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut indices = Vec::new();
        canvas
            .kept()
            .append_indices(&mut frame, rect, &mut palette, &mut indices);

        let expected: Vec<u8> = [1u8, 2, 5, 6]
            .iter()
            .map(|&at| {
                palette
                    .global_bytes()
                    .chunks_exact(3)
                    .position(|color| color == [at, 0x20, 0x10])
                    .unwrap() as u8
            })
            .collect();
        assert_eq!(indices, expected);
    }
}
