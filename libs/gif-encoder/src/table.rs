//! カラーテーブルと、色から添字を引く対応

use crate::normalize::{TRANSPARENT, pack};
use crate::quantize::Nearest;
use anim_core::{Colors, Indexed, MAX_COLORS};

/// 写す先が1つも残らないテーブルへ足す色
///
/// カラーテーブルは2エントリ未満を書けず、埋め草はどのみち黒になる。
/// その埋め草を写す先にすれば、透過だけの区間から据えたテーブルでも
/// 不透明な画素を写せる。
const OPAQUE_BLACK: u32 = 0xFF00_0000;

/// 量子化したテーブルに載せる非透過色の上限
///
/// 量子化の経路では1色多く載せるより透過ランを取る方が常に得なので、
/// 透過スロットを必ず1つ残す。
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
    /// `colors` は `R | G<<8 | B<<16 | A<<24` で詰めた色を添字順に並べたもので、
    /// 1色以上 [`MAX_COLORS`] 以下であること。パディングは黒で埋める。
    pub(crate) fn new(colors: &[u32]) -> Self {
        assert!(!colors.is_empty(), "カラーテーブルは1色以上必要");
        assert!(colors.len() <= MAX_COLORS, "色数が上限を超えている");

        let entries = colors.len().max(MIN_ENTRIES).next_power_of_two();
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
/// 「もっと良く表せる」で、据え直す値打ちは寄った画素の数で決まる。後者は
/// 「表す手立てが無い」で、画素の数に関わらず素材の色が失われる。
pub(crate) enum Fit {
    /// テーブルにその色がそのまま載っていた
    Exact,
    /// 最近傍へ寄せた
    Approximated {
        /// 写す先の色との二乗距離
        error: u32,
    },
    /// 写す先が無く、[`OPAQUE_BLACK`] の埋め草へ置いた
    Substituted,
}

/// 画素をテーブルへ写した結果
pub(crate) struct Mapped {
    /// 写す先の添字
    pub(crate) index: u8,
    /// 写せた度合い
    pub(crate) fit: Fit,
}

/// 据え直しても残すエントリ1つ
#[derive(Clone, Copy)]
pub(crate) struct Kept {
    pub(crate) color: u32,
    /// 最後に添字を出力へ書いたフレーム番号
    pub(crate) last_used: u32,
}

/// 据えたカラーテーブルと、そこへ色を写す対応
pub(crate) struct Palette {
    /// 和集合の色から添字を引く対応
    indexed: Indexed,
    /// 添字順に並べたテーブルの色
    entries: Vec<u32>,
    /// 書き出すカラーテーブル
    table: ColorTable,
    /// このテーブルの透過インデックス
    transparent: Option<u8>,
    /// 完全一致が外れた色を写す先
    nearest: Nearest,
    /// 完全一致が無く最近傍へ写した画素数
    approximated: u64,
    /// 写す先が無く埋め草へ置いた画素数
    substituted: u64,
    /// 写す先として足した [`OPAQUE_BLACK`] の添字
    fallback: Option<u8>,
    /// グローバルカラーテーブルではなく、フレームごとに書く色表か
    local: bool,
    /// エントリごとの、最後に添字を出力へ書いたフレーム番号 (0は一度も無い)
    last_used: Vec<u32>,
    /// いま処理しているフレーム番号
    frame: u32,
}

impl Palette {
    /// 色の和集合をカラーテーブルへ据える
    ///
    /// 透過標識が和集合にあるなら、そのエントリがそのまま透過インデックスになる。
    /// 素材自身の透過画素と未変更画素のランはどちらも「キャンバスを書き換えない」
    /// という同じ意味なので、スロットを分けない。標識が無いときだけ透過ラン用の
    /// スロットを1つ足す。
    ///
    /// `reserve_transparent` は、和集合が [`MAX_COLORS`] を埋めていてもスロットを
    /// 取るかを決める。取るときは最後に見つけた色を1つ落とす。落とした色の画素は
    /// 最近傍へ写り、[`Palette::approximated`] に数えられる。
    ///
    /// # Panics
    /// 色数が [`MAX_COLORS`] を超えているとき。
    pub(crate) fn from_colors(colors: Colors, reserve_transparent: bool) -> Self {
        // GIFは添字の局所性に無関心なので、見つけた順のまま添字を振る
        let mut entries = colors.into_indexed(|_| ()).colors().to_vec();
        if !entries.contains(&TRANSPARENT) {
            if reserve_transparent && entries.len() == MAX_COLORS {
                entries.pop();
            }
            if entries.len() < MAX_COLORS {
                entries.push(TRANSPARENT);
            }
        }
        Palette::new(entries)
    }

    /// 量子化した色をカラーテーブルへ据える
    ///
    /// 透過スロットを必ず1つ足す。素材自身の透過画素と未変更画素のランは
    /// どちらも「キャンバスを書き換えない」という同じ意味なので、
    /// [`Palette::from_colors`] と同じくスロットを分けない。
    ///
    /// # Panics
    /// `colors` が空か、[`QUANTIZED_COLORS`] を超えているとき。
    pub(crate) fn from_quantized(colors: &[u32]) -> Self {
        assert!(!colors.is_empty(), "量子化した色が1つも無い");
        assert!(colors.len() <= QUANTIZED_COLORS, "透過スロットが取れない");

        let mut entries = colors.to_vec();
        entries.push(TRANSPARENT);
        Palette::new(entries)
    }

    /// 維持したエントリと残差の色でテーブルを据え直す
    ///
    /// 非透過色は255色までに抑え、透過スロットを1つ確保する。維持したエントリの
    /// 最終使用は据え直した後も引き継ぐ。据え直したテーブルはフレームごとに
    /// 書き出される ([`Palette::promote_to_global`] を通したものを除く)。
    ///
    /// # Panics
    /// 非透過色が1つも残らないとき。
    pub(crate) fn from_rebuilt(kept: &[Kept], residual: &[u32]) -> Self {
        let mut entries: Vec<u32> = kept.iter().map(|entry| entry.color).collect();
        entries.extend_from_slice(residual);
        entries.truncate(QUANTIZED_COLORS);
        assert!(!entries.is_empty(), "据え直したテーブルに非透過色が無い");
        entries.push(TRANSPARENT);

        let mut palette = Palette::new(entries);
        palette.local = true;
        for entry in kept {
            if let Some(index) = palette.indexed.index_of(&entry.color.to_le_bytes(), 4) {
                palette.last_used[index as usize] = entry.last_used;
            }
        }
        palette
    }

    /// 添字順に並べた色からテーブルと引く対応を作る
    ///
    /// 引く対応はテーブルのエントリそのものから作る。書き出す色がすべて完全一致で
    /// 引けるので、写した後の色を写し直しても最近傍へ落ちない。
    fn new(mut entries: Vec<u32>) -> Self {
        let black_fallback = entries.iter().all(|&color| color == TRANSPARENT);
        if black_fallback {
            entries.push(OPAQUE_BLACK);
        }

        let bytes: Vec<u8> = entries
            .iter()
            .flat_map(|color| color.to_le_bytes())
            .collect();
        let mut observed = Colors::new();
        observed.observe(&bytes, 4);

        // 別々の箱が同じ平均色に落ちることがあり、その重複はここで畳まれる
        let indexed = observed.into_indexed(|_| ());
        let entries = indexed.colors().to_vec();
        let transparent = entries
            .iter()
            .position(|&color| color == TRANSPARENT)
            .map(|index| index as u8);
        let fallback = black_fallback.then(|| {
            entries
                .iter()
                .position(|&color| color == OPAQUE_BLACK)
                .expect("足した埋め草がテーブルに無い") as u8
        });
        let table = ColorTable::new(&entries);
        let nearest = Nearest::new(&entries);

        Palette {
            last_used: vec![0; entries.len()],
            indexed,
            entries,
            table,
            transparent,
            nearest,
            approximated: 0,
            substituted: 0,
            fallback,
            local: false,
            frame: 0,
        }
    }

    /// これから処理するフレーム番号を覚える
    ///
    /// 以降の [`Palette::mark_used`] はこの番号で最終使用を更新する。
    pub(crate) fn set_frame(&mut self, frame: u32) {
        self.frame = frame;
    }

    /// 添字を出力へ書いたことを覚える
    pub(crate) fn mark_used(&mut self, index: u8) {
        self.last_used[index as usize] = self.frame;
    }

    /// 直近 `window` フレームの出力で使った非透過エントリ
    pub(crate) fn recently_used(&self, window: u32) -> Vec<Kept> {
        let oldest = self.frame.saturating_sub(window);
        self.entries
            .iter()
            .zip(&self.last_used)
            .filter(|&(&color, &used)| color != TRANSPARENT && used > 0 && used >= oldest)
            .map(|(&color, &used)| Kept {
                color,
                last_used: used,
            })
            .collect()
    }

    /// このフレームに書くローカルカラーテーブル。グローバルのままなら `None`
    pub(crate) fn local_table(&self) -> Option<ColorTable> {
        self.local.then(|| self.table.clone())
    }

    /// フレームごとではなく、グローバルカラーテーブルとして書くテーブルにする
    ///
    /// 据え直したテーブルは既に書いたフレームと組み合わないためフレームごとに
    /// 書くが、まだ1フレームも符号化していないなら、そのテーブルをそのまま
    /// グローバルへ据えられる。
    pub(crate) fn promote_to_global(&mut self) {
        self.local = false;
    }

    /// 最近傍へ写した画素を数に加える
    pub(crate) fn note_approximated(&mut self, count: u64) {
        self.approximated += count;
    }

    /// 埋め草へ置いた画素を数に加える
    pub(crate) fn note_substituted(&mut self, count: u64) {
        self.substituted += count;
    }

    /// 透過でないエントリの数
    pub(crate) fn colors(&self) -> u16 {
        (self.entries.len() - usize::from(self.transparent.is_some())) as u16
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
        self.fallback.is_some()
    }

    /// 添字が指す色
    ///
    /// # Panics
    /// このテーブルに無い添字のとき。
    pub(crate) fn color_at(&self, index: u8) -> u32 {
        self.entries[index as usize]
    }

    /// 書き出すカラーテーブル
    pub(crate) fn table(&self) -> &ColorTable {
        &self.table
    }

    /// キャンバスを書き換えない添字。持たないテーブルでは `None`
    pub(crate) fn transparent(&self) -> Option<u8> {
        self.transparent
    }

    /// 画素の色を写す先と、そこまでの誤差
    ///
    /// `pixel` は1画素 `bpp` バイトが並んでいること。まず完全一致を引き、外れた
    /// ときだけ最近傍探索へ落とす。量子化したテーブルにも素材の色がそのまま
    /// 載ることがあり、可逆の経路では全画素が完全一致で解決する。
    ///
    /// 透過のエントリは最近傍の候補にならない。素材自身の透過画素は完全一致で
    /// 引け、透過ラン用に足したスロットはキャンバスと一致する画素にだけ置く
    /// もので、どちらも色を近似する相手ではない。
    ///
    /// 非透過エントリが [`OPAQUE_BLACK`] の埋め草しか無いテーブルでは、最近傍は
    /// 必ずその埋め草になる。そこへ落ちた画素は近似ではなく代替として返す。
    pub(crate) fn map(&mut self, pixel: &[u8], bpp: usize) -> Mapped {
        if let Some(index) = self.indexed.index_of(pixel, bpp) {
            return Mapped {
                index,
                fit: Fit::Exact,
            };
        }

        let color = pack(pixel, bpp);
        // 標識がここへ落ちるのは透過を表現できないテーブルのときだけ。
        // 先頭フレームの矩形は論理画面全体なので、素材に透過があれば
        // 標識は和集合に入る。以降のフレームで現れた標識は、廃棄方法の
        // 判定が先に弾く
        debug_assert!(color != TRANSPARENT, "透過標識を色として近似している");
        let index = self.nearest.index_of(color);
        let fit = if self.fallback == Some(index) {
            Fit::Substituted
        } else {
            Fit::Approximated {
                error: distance(color, self.entries[index as usize]),
            }
        };
        Mapped { index, fit }
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
            let table = ColorTable::new(&gray(colors));
            assert_eq!(table.len(), entries, "{colors}色");
            assert_eq!(table.bytes().len(), entries * 3, "{colors}色");
        }
    }

    #[test]
    fn the_padding_is_black() {
        let table = ColorTable::new(&[0x00FF_FFFF, 0x0000_00FF]);
        assert_eq!(table.len(), 2);

        let table = ColorTable::new(&[0x00FF_FFFF, 0x0000_00FF, 0x0000_FF00]);
        assert_eq!(
            table.bytes(),
            [255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn the_size_field_is_the_exponent_minus_one() {
        for (colors, field) in [(1, 0), (2, 0), (3, 1), (5, 2), (256, 7)] {
            assert_eq!(
                ColorTable::new(&gray(colors)).size_field(),
                field,
                "{colors}色"
            );
        }
    }

    /// 和集合を数える
    fn colors_of(pixels: &[u8], bpp: usize) -> Colors {
        let mut colors = Colors::new();
        colors.observe(pixels, bpp);
        colors
    }

    /// 透過標識が和集合にあるなら、そのエントリがそのまま透過インデックスになる
    #[test]
    fn the_marker_entry_doubles_as_the_transparent_index() {
        let pixels = [1u8, 2, 3, 0xFF, 0, 0, 0, 0, 4, 5, 6, 0xFF];
        let palette = Palette::from_colors(colors_of(&pixels, 4), false);

        assert_eq!(palette.transparent(), Some(1));
        assert_eq!(palette.table().len(), 4, "透過スロットを余分に足している");
    }

    /// 標識が無く空きがあるときは、透過ラン用のスロットを1つ足す
    #[test]
    fn an_opaque_union_gains_a_transparent_slot() {
        let pixels = [1u8, 2, 3, 4, 5, 6];
        let palette = Palette::from_colors(colors_of(&pixels, 3), false);

        assert_eq!(palette.transparent(), Some(2));
        assert_eq!(
            &palette.table().bytes()[6..9],
            [0, 0, 0],
            "透過スロットが標識のエントリになっていない"
        );
    }

    /// 和集合が上限を埋めていると透過スロットを取れない
    ///
    /// 可逆性は透過ランの削減より優先するため、色を落として空けることはしない。
    #[test]
    fn a_full_opaque_union_keeps_every_color_and_loses_the_run() {
        let pixels: Vec<u8> = (0..MAX_COLORS)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let palette = Palette::from_colors(colors_of(&pixels, 3), false);

        assert_eq!(palette.transparent(), None);
        assert_eq!(palette.table().len(), MAX_COLORS);
    }

    /// スロットを確保するときは、最後に見つけた色を明け渡す
    #[test]
    fn reserving_a_slot_drops_the_last_color_found() {
        let pixels: Vec<u8> = (0..MAX_COLORS)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let palette = Palette::from_colors(colors_of(&pixels, 3), true);

        assert_eq!(palette.transparent(), Some((MAX_COLORS - 1) as u8));
        assert_eq!(palette.colors(), (MAX_COLORS - 1) as u16);
        assert!(
            !palette
                .table()
                .bytes()
                .chunks_exact(3)
                .any(|color| color == [(MAX_COLORS - 1) as u8, ((MAX_COLORS - 1) >> 8) as u8, 0]),
            "明け渡した色がテーブルに残っている"
        );
    }

    /// 上限に届いていない和集合は、確保を頼まれても色を明け渡さない
    #[test]
    fn reserving_a_slot_keeps_every_color_below_the_limit() {
        let pixels: Vec<u8> = (0..MAX_COLORS - 1)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let palette = Palette::from_colors(colors_of(&pixels, 3), true);

        assert_eq!(palette.colors(), (MAX_COLORS - 1) as u16);
        assert_eq!(palette.transparent(), Some((MAX_COLORS - 1) as u8));
    }

    /// 空きが1つだけ残っている和集合にはスロットが入る
    #[test]
    fn a_union_one_short_of_the_limit_still_gains_a_slot() {
        let pixels: Vec<u8> = (0..MAX_COLORS - 1)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let palette = Palette::from_colors(colors_of(&pixels, 3), false);

        assert_eq!(palette.transparent(), Some((MAX_COLORS - 1) as u8));
        assert_eq!(palette.table().len(), MAX_COLORS);
    }

    /// 写す先が1つも残らないテーブルへ足す埋め草は、不透明な黒
    ///
    /// カラーテーブルのパディングは黒なので、写す先にする埋め草も黒にする。
    /// 別の色を足すと、透過だけの区間から据えたテーブルがその色を画面へ出す。
    #[test]
    fn the_padding_that_becomes_a_target_is_opaque_black() {
        let pixels = [0u8, 0, 0, 0];
        let mut palette = Palette::from_colors(colors_of(&pixels, 4), false);
        assert!(palette.black_fallback(), "写す先の埋め草を足していない");

        let mapped = palette.map(&[0x10, 0x20, 0x30, 0xFF], 4);
        assert!(
            matches!(mapped.fit, Fit::Substituted),
            "埋め草へ落ちた画素を近似として返している"
        );
        assert_eq!(palette.color_at(mapped.index), 0xFF00_0000);
    }

    /// 透過のエントリは維持の対象にならない
    ///
    /// 透過スロットは据え直したテーブルが必ず1つ取り直すもので、維持へ数えると
    /// 非透過色の空きを食う。
    #[test]
    fn the_transparent_entry_is_never_kept() {
        let pixels = [1u8, 2, 3, 4, 5, 6];
        let mut palette = Palette::from_colors(colors_of(&pixels, 3), false);
        let transparent = palette.transparent().expect("透過インデックスが無い");

        palette.set_frame(1);
        palette.mark_used(0);
        palette.mark_used(transparent);

        let kept = palette.recently_used(8);
        assert_eq!(kept.len(), 1, "透過のエントリまで維持している");
        assert_eq!(kept[0].color, 0xFF03_0201);
    }

    /// 一度も添字を書いていないエントリは維持の対象にならない
    #[test]
    fn an_entry_that_was_never_written_is_not_kept() {
        let pixels = [1u8, 2, 3, 4, 5, 6];
        let mut palette = Palette::from_colors(colors_of(&pixels, 3), false);

        palette.set_frame(1);
        palette.mark_used(0);

        let kept = palette.recently_used(8);
        assert_eq!(kept.len(), 1, "書いていないエントリまで維持している");
        assert_eq!(kept[0].color, 0xFF03_0201);
    }

    /// 維持したエントリの最終使用は、据え直した後のテーブルへ引き継ぐ
    #[test]
    fn a_kept_entry_carries_its_last_use_into_the_rebuilt_table() {
        let kept = [
            Kept {
                color: 0xFF00_0000,
                last_used: 9,
            },
            Kept {
                color: 0xFF00_00FF,
                last_used: 2,
            },
        ];
        let mut palette = Palette::from_rebuilt(&kept, &[]);
        palette.set_frame(10);

        let carried = palette.recently_used(4);
        assert_eq!(carried.len(), 1, "最終使用が引き継がれていない");
        assert_eq!(carried[0].color, 0xFF00_0000);
        assert_eq!(carried[0].last_used, 9);
    }
}
