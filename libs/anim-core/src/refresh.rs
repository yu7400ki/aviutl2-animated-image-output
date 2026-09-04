//! 画面に出ている値と入力の隔たりから決まる、書き直す画素

use crate::color::ColorType;
use crate::diff::Rect;

/// 1つの語が持つビット数
const WORD_BITS: usize = u64::BITS as usize;

/// 画面に出ている値が入力から離れてよい量
pub const TOLERANCE: u8 = 0;

/// 端を含む整数の範囲
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub min: u32,
    pub max: u32,
}

impl Span {
    /// 含む値の数
    pub fn count(self) -> u32 {
        self.max - self.min + 1
    }
}

/// `value` を含むまで広げた範囲
fn widened(span: Option<Span>, value: u32) -> Span {
    match span {
        Some(span) => Span {
            min: span.min.min(value),
            max: span.max.max(value),
        },
        None => Span {
            min: value,
            max: value,
        },
    }
}

/// 矩形の中で書き直す画素が、各行と各列で占める範囲
///
/// 座標は矩形の左上からの相対。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// 行ごとの、書き直す画素が占めるxの範囲。矩形の高さぶん並ぶ
    pub rows: Vec<Option<Span>>,
    /// 列ごとの、書き直す画素が占めるyの範囲。矩形の幅ぶん並ぶ
    pub cols: Vec<Option<Span>>,
}

/// 1画素1ビットの面
///
/// 行は語の境界から始まる。幅と高さの内側だけがビットを持ち、外は倒れている。
#[derive(Debug, Clone)]
struct Plane {
    words: Vec<u64>,
    words_per_row: usize,
    width: u32,
    height: u32,
}

impl Plane {
    /// `width` x `height` の、すべて倒れた面
    fn new(width: u32, height: u32) -> Self {
        let words_per_row = (width as usize).div_ceil(WORD_BITS);
        Plane {
            words: vec![0; words_per_row * height as usize],
            words_per_row,
            width,
            height,
        }
    }

    fn get(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let (word, bit) = self.at(x, y);
        self.words[word] & bit != 0
    }

    /// 面の内側の `(x, y)` を `value` にする
    fn set(&mut self, x: u32, y: u32, value: bool) {
        debug_assert!(x < self.width && y < self.height);
        let (word, bit) = self.at(x, y);
        if value {
            self.words[word] |= bit;
        } else {
            self.words[word] &= !bit;
        }
    }

    /// すべてのビットを倒す
    fn clear(&mut self) {
        self.words.fill(0);
    }

    /// `(x, y)` が乗る語の位置と、その中のビット
    fn at(&self, x: u32, y: u32) -> (usize, u64) {
        let x = x as usize;
        (
            y as usize * self.words_per_row + x / WORD_BITS,
            1 << (x % WORD_BITS),
        )
    }

    /// `rect` が面に収まるか
    fn holds(&self, rect: Rect) -> bool {
        u64::from(rect.x) + u64::from(rect.width) <= u64::from(self.width)
            && u64::from(rect.y) + u64::from(rect.height) <= u64::from(self.height)
    }

    /// `rect` の中で立っているビットが、各行と各列で占める範囲
    fn profile(&self, rect: Rect) -> Profile {
        let mut rows = vec![None; rect.height as usize];
        let mut cols = vec![None; rect.width as usize];
        let left = rect.x as usize;
        let right = left + rect.width as usize;
        let words = left / WORD_BITS..right.div_ceil(WORD_BITS);

        for (row, span) in rows.iter_mut().enumerate() {
            let base = (rect.y as usize + row) * self.words_per_row;
            for index in words.clone() {
                let mut word = self.words[base + index];
                if index == words.start {
                    word &= u64::MAX << (left % WORD_BITS);
                }
                let tail = right - (words.end - 1) * WORD_BITS;
                if index + 1 == words.end && tail < WORD_BITS {
                    word &= (1 << tail) - 1;
                }
                while word != 0 {
                    let x = (index * WORD_BITS + word.trailing_zeros() as usize - left) as u32;
                    *span = Some(widened(*span, x));
                    cols[x as usize] = Some(widened(cols[x as usize], row as u32));
                    word &= word - 1;
                }
            }
        }

        Profile { rows, cols }
    }
}

/// 書き直す画素の地図
///
/// 座標はキャンバスの左上を原点とする画素の位置。
#[derive(Debug, Clone)]
pub struct Triggers {
    map: Plane,
    bounds: Option<Rect>,
}

impl Triggers {
    /// 書き直す画素をすべて含む最小の矩形
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// `rect` の中で書き直す画素が、各行と各列で占める範囲
    ///
    /// # Panics
    /// `rect` が地図からはみ出すとき。
    pub fn profile(&self, rect: Rect) -> Profile {
        assert!(
            self.map.holds(rect),
            "矩形が地図からはみ出している: {rect:?}"
        );
        self.map.profile(rect)
    }

    /// `width` x `height` の、書き直す画素がひとつも無い地図
    fn empty(width: u32, height: u32) -> Self {
        Triggers {
            map: Plane::new(width, height),
            bounds: None,
        }
    }

    /// `(x, y)` を書き直すか
    fn contains(&self, x: u32, y: u32) -> bool {
        self.map.get(x, y)
    }

    /// `(x, y)` を書き直す画素にする
    fn mark(&mut self, x: u32, y: u32) {
        self.map.set(x, y, true);
        self.bounds = Some(match self.bounds {
            Some(bounds) => grown(bounds, x, y),
            None => Rect {
                x,
                y,
                width: 1,
                height: 1,
            },
        });
    }
}

/// `(x, y)` を含むまで広げた矩形
fn grown(rect: Rect, x: u32, y: u32) -> Rect {
    let left = rect.x.min(x);
    let top = rect.y.min(y);
    let right = (rect.x + rect.width - 1).max(x);
    let bottom = (rect.y + rect.height - 1).max(y);
    Rect {
        x: left,
        y: top,
        width: right - left + 1,
        height: bottom - top + 1,
    }
}

/// 2つの画素が離れているか
///
/// RGBは1チャネルでも差が `tolerance` を超えれば離れている。`BPP` が4の画素は
/// αを厳密に比べ、両方が完全透過なら等しい。
fn differs<const BPP: usize>(a: &[u8], b: &[u8], tolerance: u8) -> bool {
    if BPP == 4 {
        if a[3] != b[3] {
            return true;
        }
        if a[3] == 0 {
            return false;
        }
    }
    a[..3]
        .iter()
        .zip(&b[..3])
        .any(|(a, b)| a.abs_diff(*b) > tolerance)
}

/// 入力・画面に出ている値・最後に書いた入力から、書き直す画素を決める
///
/// 画面に出ている値が入力から許容量を超えて離れた画素を書き直す。入力が変わらないまま
/// 離れた画素の書き直しは1回で止まり、入力が変われば次の1回が戻る。
pub struct Refresh {
    /// 画素を最後に書いたときの入力
    written: Vec<u8>,
    /// 最後の書き直しが入力の変化を伴わなかった画素
    retried: Plane,
    width: u32,
    height: u32,
    color_type: ColorType,
}

impl Refresh {
    /// `width` x `height` の `color_type` を追う、まだ何も書いていない状態
    pub fn new(width: u32, height: u32, color_type: ColorType) -> Self {
        let pixels = width as usize * height as usize;
        Refresh {
            written: vec![0; pixels * color_type.bytes_per_pixel()],
            retried: Plane::new(width, height),
            width,
            height,
            color_type,
        }
    }

    /// 書き直す画素の地図
    ///
    /// `src` は投入された入力、`shown` は画面に出ている値で、どちらも `color_type` の
    /// 画素が隙間なく1フレームぶん並んでいること。`tolerance` は画面に出ている値が
    /// 入力から離れてよい量。
    pub fn triggers(&self, src: &[u8], shown: &[u8], tolerance: u8) -> Triggers {
        match self.color_type {
            ColorType::Rgb8 => self.scan::<3>(src, shown, tolerance),
            ColorType::Rgba8 => self.scan::<4>(src, shown, tolerance),
        }
    }

    /// 全面を `src` で書いた後の状態へ進める
    ///
    /// `src` は1フレームぶんの入力。どの画素も `src` を覚え、書き直しの1回を持つ。
    pub fn commit_whole(&mut self, src: &[u8]) {
        self.written.copy_from_slice(src);
        self.retried.clear();
    }

    /// `rect` を `src` で書いた後の状態へ進める
    ///
    /// `src` は [`Self::triggers`] へ渡すのと同じ、1フレームぶんの入力。`triggers` は
    /// `rect` を決めた地図で、矩形を分けて書くなら1枚ごとに呼ぶ。`rect` の中の画素は
    /// すべて `src` を覚え、書き直しの1回は入力が変わっていない引き金だけが使う。
    ///
    /// # Panics
    /// `rect` がキャンバスからはみ出すとき。
    pub fn commit(&mut self, triggers: &Triggers, rect: Rect, src: &[u8]) {
        assert!(
            self.retried.holds(rect),
            "矩形がキャンバスからはみ出している: {rect:?}"
        );
        match self.color_type {
            ColorType::Rgb8 => self.apply::<3>(triggers, rect, src),
            ColorType::Rgba8 => self.apply::<4>(triggers, rect, src),
        }
    }

    fn scan<const BPP: usize>(&self, src: &[u8], shown: &[u8], tolerance: u8) -> Triggers {
        let mut triggers = Triggers::empty(self.width, self.height);
        let stride = self.width as usize * BPP;
        for y in 0..self.height {
            let start = y as usize * stride;
            let end = start + stride;
            let row = src[start..end]
                .chunks_exact(BPP)
                .zip(shown[start..end].chunks_exact(BPP))
                .zip(self.written[start..end].chunks_exact(BPP));
            for (column, ((src, shown), written)) in row.enumerate() {
                let x = column as u32;
                if differs::<BPP>(src, shown, tolerance)
                    && (differs::<BPP>(src, written, 0) || !self.retried.get(x, y))
                {
                    triggers.mark(x, y);
                }
            }
        }
        triggers
    }

    fn apply<const BPP: usize>(&mut self, triggers: &Triggers, rect: Rect, src: &[u8]) {
        let stride = self.width as usize * BPP;
        let row_len = rect.width as usize * BPP;
        let Refresh {
            written, retried, ..
        } = self;
        for row in 0..rect.height {
            let y = rect.y + row;
            let start = y as usize * stride + rect.x as usize * BPP;
            let end = start + row_len;
            let pixels = src[start..end]
                .chunks_exact(BPP)
                .zip(written[start..end].chunks_exact_mut(BPP));
            for (column, (src, written)) in pixels.enumerate() {
                let x = rect.x + column as u32;
                if differs::<BPP>(src, written, 0) {
                    retried.set(x, y, false);
                } else if triggers.contains(x, y) {
                    retried.set(x, y, true);
                }
                written.copy_from_slice(src);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::dirty_rect;

    const COLOR_TYPES: [ColorType; 2] = [ColorType::Rgb8, ColorType::Rgba8];

    /// 隔たりを跨いだ側と跨がない側を並べるための許容量
    const TAU: u8 = 5;

    /// 画素の並びを `color_type` のバイト列へ直す
    fn bytes(color_type: ColorType, pixels: &[[u8; 4]]) -> Vec<u8> {
        let bpp = color_type.bytes_per_pixel();
        let mut out = Vec::with_capacity(pixels.len() * bpp);
        for pixel in pixels {
            out.extend_from_slice(&pixel[..bpp]);
        }
        out
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// `src` を全面で書いた後の状態
    fn started(color_type: ColorType, width: u32, height: u32, src: &[u8]) -> Refresh {
        let mut refresh = Refresh::new(width, height, color_type);
        refresh.commit_whole(src);
        refresh
    }

    fn span(min: u32, max: u32) -> Option<Span> {
        Some(Span { min, max })
    }

    /// 再現できる疑似乱数
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: u32) -> u32 {
            (self.next() % u64::from(bound)) as u32
        }

        fn byte(&mut self) -> u8 {
            self.next() as u8
        }
    }

    /// 疑似乱数で埋めた画素。完全透過の画素はRGBが0に潰れている
    fn noise(random: &mut Random, color_type: ColorType, count: usize) -> Vec<[u8; 4]> {
        (0..count)
            .map(|_| {
                let alpha = match color_type {
                    ColorType::Rgb8 => 0xFF,
                    ColorType::Rgba8 => [0x00, 0x40, 0xFF][random.below(3) as usize],
                };
                match alpha {
                    0 => [0, 0, 0, 0],
                    _ => [random.byte(), random.byte(), random.byte(), alpha],
                }
            })
            .collect()
    }

    /// 許容量ちょうどの隔たりは残り、1つ超えると書き直す
    #[test]
    fn the_tolerance_is_the_last_gap_that_stays() {
        for color_type in COLOR_TYPES {
            let bpp = color_type.bytes_per_pixel();
            let src = bytes(color_type, &[[100, 100, 100, 0xFF]; 3]);
            let refresh = started(color_type, 3, 1, &src);

            for (gap, expected) in [(TAU, None), (TAU + 1, Some(rect(1, 0, 1, 1)))] {
                let mut shown = src.clone();
                shown[bpp] = 100 + gap;
                assert_eq!(
                    refresh.triggers(&src, &shown, TAU).bounds(),
                    expected,
                    "{color_type:?} 隔たり {gap}"
                );
            }
        }
    }

    #[test]
    fn an_unchanged_frame_has_no_rect() {
        for color_type in COLOR_TYPES {
            let src = bytes(color_type, &[[100, 100, 100, 0xFF]; 12]);
            let refresh = started(color_type, 4, 3, &src);
            assert_eq!(
                refresh.triggers(&src, &src, TAU).bounds(),
                None,
                "{color_type:?}"
            );
        }
    }

    /// 離れた2画素の外接矩形は、その間の画素を含む
    #[test]
    fn distant_pixels_span_a_bounding_rect() {
        for color_type in COLOR_TYPES {
            let bpp = color_type.bytes_per_pixel();
            let src = bytes(color_type, &[[100, 100, 100, 0xFF]; 20]);
            let refresh = started(color_type, 5, 4, &src);

            let mut shown = src.clone();
            for index in [5 + 3, 3 * 5 + 1] {
                shown[index * bpp] = 200;
            }
            assert_eq!(
                refresh.triggers(&src, &shown, TAU).bounds(),
                Some(rect(1, 1, 3, 3)),
                "{color_type:?}"
            );
        }
    }

    /// αは許容量に関わらず、1違えば書き直す
    #[test]
    fn a_single_step_of_alpha_is_rewritten() {
        let src = bytes(ColorType::Rgba8, &[[10, 20, 30, 200]; 3]);
        let refresh = started(ColorType::Rgba8, 3, 1, &src);

        let mut shown = src.clone();
        shown[4 + 3] = 199;
        assert_eq!(
            refresh.triggers(&src, &shown, TAU).bounds(),
            Some(rect(1, 0, 1, 1))
        );
    }

    /// 両方が完全透過の画素は、RGBが動いても書き直さない
    #[test]
    fn rgb_under_full_transparency_stays() {
        let src = bytes(ColorType::Rgba8, &[[0, 0, 0, 0]; 3]);
        let refresh = started(ColorType::Rgba8, 3, 1, &src);

        let mut shown = src.clone();
        shown[4..7].copy_from_slice(&[200, 200, 200]);
        assert_eq!(refresh.triggers(&src, &shown, TAU).bounds(), None);
    }

    /// 完全透過の下のRGBは、入力が変わったかの判定にも入らない
    #[test]
    fn full_transparency_hides_rgb_from_the_input_comparison() {
        let src = bytes(ColorType::Rgba8, &[[0, 0, 0, 0]; 3]);
        let mut refresh = started(ColorType::Rgba8, 3, 1, &src);

        // 画面のαだけが食い違い、書き直しの1回を使う
        let mut shown = src.clone();
        shown[4 + 3] = 0xFF;
        let triggers = refresh.triggers(&src, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(1, 0, 1, 1)));
        refresh.commit(&triggers, rect(1, 0, 1, 1), &src);

        // 完全透過のままRGBが動いても、書き直しの1回は戻らない
        let mut moved = src.clone();
        moved[4..7].copy_from_slice(&[200, 200, 200]);
        assert_eq!(refresh.triggers(&moved, &shown, TAU).bounds(), None);
    }

    /// 入力が変わらないまま許容量を超えた画素は、一度だけ書き直す
    #[test]
    fn a_pixel_is_rewritten_once_while_the_input_stays() {
        for color_type in COLOR_TYPES {
            let bpp = color_type.bytes_per_pixel();
            let src = bytes(color_type, &[[100, 100, 100, 0xFF]; 3]);
            let mut refresh = started(color_type, 3, 1, &src);

            let mut shown = src.clone();
            shown[bpp] = 130;
            let triggers = refresh.triggers(&src, &shown, TAU);
            assert_eq!(triggers.bounds(), Some(rect(1, 0, 1, 1)), "{color_type:?}");
            refresh.commit(&triggers, rect(1, 0, 1, 1), &src);

            // 書き直しても画面の値が届かない画素は、そこで止まる
            assert_eq!(
                refresh.triggers(&src, &shown, TAU).bounds(),
                None,
                "{color_type:?}"
            );
        }
    }

    /// 入力が変われば、書き直しの1回が戻る
    #[test]
    fn a_changed_input_restores_the_rewrite() {
        for color_type in COLOR_TYPES {
            let bpp = color_type.bytes_per_pixel();
            let first = bytes(color_type, &[[100, 100, 100, 0xFF]; 3]);
            let mut refresh = started(color_type, 3, 1, &first);

            let mut shown = first.clone();
            shown[bpp] = 130;
            let triggers = refresh.triggers(&first, &shown, TAU);
            refresh.commit(&triggers, rect(1, 0, 1, 1), &first);

            let mut second = first.clone();
            second[bpp] = 150;
            let triggers = refresh.triggers(&second, &shown, TAU);
            assert_eq!(triggers.bounds(), Some(rect(1, 0, 1, 1)), "{color_type:?}");
            refresh.commit(&triggers, rect(1, 0, 1, 1), &second);

            // 入力の変化で書いた画素は、届かなければもう一度書き直す
            assert_eq!(
                refresh.triggers(&second, &shown, TAU).bounds(),
                Some(rect(1, 0, 1, 1)),
                "{color_type:?}"
            );
        }
    }

    /// 外接矩形が巻き込んだ画素は、書き直しの1回を残す
    #[test]
    fn pixels_swept_in_by_the_bounding_rect_keep_their_rewrite() {
        let src = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 5]);
        let mut refresh = started(ColorType::Rgb8, 5, 1, &src);

        let mut shown = src.clone();
        shown[0] = 130;
        shown[4 * 3] = 130;
        let triggers = refresh.triggers(&src, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(0, 0, 5, 1)));
        refresh.commit(&triggers, rect(0, 0, 5, 1), &src);

        // 書いた縁の滲みが許容量を超えた画素は、次のフレームで書き直す
        let mut bled = src.clone();
        bled[2 * 3] = 130;
        assert_eq!(
            refresh.triggers(&src, &bled, TAU).bounds(),
            Some(rect(2, 0, 1, 1))
        );
    }

    /// 矩形の中の画素は、書き直しの引き金でなくても入力を覚える
    #[test]
    fn the_whole_rect_remembers_the_input() {
        let first = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 5]);
        let mut refresh = started(ColorType::Rgb8, 5, 1, &first);

        // 両端が大きく動き、間の1画素が許容量の内で動く
        let mut second = first.clone();
        second[0] = 200;
        second[4 * 3] = 200;
        second[2 * 3] = 100 + TAU;
        let triggers = refresh.triggers(&second, &first, TAU);
        assert_eq!(triggers.bounds(), Some(rect(0, 0, 5, 1)));
        refresh.commit(&triggers, rect(0, 0, 5, 1), &second);

        // 間の画素は入力を覚えたので、届かなかった1回だけを書き直して止まる
        let mut shown = second.clone();
        shown[2 * 3] = 130;
        let triggers = refresh.triggers(&second, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(2, 0, 1, 1)));
        refresh.commit(&triggers, rect(2, 0, 1, 1), &second);

        assert_eq!(refresh.triggers(&second, &shown, TAU).bounds(), None);
    }

    /// 外接矩形を2枚へ割ると、どちらにも入らない画素は動かない
    #[test]
    fn pixels_outside_the_written_pieces_are_left_alone() {
        let first = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 7]);
        let mut refresh = started(ColorType::Rgb8, 7, 1, &first);

        // 両端が大きく動き、間の1画素が許容量の内で動く
        let mut second = first.clone();
        second[0] = 200;
        second[6 * 3] = 200;
        second[3 * 3] = 100 + TAU;

        let triggers = refresh.triggers(&second, &first, TAU);
        assert_eq!(triggers.bounds(), Some(rect(0, 0, 7, 1)));

        // 外接矩形を両端の2枚へ割って書く
        refresh.commit(&triggers, rect(0, 0, 1, 1), &second);
        refresh.commit(&triggers, rect(6, 0, 1, 1), &second);

        // 間の画素は入力も書き直しの1回も覚えていないので、書き直しが2回残る
        let mut shown = second.clone();
        shown[3 * 3] = 130;
        let triggers = refresh.triggers(&second, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(3, 0, 1, 1)));
        refresh.commit(&triggers, rect(3, 0, 1, 1), &second);
        assert_eq!(
            refresh.triggers(&second, &shown, TAU).bounds(),
            Some(rect(3, 0, 1, 1))
        );
    }

    /// キャンバスからはみ出した矩形は書けない
    #[test]
    #[should_panic(expected = "矩形がキャンバスからはみ出している")]
    fn a_rect_beyond_the_canvas_is_refused() {
        let src = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 8]);
        let mut refresh = started(ColorType::Rgb8, 4, 2, &src);
        let triggers = refresh.triggers(&src, &src, TAU);

        refresh.commit(&triggers, rect(0, 0, 6, 1), &src);
    }

    /// 地図は矩形からの相対で、行ごと・列ごとの範囲として読める
    #[test]
    fn the_map_reads_as_rows_and_columns() {
        const WIDTH: u32 = 4;

        let src = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 16]);
        let refresh = started(ColorType::Rgb8, WIDTH, 4, &src);

        let mut shown = src.clone();
        for (x, y) in [(1, 0), (1, 1), (1, 2), (3, 2)] {
            shown[(y * WIDTH as usize + x) * 3] = 200;
        }
        let triggers = refresh.triggers(&src, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(1, 0, 3, 3)));

        let profile = triggers.profile(rect(1, 0, 3, 3));
        assert_eq!(profile.rows, [span(0, 0), span(0, 0), span(0, 2)]);
        assert_eq!(profile.cols, [span(0, 2), None, span(2, 2)]);
        assert_eq!(profile.rows[2].map(Span::count), Some(3));
    }

    /// 範囲は矩形の中の引き金だけから決まる
    #[test]
    fn a_profile_reads_only_inside_the_rect() {
        const WIDTH: u32 = 5;

        let src = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 25]);
        let refresh = started(ColorType::Rgb8, WIDTH, 5, &src);

        // 中の1画素を、四方の引き金が囲む
        let mut shown = src.clone();
        for (x, y) in [(2, 2), (0, 2), (4, 2), (2, 0), (2, 4)] {
            shown[(y * WIDTH as usize + x) * 3] = 200;
        }
        let triggers = refresh.triggers(&src, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(0, 0, 5, 5)));

        let profile = triggers.profile(rect(1, 1, 3, 3));
        assert_eq!(profile.rows, [None, span(1, 1), None]);
        assert_eq!(profile.cols, [None, span(1, 1), None]);
    }

    /// 語をまたぐ矩形でも、範囲は矩形の端で切られる
    #[test]
    fn a_rect_spanning_words_is_clipped_at_its_edges() {
        const WIDTH: u32 = 200;

        let src = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; WIDTH as usize]);
        let refresh = started(ColorType::Rgb8, WIDTH, 1, &src);

        let mut shown = src.clone();
        for x in [63, 64, 130, 199] {
            shown[x * 3] = 200;
        }
        let triggers = refresh.triggers(&src, &shown, TAU);
        assert_eq!(triggers.bounds(), Some(rect(63, 0, 137, 1)));

        let profile = triggers.profile(rect(63, 0, 137, 1));
        assert_eq!(profile.rows, [span(0, 136)]);
        for (column, expected) in [
            (0, span(0, 0)),
            (1, span(0, 0)),
            (2, None),
            (136, span(0, 0)),
        ] {
            assert_eq!(profile.cols[column], expected, "列 {column}");
        }

        // 端の語を跨ぐ両端の引き金を、矩形の外へ出す
        let profile = triggers.profile(rect(64, 0, 67, 1));
        assert_eq!(profile.rows, [span(0, 66)]);
        assert_eq!(profile.cols[0], span(0, 0));
        assert_eq!(profile.cols[66], span(0, 0));
    }

    #[test]
    fn a_map_without_triggers_has_no_bounds_and_no_spans() {
        let src = bytes(ColorType::Rgb8, &[[100, 100, 100, 0xFF]; 12]);
        let refresh = started(ColorType::Rgb8, 4, 3, &src);
        let triggers = refresh.triggers(&src, &src, TAU);

        assert_eq!(triggers.bounds(), None);
        let profile = triggers.profile(rect(0, 0, 4, 3));
        assert_eq!(profile.rows, [None; 3]);
        assert_eq!(profile.cols, [None; 4]);
    }

    /// 許容量0の外接矩形は、厳密一致で採った矩形と一致する
    ///
    /// 完全透過の画素のRGBが0へ潰れている入力で突き合わせる。
    #[test]
    fn a_zero_tolerance_matches_the_exact_rect() {
        const WIDTH: u32 = 13;
        const HEIGHT: u32 = 7;
        const PIXELS: usize = (WIDTH * HEIGHT) as usize;

        let mut random = Random(0x2545_F491_4F6C_DD1D);
        for color_type in COLOR_TYPES {
            let bpp = color_type.bytes_per_pixel();
            let refresh = Refresh::new(WIDTH, HEIGHT, color_type);
            for round in 0..64 {
                let shown = noise(&mut random, color_type, PIXELS);
                let mut src = shown.clone();
                for _ in 0..random.below(6) {
                    let at = random.below(WIDTH * HEIGHT) as usize;
                    src[at] = noise(&mut random, color_type, 1)[0];
                }
                let (src, shown) = (bytes(color_type, &src), bytes(color_type, &shown));

                assert_eq!(
                    refresh.triggers(&src, &shown, 0).bounds(),
                    dirty_rect(&shown, &src, WIDTH as usize * bpp, bpp),
                    "{color_type:?} {round}回目"
                );
            }
        }
    }
}
