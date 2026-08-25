//! カラーテーブルと、色から添字を引く対応

use crate::normalize::TRANSPARENT;
use anim_core::{Colors, Indexed, MAX_COLORS};

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
    /// 書き出すカラーテーブル
    table: ColorTable,
    /// このテーブルの透過インデックス
    transparent: Option<u8>,
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
        let indexed = colors.into_indexed(|_| ());
        let mut entries = indexed.colors().to_vec();
        let transparent = match entries.iter().position(|&color| color == TRANSPARENT) {
            Some(index) => Some(index as u8),
            None if entries.len() < MAX_COLORS => {
                entries.push(TRANSPARENT);
                Some((entries.len() - 1) as u8)
            }
            None => None,
        };
        let table = ColorTable::new(&entries);

        Palette {
            indexed,
            table,
            transparent,
        }
    }

    /// 書き出すカラーテーブル
    pub(crate) fn table(&self) -> &ColorTable {
        &self.table
    }

    /// キャンバスを書き換えない添字。持たないテーブルでは `None`
    pub(crate) fn transparent(&self) -> Option<u8> {
        self.transparent
    }

    /// 画素列を添字へ写して `out` へ追記する
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並び、その色がすべてこのテーブルに
    /// 載っていること。
    pub(crate) fn append_indices(&self, pixels: &[u8], bpp: usize, out: &mut Vec<u8>) {
        self.indexed.append_indices(pixels, bpp, out);
    }
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
