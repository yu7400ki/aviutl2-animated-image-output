//! カラーテーブルと、色から添字を引く対応

use crate::normalize::{TRANSPARENT, pack};
use crate::quantize::Nearest;
use anim_core::{Colors, MAX_COLORS};

/// 写す先が1つも残らないテーブルへ足す色
///
/// 非透過色を1つも持たないテーブルでも、不透明な画素をここへ写せる。
const OPAQUE_BLACK: u32 = 0xFF00_0000;

/// 閉じたテーブルに載せる非透過色の上限
///
/// 閉じるときは1色多く載せるより透過ランを取る方が常に得なので、
/// 透過添字の余地を必ず1つ残す。
pub(crate) const QUANTIZED_COLORS: usize = MAX_COLORS - 1;

/// カラーテーブルが持てる最小のエントリ数
///
/// 大きさの欄が `log2(エントリ数) - 1` なので、1色しか無くても2エントリ書く。
const MIN_ENTRIES: usize = 2;

/// 添字順に並べたRGBの三つ組
#[derive(Clone)]
pub(crate) struct ColorTable {
    /// RGBの三つ組を並べたバイト列 (長さは `3 * エントリ数`)
    bytes: Vec<u8>,
}

impl ColorTable {
    /// `colors` を並べ、エントリ数を2の冪へ切り上げる
    ///
    /// `colors` は `R | G<<8 | B<<16 | A<<24` で詰めた色を添字順に並べたもの。
    /// `transparent` の添字はそのどれでもないため、そこまで届く大きさにする。
    /// パディングは黒で埋める。
    ///
    /// # Panics
    /// 色数が [`MAX_COLORS`] を超えているとき。
    pub(crate) fn new(colors: &[u32], transparent: Option<u8>) -> Self {
        assert!(colors.len() <= MAX_COLORS, "色数が上限を超えている");

        let reach = transparent.map_or(0, |index| usize::from(index) + 1);
        let entries = colors.len().max(reach).max(MIN_ENTRIES).next_power_of_two();
        let mut bytes = vec![0; entries * 3];
        for (entry, color) in bytes.chunks_exact_mut(3).zip(colors) {
            entry.copy_from_slice(&color.to_le_bytes()[..3]);
        }
        ColorTable { bytes }
    }

    /// エントリ数 (2..=256 の2の冪)
    pub(crate) fn len(&self) -> usize {
        self.bytes.len() / 3
    }

    /// 書き出すバイト列
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 画像記述子が持つローカルカラーテーブルの大きさの欄
    pub(crate) fn size_field(&self) -> u8 {
        self.len().trailing_zeros() as u8 - 1
    }
}

/// 画素をテーブルへ写せた度合い
///
/// [`Self::Approximated`] と [`Self::Substituted`] は性質が違う。前者は
/// 「もっと良く表せる」で素材の色に寄った色が画面に残り、後者は
/// 「表す手立てが無い」で素材の色が失われる。どちらも写した先までの隔たりを
/// 二乗距離で持つ。
pub(crate) enum Fit {
    /// テーブルにその色がそのまま載っていた
    Exact,
    /// 最近傍へ寄せた
    Approximated {
        /// 写す先の色との二乗距離
        error: u32,
    },
    /// 写す先が無く、[`OPAQUE_BLACK`] の埋め草へ置いた
    Substituted {
        /// 埋め草との二乗距離
        error: u32,
    },
}

/// 画素をテーブルへ写した結果
pub(crate) struct Mapped {
    /// 写す先の添字
    pub(crate) index: u8,
    /// 写せた度合い
    pub(crate) fit: Fit,
}

/// 閉じたテーブルが持つ、完全一致が外れた色の写し方
struct Settled {
    /// 完全一致が外れた色を写す先
    nearest: Nearest,
    /// 写す先として足した [`OPAQUE_BLACK`] の添字
    fallback: Option<u8>,
}

/// カラーテーブルと、そこへ色を写す対応
///
/// 添字は色を見つけた順に振り、一度振った添字は動かさない。開いている間は
/// 色を足せて完全一致だけを引き、閉じた後は足せなくなる代わりに最近傍へも写す。
///
/// 透過添字はエントリを占有しない。まだ色の割り当たっていない最小の添字を、
/// フレームごとにグラフィック制御拡張で宣言する。
pub(crate) struct Palette {
    /// 色から添字を引く対応
    lookup: Colors,
    /// 添字順に並べた色
    entries: Vec<u32>,
    /// 閉じたテーブルの写し方。開いている間は `None`
    settled: Option<Settled>,
    /// 透過標識を一度でも見たか
    transparent_seen: bool,
    /// 完全一致が無く最近傍へ写した画素数
    approximated: u64,
    /// 写す先が無く埋め草へ置いた画素数
    substituted: u64,
    /// グローバルカラーテーブルではなく、フレームごとに書く色表か
    local: bool,
}

impl Palette {
    /// 色が1つも入っていない、開いたテーブル
    pub(crate) fn new() -> Self {
        Palette {
            lookup: Colors::new(),
            entries: Vec::new(),
            settled: None,
            transparent_seen: false,
            approximated: 0,
            substituted: 0,
            local: false,
        }
    }

    /// 渡した色だけを載せた、そのフレーム専用の閉じたテーブル
    ///
    /// 非透過色は [`QUANTIZED_COLORS`] までに抑えるので、透過添字が必ず取れる。
    /// このテーブルは画像記述子のローカルカラーテーブルとして書き出される。
    pub(crate) fn from_colors(colors: &[u32]) -> Self {
        let mut palette = Palette::new();
        for &color in colors {
            if palette.entries.len() == QUANTIZED_COLORS {
                break;
            }
            palette.push(color);
        }

        palette.local = true;
        palette.settle(&[]);
        palette
    }

    /// まだ色を足せるか
    pub(crate) fn is_open(&self) -> bool {
        self.settled.is_none()
    }

    /// このフレームで新しく要る色をまとめて足す。上限に収まらなければ何も足さない
    ///
    /// 走査するのは直前のフレームから変わった画素だけで、`previous` が空なら
    /// 全画素。変わっていない画素の色は、その画素が最後に変わったフレームで
    /// 既に足されている。
    ///
    /// 透過標識は添字を占めないが、見た後の上限は透過添字のぶん1つ縮む。
    ///
    /// # Panics
    /// 閉じたテーブルのとき。
    pub(crate) fn admit(&mut self, bpp: usize, previous: &[u8], frame: &[u8]) -> bool {
        assert!(self.is_open(), "閉じたテーブルへ色を足そうとしている");

        let mut fresh = Colors::new();
        for (at, pixel) in frame.chunks_exact(bpp).enumerate() {
            let at = at * bpp;
            if !previous.is_empty() && previous[at..at + bpp] == *pixel {
                continue;
            }

            let color = pack(pixel, bpp);
            if color == TRANSPARENT {
                self.transparent_seen = true;
                continue;
            }
            if self.lookup.index_of(color).is_some() {
                continue;
            }
            if !fresh.observe_color(color) || !self.fits(fresh.count()) {
                return false;
            }
        }
        if !self.fits(fresh.count()) {
            return false;
        }

        for &color in fresh.into_indexed(|_| ()).colors() {
            self.push(color);
        }
        true
    }

    /// 量子化した色を足してテーブルを閉じる
    ///
    /// 既に振った添字は動かないため、閉じる前に書いたフレームが指す色は変わらない。
    /// 非透過色が1つも無いときだけ、写す先として [`OPAQUE_BLACK`] を添字0へ置く。
    ///
    /// # Panics
    /// 閉じたテーブルのとき。
    pub(crate) fn settle(&mut self, quantized: &[u32]) {
        assert!(self.is_open(), "閉じたテーブルを閉じ直そうとしている");

        for &color in quantized {
            self.push(color);
        }

        let fallback = self.entries.is_empty().then(|| {
            self.push(OPAQUE_BLACK);
            0
        });
        self.settled = Some(Settled {
            nearest: Nearest::new(&self.entries),
            fallback,
        });
    }

    /// 添字を出力へ書いたことを確かめる
    pub(crate) fn mark_used(&self, index: u8) {
        debug_assert!(
            usize::from(index) < self.entries.len() || Some(index) == self.transparent(),
            "割り当て済みの色でも透過添字でもない添字"
        );
    }

    /// このフレームに書くローカルカラーテーブル。グローバルのままなら `None`
    pub(crate) fn local_table(&self) -> Option<ColorTable> {
        self.local
            .then(|| ColorTable::new(&self.entries, self.transparent()))
    }

    /// グローバルカラーテーブルへ書く、割り当て済みの色のバイト列
    pub(crate) fn global_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.entries.len() * 3);
        for color in &self.entries {
            bytes.extend_from_slice(&color.to_le_bytes()[..3]);
        }
        bytes
    }

    /// 最近傍へ写した画素を数に加える
    pub(crate) fn note_approximated(&mut self, count: u64) {
        self.approximated += count;
    }

    /// 埋め草へ置いた画素を数に加える
    pub(crate) fn note_substituted(&mut self, count: u64) {
        self.substituted += count;
    }

    /// 割り当て済みの色数
    pub(crate) fn colors(&self) -> u16 {
        self.entries.len() as u16
    }

    /// 完全一致が無く最近傍へ写した画素数
    pub(crate) fn approximated(&self) -> u64 {
        self.approximated
    }

    /// 写す先が無く埋め草へ置いた画素数
    pub(crate) fn substituted(&self) -> u64 {
        self.substituted
    }

    /// 非透過色が1つも無く、写す先として黒を足したか
    pub(crate) fn black_fallback(&self) -> bool {
        self.settled
            .as_ref()
            .is_some_and(|settled| settled.fallback.is_some())
    }

    /// 添字が指す色
    ///
    /// # Panics
    /// 色の割り当たっていない添字のとき。
    pub(crate) fn color_at(&self, index: u8) -> u32 {
        self.entries[usize::from(index)]
    }

    /// キャンバスを書き換えない添字。色で埋まったテーブルでは `None`
    pub(crate) fn transparent(&self) -> Option<u8> {
        (self.entries.len() < MAX_COLORS).then_some(self.entries.len() as u8)
    }

    /// 画素の色を写す先と、そこまでの誤差
    ///
    /// `pixel` は1画素 `bpp` バイトが並んでいること。まず完全一致を引き、外れた
    /// ときだけ最近傍探索へ落とす。閉じたテーブルにも素材の色がそのまま
    /// 載ることがあり、可逆の経路では全画素が完全一致で解決する。
    ///
    /// 透過標識は透過添字への完全一致になる。素材自身の透過画素と未変更画素の
    /// ランはどちらも「キャンバスを書き換えない」という同じ意味なので、
    /// スロットを分けない。
    ///
    /// 非透過エントリが [`OPAQUE_BLACK`] の埋め草しか無いテーブルでは、最近傍は
    /// 必ずその埋め草になる。そこへ落ちた画素は近似ではなく代替として返す。
    ///
    /// # Panics
    /// 透過添字を持たないテーブルへ透過標識を渡したとき。開いたテーブルへ
    /// 割り当てていない色を渡したとき。
    pub(crate) fn map(&mut self, pixel: &[u8], bpp: usize) -> Mapped {
        let color = pack(pixel, bpp);
        if color == TRANSPARENT {
            return Mapped {
                index: self.transparent().expect("透過標識を書く添字が無い"),
                fit: Fit::Exact,
            };
        }
        if let Some(index) = self.lookup.index_of(color) {
            return Mapped {
                index,
                fit: Fit::Exact,
            };
        }

        let settled = self
            .settled
            .as_mut()
            .expect("開いたテーブルに割り当てていない色を写している");
        let index = settled.nearest.index_of(color);
        let error = distance(color, self.entries[usize::from(index)]);
        let fit = if settled.fallback == Some(index) {
            Fit::Substituted { error }
        } else {
            Fit::Approximated { error }
        };
        Mapped { index, fit }
    }

    /// 新しく `count` 色を足しても上限に収まるか
    ///
    /// 透過標識を見た素材では、透過添字のぶんを1つ残す。
    fn fits(&self, count: u16) -> bool {
        let limit = MAX_COLORS - usize::from(self.transparent_seen);
        self.entries.len() + usize::from(count) <= limit
    }

    /// 色を添字順の末尾へ足す
    ///
    /// 既に載っている色と、上限を超える色は落ちる。
    fn push(&mut self, color: u32) {
        let before = self.lookup.count();
        if self.lookup.observe_color(color) && self.lookup.count() != before {
            self.entries.push(color);
        }
    }
}

/// 2色のRGBの二乗距離
fn distance(a: u32, b: u32) -> u32 {
    let [ar, ag, ab, _] = a.to_le_bytes();
    let [br, bg, bb, _] = b.to_le_bytes();
    let squared = |x: u8, y: u8| {
        let difference = i32::from(x) - i32::from(y);
        (difference * difference) as u32
    };
    squared(ar, br) + squared(ag, bg) + squared(ab, bb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(count: usize) -> Vec<u32> {
        (0..count as u32).map(|i| i | i << 8 | i << 16).collect()
    }

    /// 画素列を1フレームとして受け入れた、開いたテーブル
    fn opened(pixels: &[u8], bpp: usize) -> Palette {
        let mut palette = Palette::new();
        assert!(palette.admit(bpp, &[], pixels), "1フレーム目が入らない");
        palette
    }

    #[test]
    fn the_entry_count_is_rounded_up_to_a_power_of_two() {
        for (colors, entries) in [
            (1, 2),
            (2, 2),
            (3, 4),
            (4, 4),
            (5, 8),
            (8, 8),
            (9, 16),
            (129, 256),
            (256, 256),
        ] {
            let table = ColorTable::new(&gray(colors), None);
            assert_eq!(table.len(), entries, "{colors}色");
            assert_eq!(table.bytes().len(), entries * 3, "{colors}色");
        }
    }

    /// 透過添字は色を持たないため、そこまで届くエントリ数が要る
    #[test]
    fn the_transparent_index_widens_the_table() {
        for (colors, entries) in [(1, 2), (2, 4), (4, 8), (255, 256)] {
            let table = ColorTable::new(&gray(colors), Some(colors as u8));
            assert_eq!(table.len(), entries, "{colors}色");
        }
    }

    #[test]
    fn the_padding_is_black() {
        let table = ColorTable::new(&[0x00FF_FFFF, 0x0000_00FF], None);
        assert_eq!(table.len(), 2);

        let table = ColorTable::new(&[0x00FF_FFFF, 0x0000_00FF, 0x0000_FF00], None);
        assert_eq!(
            table.bytes(),
            [255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn the_size_field_is_the_exponent_minus_one() {
        for (colors, field) in [(1, 0), (2, 0), (3, 1), (5, 2), (256, 7)] {
            assert_eq!(
                ColorTable::new(&gray(colors), None).size_field(),
                field,
                "{colors}色"
            );
        }
    }

    /// 添字は色を見つけた順に振る
    #[test]
    fn indices_follow_the_order_the_colors_were_found() {
        let palette = opened(&[1u8, 2, 3, 4, 5, 6, 1, 2, 3], 3);

        assert_eq!(palette.colors(), 2);
        assert_eq!(palette.color_at(0), 0xFF03_0201);
        assert_eq!(palette.color_at(1), 0xFF06_0504);
    }

    /// 直前のフレームから変わっていない画素は走査しない
    #[test]
    fn unchanged_pixels_are_left_out_of_the_scan() {
        let first = [1u8, 2, 3, 4, 5, 6];
        let second = [1u8, 2, 3, 7, 8, 9];

        let mut palette = opened(&first, 3);
        assert!(palette.admit(3, &first, &second));
        assert_eq!(palette.colors(), 3);
        assert_eq!(palette.color_at(2), 0xFF09_0807);
    }

    /// 透過標識は添字を占めない
    #[test]
    fn the_marker_takes_no_entry() {
        let palette = opened(&[0u8, 0, 0, 0, 1, 2, 3, 0xFF], 4);

        assert_eq!(palette.colors(), 1);
        assert_eq!(palette.transparent(), Some(1));
    }

    /// 透過添字は、まだ色の割り当たっていない最小の添字
    #[test]
    fn the_transparent_index_follows_the_colors_allocated_so_far() {
        let first = [1u8, 2, 3];
        let mut palette = opened(&first, 3);
        assert_eq!(palette.transparent(), Some(1));

        assert!(palette.admit(3, &first, &[4, 5, 6]));
        assert_eq!(palette.transparent(), Some(2));
    }

    /// 上限に収まらないフレームは1色も足さない
    ///
    /// 透過標識を見ていない素材の上限は [`MAX_COLORS`]。
    #[test]
    fn a_frame_that_does_not_fit_adds_nothing() {
        let full: Vec<u8> = (0..MAX_COLORS)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let mut palette = opened(&full, 3);
        assert_eq!(palette.colors(), MAX_COLORS as u16);
        assert_eq!(palette.transparent(), None, "色で埋まっても透過添字がある");

        assert!(!palette.admit(3, &full, &[0xFF, 0xFF, 0xFF]));
        assert_eq!(palette.colors(), MAX_COLORS as u16, "色を足している");
    }

    /// 透過標識を見た素材の上限は、透過添字のぶん1つ縮む
    #[test]
    fn a_marker_reserves_the_last_index() {
        let mut pixels: Vec<u8> = (0..MAX_COLORS - 1)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0, 0xFF])
            .collect();
        pixels.extend_from_slice(&[0, 0, 0, 0]);

        let mut palette = opened(&pixels, 4);
        assert_eq!(palette.colors(), (MAX_COLORS - 1) as u16);
        assert_eq!(palette.transparent(), Some((MAX_COLORS - 1) as u8));

        assert!(!palette.admit(4, &[], &[0xFF, 0xFF, 0xFF, 0xFF]));
    }

    /// 閉じたテーブルは、割り当て済みの色をそのままの添字で残す
    #[test]
    fn settling_keeps_the_indices_that_were_already_handed_out() {
        let mut palette = opened(&[1u8, 2, 3, 4, 5, 6], 3);
        palette.settle(&[0xFF80_8080, 0xFF03_0201]);

        assert_eq!(palette.color_at(0), 0xFF03_0201);
        assert_eq!(palette.color_at(1), 0xFF06_0504);
        assert_eq!(palette.color_at(2), 0xFF80_8080, "量子化した色が入らない");
        assert_eq!(palette.colors(), 3, "重なった色を二重に足している");
    }

    /// 閉じたテーブルに無い色は最近傍へ写る
    #[test]
    fn a_color_outside_a_settled_table_is_approximated() {
        let mut palette = opened(&[0u8, 0, 0], 3);
        palette.settle(&[]);

        let mapped = palette.map(&[1, 0, 0], 3);
        assert_eq!(mapped.index, 0);
        assert!(matches!(mapped.fit, Fit::Approximated { .. }));
    }

    /// 透過標識は透過添字への完全一致になる
    #[test]
    fn the_marker_maps_to_the_transparent_index() {
        let mut palette = opened(&[1u8, 2, 3, 0xFF], 4);
        let mapped = palette.map(&[0, 0, 0, 0], 4);

        assert_eq!(mapped.index, 1);
        assert!(matches!(mapped.fit, Fit::Exact));
    }

    /// 写す先が1つも残らないテーブルへ足す埋め草は、不透明な黒
    #[test]
    fn the_padding_that_becomes_a_target_is_opaque_black() {
        let mut palette = opened(&[0u8, 0, 0, 0], 4);
        assert_eq!(palette.colors(), 0);

        palette.settle(&[]);
        assert!(palette.black_fallback(), "写す先の埋め草を足していない");

        let mapped = palette.map(&[0x10, 0x20, 0x30, 0xFF], 4);
        assert!(
            matches!(mapped.fit, Fit::Substituted { .. }),
            "埋め草へ落ちた画素を近似として返している"
        );
        assert_eq!(palette.color_at(mapped.index), OPAQUE_BLACK);
    }

    /// フレームごとの色表は、渡した色を見つけた順に載せる
    #[test]
    fn a_local_table_keeps_the_order_of_the_colors_it_was_given() {
        let palette = Palette::from_colors(&[0xFF00_00FF, 0xFF00_0000]);

        assert_eq!(palette.colors(), 2);
        assert_eq!(palette.color_at(0), 0xFF00_00FF);
        assert!(palette.local_table().is_some(), "色表を運んでいない");
    }

    /// フレームごとの色表は非透過色を [`QUANTIZED_COLORS`] までに抑える
    #[test]
    fn a_local_table_leaves_room_for_the_transparent_index() {
        let colors: Vec<u32> = (0..MAX_COLORS).map(|i| 0xFF00_0000 | i as u32).collect();
        let palette = Palette::from_colors(&colors);

        assert_eq!(palette.colors(), QUANTIZED_COLORS as u16);
        assert_eq!(palette.transparent(), Some(QUANTIZED_COLORS as u8));
    }
}
