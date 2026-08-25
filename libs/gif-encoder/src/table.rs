//! カラーテーブルと、色から添字を引く対応

use crate::normalize::TRANSPARENT;
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

/// LZWの最小符号長の下限
///
/// 1色や2色のテーブルでも符号長1は使えない。
const MIN_CODE_SIZE: u8 = 2;

/// 添字順に並べたRGBの三つ組
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

    /// 論理画面記述子と画像記述子が持つカラーテーブルの大きさの欄
    pub(crate) fn size_field(&self) -> u8 {
        self.len().trailing_zeros() as u8 - 1
    }

    /// このテーブルを引く添字のLZW最小符号長
    pub(crate) fn min_code_size(&self) -> u8 {
        (self.len().trailing_zeros() as u8).max(MIN_CODE_SIZE)
    }
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
}

impl Palette {
    /// 色の和集合をカラーテーブルへ据える
    ///
    /// 透過標識が和集合にあるなら、そのエントリがそのまま透過インデックスになる。
    /// 素材自身の透過画素と未変更画素のランはどちらも「キャンバスを書き換えない」
    /// という同じ意味なので、スロットを分けない。標識が無いときだけ透過ラン用の
    /// スロットを1つ足すが、和集合が [`MAX_COLORS`] を埋めているなら足せない。
    /// 可逆性は透過ランの削減より優先するため、そのときは透過ランを諦める。
    ///
    /// # Panics
    /// 色数が [`MAX_COLORS`] を超えているとき。
    pub(crate) fn from_colors(colors: Colors) -> Self {
        // GIFは添字の局所性に無関心なので、見つけた順のまま添字を振る
        let mut entries = colors.into_indexed(|_| ()).colors().to_vec();
        if !entries.contains(&TRANSPARENT) && entries.len() < MAX_COLORS {
            entries.push(TRANSPARENT);
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

    /// 添字順に並べた色からテーブルと引く対応を作る
    ///
    /// 引く対応はテーブルのエントリそのものから作る。書き出す色がすべて完全一致で
    /// 引けるので、写した後の色を写し直しても最近傍へ落ちない。
    fn new(mut entries: Vec<u32>) -> Self {
        if entries.iter().all(|&color| color == TRANSPARENT) {
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
        let table = ColorTable::new(&entries);
        let nearest = Nearest::new(&entries);

        Palette {
            indexed,
            entries,
            table,
            transparent,
            nearest,
            approximated: 0,
        }
    }

    /// 透過でないエントリの数
    pub(crate) fn colors(&self) -> u16 {
        (self.entries.len() - usize::from(self.transparent.is_some())) as u16
    }

    /// 完全一致が無く最近傍へ写した画素数
    pub(crate) fn approximated(&self) -> u64 {
        self.approximated
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

    /// 画素の色を写す先の添字
    ///
    /// `pixel` は1画素 `bpp` バイトが並んでいること。まず完全一致を引き、外れた
    /// ときだけ最近傍探索へ落とす。量子化したテーブルにも素材の色がそのまま
    /// 載ることがあり、可逆の経路では全画素が完全一致で解決する。
    ///
    /// 透過のエントリは最近傍の候補にならない。素材自身の透過画素は完全一致で
    /// 引け、透過ラン用に足したスロットはキャンバスと一致する画素にだけ置く
    /// もので、どちらも色を近似する相手ではない。
    pub(crate) fn index_of(&mut self, pixel: &[u8], bpp: usize) -> u8 {
        match self.indexed.index_of(pixel, bpp) {
            Some(index) => index,
            None => {
                let color = pack(pixel, bpp);
                // 標識がここへ落ちるのは透過を表現できないテーブルのときだけで、
                // その画素は廃棄方法の判定が先に弾く
                debug_assert!(color != TRANSPARENT, "透過標識を色として近似している");
                self.approximated += 1;
                self.nearest.index_of(color)
            }
        }
    }
}

/// 色を `R | G<<8 | B<<16 | A<<24` へ詰める
fn pack(pixel: &[u8], bpp: usize) -> u32 {
    let alpha = if bpp == 4 { pixel[3] } else { u8::MAX };
    u32::from_le_bytes([pixel[0], pixel[1], pixel[2], alpha])
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
        let palette = Palette::from_colors(colors_of(&pixels, 4));

        assert_eq!(palette.transparent(), Some(1));
        assert_eq!(palette.table().len(), 4, "透過スロットを余分に足している");
    }

    /// 標識が無く空きがあるときは、透過ラン用のスロットを1つ足す
    #[test]
    fn an_opaque_union_gains_a_transparent_slot() {
        let pixels = [1u8, 2, 3, 4, 5, 6];
        let palette = Palette::from_colors(colors_of(&pixels, 3));

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
        let palette = Palette::from_colors(colors_of(&pixels, 3));

        assert_eq!(palette.transparent(), None);
        assert_eq!(palette.table().len(), MAX_COLORS);
    }

    /// 空きが1つだけ残っている和集合にはスロットが入る
    #[test]
    fn a_union_one_short_of_the_limit_still_gains_a_slot() {
        let pixels: Vec<u8> = (0..MAX_COLORS - 1)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let palette = Palette::from_colors(colors_of(&pixels, 3));

        assert_eq!(palette.transparent(), Some((MAX_COLORS - 1) as u8));
        assert_eq!(palette.table().len(), MAX_COLORS);
    }

    #[test]
    fn the_minimum_code_size_never_drops_below_two() {
        for (colors, size) in [(1, 2), (2, 2), (3, 2), (4, 2), (5, 3), (255, 8), (256, 8)] {
            assert_eq!(
                ColorTable::new(&gray(colors)).min_code_size(),
                size,
                "{colors}色"
            );
        }
    }
}
