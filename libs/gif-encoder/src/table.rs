//! カラーテーブル

use anim_core::MAX_COLORS;

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
