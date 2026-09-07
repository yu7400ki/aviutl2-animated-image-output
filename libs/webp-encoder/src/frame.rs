//! キャンバスの追跡と、フレームの載せ方の決定

use crate::Config;
use crate::normalize::normalize;
use anim_core::{ColorType, Layout, Rect, Rewrite, dirty_rect};

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

/// 据えた入力の載せ方と、保留中のフレームの廃棄方法
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    /// キャンバス上の矩形。`x` と `y` は偶数
    pub(crate) rect: Rect,
    /// 透過画素を下のキャンバスへ重ねるか
    pub(crate) blend: bool,
    /// 保留中のフレームを表示した後にその矩形を抜くか
    pub(crate) dispose: bool,
}

/// キャンバスが持つ面
///
/// どれも `stride` バイトの行が隙間なく並ぶRGBA。
struct Sheets<'a> {
    /// 正規化して据えた入力
    staged: &'a mut Vec<u8>,
    /// 前のフレームを描いた画素
    drawn: &'a mut Vec<u8>,
    /// 前のフレームの矩形を抜いた画素
    disposed: &'a mut Vec<u8>,
    /// 1行のバイト数
    stride: usize,
}

impl Sheets<'_> {
    /// 据えた入力を重ねる先の面
    fn base(&self, dispose: bool) -> &[u8] {
        if dispose { self.disposed } else { self.drawn }
    }

    /// 据えた入力と `base` の差分矩形。オフセットは偶数へ寄る
    fn exact(&self, base: &[u8]) -> Option<Rect> {
        dirty_rect(base, self.staged, self.stride, PIXEL).map(snap_to_even)
    }

    /// `rect` の中の据えた入力を、重ねる先の面へ重ねられるか
    ///
    /// 完全不透明な画素はそのまま置き換わる。残る画素は `mixes` が分ける。
    fn blendable(&self, rect: Rect, dispose: bool, mixes: impl Fn(&[u8], &[u8]) -> bool) -> bool {
        let base = self.base(dispose);
        let row_len = rect.width as usize * PIXEL;
        let head = rect.y as usize * self.stride + rect.x as usize * PIXEL;
        (0..rect.height as usize).all(|y| {
            let at = head + y * self.stride;
            self.staged[at..at + row_len]
                .chunks_exact(PIXEL)
                .zip(base[at..at + row_len].chunks_exact(PIXEL))
                .all(|(staged, base)| staged[3] == OPAQUE || mixes(staged, base))
        })
    }

    /// 2つの候補の狭い方で載せ方を組む
    ///
    /// `kept` は矩形を抜かない仮定の候補、`cleared` は抜いた仮定のもの。`mixes` は
    /// 完全不透明でない画素を重ねられるかを分ける。
    fn placement(
        &self,
        kept: Rect,
        cleared: Option<Rect>,
        mixes: impl Fn(&[u8], &[u8]) -> bool,
    ) -> Placement {
        let (rect, dispose) = narrower(kept, cleared);
        Placement {
            rect,
            blend: self.blendable(rect, dispose, mixes),
            dispose,
        }
    }

    /// 据えた入力を描いた面と、そこから `rect` を抜いた面へ進める
    fn draw(&mut self, rect: Rect) {
        std::mem::swap(self.drawn, self.staged);
        self.dispose(rect);
    }

    /// `rect` を抜いた面を、`drawn` から作り直す
    fn dispose(&mut self, rect: Rect) {
        self.disposed.clear();
        self.disposed.extend_from_slice(self.drawn);

        let row_len = rect.width as usize * PIXEL;
        let head = rect.y as usize * self.stride + rect.x as usize * PIXEL;
        for y in 0..rect.height as usize {
            let at = head + y * self.stride;
            self.disposed[at..at + row_len].fill(0);
        }
    }
}

/// 差分矩形を決めるとき比べる相手
///
/// どちらも据えた入力どうしを比べる。直前のフレームの画素を残す土台だけが、
/// そのフレームが書いた画素をもう1回ぶん引き継ぐ。
enum Basis {
    /// 厳密に一致しない画素を書き直す
    Inputs,
    /// 直前の投入で変わった画素も書き直す
    Rewritten(Rewrite),
}

impl Basis {
    /// 据えた入力の載せ方を決め、その矩形を書いた後の状態へ進める
    ///
    /// `disposable` は、矩形を抜く廃棄方法を載せられるフレームが保留されて
    /// いること。
    fn place(&mut self, sheets: &Sheets<'_>, disposable: bool) -> Option<Placement> {
        match self {
            Basis::Inputs => {
                let kept = sheets.exact(sheets.drawn)?;
                let cleared =
                    disposable.then(|| sheets.exact(sheets.disposed).unwrap_or(SINGLE_PIXEL));
                Some(sheets.placement(kept, cleared, |staged, base| staged == base))
            }
            Basis::Rewritten(rewrite) => {
                let change = rewrite.changes(sheets.staged, sheets.drawn);
                let kept = rewrite.carried(&change).bounds().map(snap_to_even);
                let cleared = disposable.then(|| {
                    rewrite
                        .changes(sheets.staged, sheets.disposed)
                        .bounds()
                        .map_or(SINGLE_PIXEL, snap_to_even)
                });
                rewrite.advance(change);
                Some(sheets.placement(kept?, cleared, |_, base| base[3] == 0))
            }
        }
    }
}

/// 書く面積の小さい候補
///
/// `kept` は矩形を抜かない仮定のもの、`cleared` は抜いた仮定のもの。面積が
/// 並んだときは抜かない仮定を採る。
fn narrower(kept: Rect, cleared: Option<Rect>) -> (Rect, bool) {
    match cleared {
        Some(cleared) if cleared.area() < kept.area() => (cleared, true),
        _ => (kept, false),
    }
}

/// 前のフレームまでを描いたキャンバス
///
/// 画素はRGBAで持つ。廃棄方法が2値なので、前のフレームを描いた後の画素と、
/// そこから前のフレームの矩形を抜いた画素の2面を追う。
pub(crate) struct Canvas {
    /// 前のフレームを描いた画素。まだ何も描いていなければ空
    drawn: Vec<u8>,
    /// [`Self::drawn`] から前のフレームの矩形を抜いた画素
    disposed: Vec<u8>,
    /// 正規化して据えた入力
    staged: Vec<u8>,
    /// キャンバス全体を覆う矩形
    whole: Rect,
    /// 1行のバイト数
    stride: usize,
    basis: Basis,
}

impl Canvas {
    /// `layout` の大きさの、まだ何も描いていないキャンバスを作る
    ///
    /// 可逆は厳密な一致で、非可逆は直前の投入で変わった画素も含めて矩形を採る。
    pub(crate) fn new(layout: &Layout, config: &Config) -> Self {
        let stride = layout.width as usize * PIXEL;
        let basis = if config.lossless {
            Basis::Inputs
        } else {
            Basis::Rewritten(Rewrite::new(layout.width, layout.height, ColorType::Rgba8))
        };
        Canvas {
            drawn: Vec::new(),
            disposed: Vec::new(),
            staged: Vec::new(),
            whole: layout.whole(),
            stride,
            basis,
        }
    }

    /// RGBAの入力を正規化して据える
    ///
    /// `data` は画素が [`Layout`] のとおりに並んでいること。
    pub(crate) fn stage(&mut self, data: Vec<u8>) {
        self.staged = data;
        normalize(&mut self.staged);
    }

    /// RGBの入力をRGBAへ広げて据える
    ///
    /// `data` は画素が [`Layout`] のとおりに並んでいること。
    pub(crate) fn expand(&mut self, data: &[u8]) {
        self.staged.clear();
        self.staged.reserve(data.len() / 3 * PIXEL);
        for pixel in data.chunks_exact(3) {
            self.staged
                .extend_from_slice(&[pixel[0], pixel[1], pixel[2], OPAQUE]);
        }
    }

    /// 据えた入力 (RGBA)
    pub(crate) fn staged(&self) -> &[u8] {
        &self.staged
    }

    /// 据えた入力を重ねる先のキャンバス
    ///
    /// `dispose` は保留中のフレームの矩形を抜くかどうか。
    pub(crate) fn base(&self, dispose: bool) -> &[u8] {
        if dispose { &self.disposed } else { &self.drawn }
    }

    /// 据えた入力の載せ方を決める
    ///
    /// `disposable` は、矩形を抜く廃棄方法を載せられるフレームが保留されて
    /// いること。抜いた側の矩形が狭ければそちらを採る。書き直す画素が1つも
    /// 無ければ `None`。
    pub(crate) fn place(&mut self, disposable: bool) -> Option<Placement> {
        if self.drawn.is_empty() {
            return Some(Placement {
                rect: self.whole,
                blend: false,
                dispose: false,
            });
        }

        let (basis, sheets) = self.split();
        basis.place(&sheets, disposable)
    }

    /// 据えたフレームをキャンバスへ映す
    ///
    /// `placement` は [`Self::place`] が返した載せ方。
    pub(crate) fn commit(&mut self, placement: Placement) {
        let (_, mut sheets) = self.split();
        sheets.draw(placement.rect);
    }

    /// 直前の投入で変わった画素も書き直すか
    #[cfg(test)]
    pub(crate) fn carries_the_previous_change(&self) -> bool {
        matches!(self.basis, Basis::Rewritten(_))
    }

    /// 比べる相手と、それが読み書きする面を分ける
    fn split(&mut self) -> (&mut Basis, Sheets<'_>) {
        let Canvas {
            drawn,
            disposed,
            staged,
            stride,
            basis,
            ..
        } = self;
        (
            basis,
            Sheets {
                staged,
                drawn,
                disposed,
                stride: *stride,
            },
        )
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

    /// 透過の面に、αが `alpha` の四角を1つ置いたRGBA
    fn sprite(width: u32, height: u32, at: (u32, u32), size: u32, alpha: u8) -> Vec<u8> {
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    if x.wrapping_sub(at.0) < size && y.wrapping_sub(at.1) < size {
                        [0x20, 0x40, 0x60, alpha]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
            })
            .collect()
    }

    /// `at` から `size` 角を `color` で塗り替えたRGBA
    fn repainted(base: &[u8], width: u32, at: (u32, u32), size: u32, color: [u8; 4]) -> Vec<u8> {
        let mut data = base.to_vec();
        for y in at.1..at.1 + size {
            for x in at.0..at.0 + size {
                let index = (y * width + x) as usize * PIXEL;
                data[index..index + PIXEL].copy_from_slice(&color);
            }
        }
        data
    }

    /// 比べる相手を決める設定
    fn settings(color_type: ColorType, lossless: bool) -> Config {
        Config {
            color_type,
            lossless,
            quality: 75.0,
            method: 4,
            num_plays: 0,
        }
    }

    /// 入力どうしを比べる設定
    fn lossless(color_type: ColorType) -> Config {
        settings(color_type, true)
    }

    /// RGBAのフレームを1つ据えたキャンバス
    fn canvas_with(layout: &Layout, data: &[u8]) -> Canvas {
        let mut canvas = Canvas::new(layout, &lossless(layout.color_type));
        canvas.stage(data.to_vec());
        let placement = canvas.place(false).expect("先頭フレームは全面を持つ");
        canvas.commit(placement);
        canvas
    }

    /// 直前の投入で変わった画素を持ち越すのは非可逆だけ
    #[test]
    fn only_a_lossy_canvas_carries_the_previous_change() {
        for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
            for lossless in [true, false] {
                let layout = layout(color_type);
                let canvas = Canvas::new(&layout, &settings(color_type, lossless));
                assert_eq!(
                    canvas.carries_the_previous_change(),
                    !lossless,
                    "{color_type:?} 可逆{lossless}"
                );
            }
        }
    }

    /// 半透明の平らな面の対角へ、`tint` で塗った不透明な画素を置いたRGBA
    fn translucent(width: u32, height: u32, tint: u8) -> Vec<u8> {
        let corners = [(0, 0), (width - 1, height - 1)];
        (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    if corners.contains(&(x, y)) {
                        [tint, 0x22, 0x33, OPAQUE]
                    } else {
                        [0x40, 0x80, 0xC0, 0x80]
                    }
                })
            })
            .collect()
    }

    /// 半透明の未変更画素を重ねられるのは、透過置換を持つ可逆だけ
    ///
    /// 非可逆は置換を持たないので、キャンバスと一致していることが重ねてよい
    /// 理由にならない。
    #[test]
    fn a_translucent_unchanged_pixel_is_blended_only_where_it_is_substituted() {
        let layout = Layout::new(8, 8, ColorType::Rgba8).unwrap();
        let first = translucent(layout.width, layout.height, 0x11);
        let second = translucent(layout.width, layout.height, 0x99);

        for lossless in [true, false] {
            let mut canvas = Canvas::new(&layout, &settings(layout.color_type, lossless));
            canvas.stage(first.clone());
            let placement = canvas.place(false).expect("先頭フレームは全面を持つ");
            assert_eq!(placement.rect, layout.whole(), "可逆{lossless}");
            canvas.drawn = canvas.staged.clone();
            canvas.disposed = vec![0; canvas.staged.len()];

            canvas.stage(second.clone());
            let placement = canvas.place(false).expect("対角の色が変わっている");
            assert_eq!(placement.rect, layout.whole(), "可逆{lossless}");
            assert_eq!(placement.blend, lossless, "可逆{lossless}");
        }
    }

    /// 決定的な擬似乱数で埋めた不透明なRGBA
    fn noise(width: u32, height: u32, seed: u32) -> Vec<u8> {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        (0..width * height)
            .flat_map(|_| {
                let mut byte = || {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    (state >> 16) as u8
                };
                [byte(), byte(), byte(), OPAQUE]
            })
            .collect()
    }

    /// `rect` を透過にする
    fn cleared(data: &[u8], rect: Rect, stride: usize) -> Vec<u8> {
        let mut data = data.to_vec();
        let row_len = rect.width as usize * PIXEL;
        let head = rect.y as usize * stride + rect.x as usize * PIXEL;
        for y in 0..rect.height as usize {
            let at = head + y * stride;
            data[at..at + row_len].fill(0);
        }
        data
    }

    /// 抜いた仮定の候補は、直前の投入で変わった画素で膨らまない
    ///
    /// 透過の面を四角が動くと、抜いた跡は完全透過になるので直前の位置を書き直す
    /// 理由が消える。持ち越しをこの候補にも乗せると、跡まで覆う矩形と並んで
    /// 抜く廃棄方法が選ばれなくなる。
    #[test]
    fn the_cleared_candidate_is_not_widened_by_the_previous_change() {
        let layout = Layout::new(24, 20, ColorType::Rgba8).unwrap();
        let mut canvas = Canvas::new(&layout, &settings(layout.color_type, false));

        // 透過の面を不透明な四角が重なりながら動く。奇数の縦位置が矩形を1行上へ
        // 広げ、四角の外の完全透過な画素を巻き込む
        let frames =
            [(2, 2), (6, 5), (10, 8)].map(|at| sprite(layout.width, layout.height, at, 8, OPAQUE));
        let expected = [
            Placement {
                rect: layout.whole(),
                blend: false,
                dispose: false,
            },
            Placement {
                rect: Rect {
                    x: 6,
                    y: 4,
                    width: 8,
                    height: 9,
                },
                blend: true,
                dispose: true,
            },
            Placement {
                rect: Rect {
                    x: 10,
                    y: 8,
                    width: 8,
                    height: 8,
                },
                blend: true,
                dispose: true,
            },
        ];

        for (index, frame) in frames.iter().enumerate() {
            canvas.stage(frame.clone());
            let placement = canvas.place(index > 0).expect("四角が動いている");
            assert_eq!(placement, expected[index], "フレーム{index}");
            canvas.commit(placement);
        }
    }

    /// 直前の投入で変わった画素は、抜かない仮定の候補を広げる
    ///
    /// 不透明な面では抜いた跡も書き直しに掛かるので、四角の旧位置は矩形に残る。
    #[test]
    fn the_previous_change_widens_the_kept_candidate() {
        let layout = Layout::new(24, 20, ColorType::Rgba8).unwrap();
        let mut canvas = Canvas::new(&layout, &settings(layout.color_type, false));
        let base = noise(layout.width, layout.height, 0x5EED);

        let frames = [(2, 2), (6, 6), (10, 10)]
            .map(|at| repainted(&base, layout.width, at, 4, [9, 9, 9, OPAQUE]));

        canvas.stage(frames[0].clone());
        let placement = canvas.place(false).expect("先頭フレームは全面を持つ");
        canvas.commit(placement);

        // 2枚目は旧位置と新位置を覆う
        canvas.stage(frames[1].clone());
        let placement = canvas.place(true).expect("四角が動いている");
        assert_eq!(
            placement.rect,
            Rect {
                x: 2,
                y: 2,
                width: 8,
                height: 8
            }
        );
        canvas.commit(placement);

        // 3枚目は2枚目が書いた範囲まで戻る
        canvas.stage(frames[2].clone());
        let placement = canvas.place(true).expect("四角が動いている");
        assert_eq!(
            placement.rect,
            Rect {
                x: 2,
                y: 2,
                width: 12,
                height: 12
            }
        );
    }

    /// 抜いた面は、描いた面から据えた矩形を抜いたものになる
    #[test]
    fn the_disposed_face_follows_the_drawn_one() {
        let layout = Layout::new(24, 20, ColorType::Rgba8).unwrap();

        let frames: Vec<Vec<u8>> = (0..4)
            .map(|seed| {
                let mut frame = noise(layout.width, layout.height, 0x5EED);
                let at = (seed * 3 + 2) as usize;
                for y in at..at + 6 {
                    let head = (y * layout.width as usize + at) * PIXEL;
                    frame[head..head + 6 * PIXEL].fill(0x33);
                }
                frame
            })
            .collect();

        for lossless in [true, false] {
            let mut canvas = Canvas::new(&layout, &settings(layout.color_type, lossless));
            for (index, frame) in frames.iter().enumerate() {
                canvas.stage(frame.clone());
                let placement = canvas.place(index > 0).expect("フレームごとに画素が変わる");
                canvas.commit(placement);

                assert_eq!(
                    canvas.base(true),
                    cleared(canvas.base(false), placement.rect, layout.stride),
                    "可逆{lossless} のフレーム{index} {placement:?}"
                );
            }
        }
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
        let mut canvas = Canvas::new(&layout, &lossless(layout.color_type));
        canvas.stage(vec![0; layout.frame_len]);

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

        canvas.stage(data.clone());
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

        canvas.stage(second.clone());
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
        canvas.stage(changed.clone());

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
        let mut canvas = Canvas::new(&layout, &lossless(layout.color_type));
        canvas.expand(&data);

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
        canvas.stage(second.clone());
        let placement = canvas.place(false).expect("画素が1つ変わっている");
        canvas.commit(placement);

        canvas.stage(second.clone());
        assert_eq!(canvas.place(false), None);
        canvas.stage(first.clone());
        assert!(canvas.place(false).is_some());
    }

    /// 離れた場所へ動く不透明な四角は、前の場所を抜いた方が矩形が狭くなる
    #[test]
    fn clearing_the_previous_rect_wins_when_it_shrinks_the_next_one() {
        let layout = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        let mut canvas = canvas_with(&layout, &sprite(16, 16, (0, 0), 4, OPAQUE));

        canvas.stage(sprite(16, 16, (10, 10), 4, OPAQUE));
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
        let mut canvas = canvas_with(&layout, &sprite(16, 16, (0, 0), 4, OPAQUE));

        canvas.stage(sprite(16, 16, (10, 10), 4, OPAQUE));
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
        let mut canvas = Canvas::new(&layout, &lossless(layout.color_type));
        canvas.stage(sprite(16, 16, (0, 0), 16, OPAQUE));
        let placement = canvas.place(false).expect("先頭フレームは全面を持つ");
        canvas.commit(placement);
        canvas.stage(sprite(16, 16, (4, 4), 4, OPAQUE));
        let placement = canvas.place(false).expect("四角が縮んでいる");
        canvas.commit(placement);

        canvas.stage(vec![0; layout.frame_len]);
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
        canvas.stage(punched.clone());

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
        canvas.stage(repainted.clone());

        let placement = canvas.place(true).expect("画素が1つ変わっている");
        assert!(placement.blend);
    }
    /// 抜いた側と抜かない側が同じ広さなら、抜かない方を採る
    ///
    /// 同点は出力の大きさを変えず、どちらの候補も合成としては正しいので、
    /// 突き合わせでは倒し方を問えない。
    #[test]
    fn a_tie_between_the_two_rects_keeps_the_canvas() {
        let layout = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        let first = ramp(layout.width, layout.height);
        let mut canvas = canvas_with(&layout, &first);

        let second = repainted(&first, layout.width, (4, 4), 4, [0x11, 0x22, 0x33, OPAQUE]);
        canvas.stage(second.clone());
        let placement = canvas.place(false).expect("四角を塗り替えている");
        assert_eq!(
            placement.rect,
            Rect {
                x: 4,
                y: 4,
                width: 4,
                height: 4
            }
        );
        canvas.commit(placement);

        // 抜いた跡 (4,4,4,4) を含む (4,4,6,6) を透過にすると、抜いた側と
        // 抜かない側の差分がどちらも (4,4,6,6) になる
        let third = repainted(&second, layout.width, (4, 4), 6, [0, 0, 0, 0]);
        canvas.stage(third.clone());
        assert_eq!(
            canvas.place(true),
            Some(Placement {
                rect: Rect {
                    x: 4,
                    y: 4,
                    width: 6,
                    height: 6
                },
                blend: false,
                dispose: false,
            })
        );
    }

    /// 重ねられるかは、そのフレームが実際に重なる面で判定する
    ///
    /// 矩形を抜く廃棄方法を採ったフレームが重なるのは抜いた後の面で、抜く前の
    /// 面ではない。どちらで判定しても合成そのものは正しいままなので、
    /// 突き合わせでは取り違えを問えない。
    #[test]
    fn blending_is_judged_against_the_face_the_frame_lands_on() {
        let layout = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        let first = ramp(layout.width, layout.height);
        let mut canvas = canvas_with(&layout, &first);

        let second = repainted(&first, layout.width, (2, 2), 12, [0x11, 0x22, 0x33, OPAQUE]);
        canvas.stage(second.clone());
        let placement = canvas.place(false).expect("四角を塗り替えている");
        canvas.commit(placement);

        // 抜いた跡と同じ範囲を透過にし、その中の1画素だけを塗る。抜いた後の面
        // とは1画素しか違わないが、抜く前の面とは範囲全体が違う
        let mut third = repainted(&second, layout.width, (2, 2), 12, [0, 0, 0, 0]);
        let at = (7 * layout.width as usize + 7) * PIXEL;
        third[at..at + PIXEL].copy_from_slice(&[0x44, 0x55, 0x66, OPAQUE]);
        canvas.stage(third.clone());

        assert_eq!(
            canvas.place(true),
            Some(Placement {
                rect: Rect {
                    x: 6,
                    y: 6,
                    width: 2,
                    height: 2
                },
                blend: true,
                dispose: true,
            })
        );
    }
}
