//! 色の和集合の集計と、色から添字を引く表

/// 数え上げられる色数の上限
pub const MAX_COLORS: usize = 256;

/// 表の添字に使うビット数
const TABLE_BITS: u32 = 10;
/// 開放アドレス法の表の大きさ
///
/// [`MAX_COLORS`] より大きく取る。表に必ず空きが残ることが、色を探す走査が
/// 一周して戻ってこないことの根拠になる。
const TABLE_LEN: usize = 1 << TABLE_BITS;
const _: () = assert!(TABLE_LEN > MAX_COLORS);
/// 表の添字を取り出すマスク
const TABLE_MASK: usize = TABLE_LEN - 1;
/// 値を表全体へ散らす乗数 (2^32を黄金比で割った奇数)
const HASH_MULTIPLIER: u32 = 0x9E37_79B1;

/// 色が最初に占める表の位置
fn slot_of(color: u32) -> usize {
    (color.wrapping_mul(HASH_MULTIPLIER) >> (u32::BITS - TABLE_BITS)) as usize
}

/// 色から添字を引く表
///
/// [`Self::values`] が0の位置は空で、それ以外は添字に1を足した値が入る。
/// 同じ位置の [`Self::keys`] にその色が入る。
struct Table {
    keys: Box<[u32]>,
    values: Box<[u16]>,
}

impl Table {
    fn new() -> Self {
        Table {
            keys: vec![0; TABLE_LEN].into_boxed_slice(),
            values: vec![0; TABLE_LEN].into_boxed_slice(),
        }
    }

    /// 色の添字。表に無ければ `None`
    fn lookup(&self, color: u32) -> Option<u8> {
        let mut slot = slot_of(color);
        loop {
            let value = self.values[slot];
            // 表に必ず残る空きに当たれば、その色はどこにも入っていない
            if value == 0 {
                return None;
            }
            if self.keys[slot] == color {
                return Some((value - 1) as u8);
            }
            slot = (slot + 1) & TABLE_MASK;
        }
    }
}

/// 全フレームに現れた色の和集合
///
/// 見つけた色が [`MAX_COLORS`] を超えた時点で、以降は何も数えない。
pub struct Colors {
    table: Table,
    /// 見つけた順に並べた色
    entries: Vec<u32>,
    /// 上限を超えたか
    exceeded: bool,
}

impl Default for Colors {
    fn default() -> Self {
        Colors::new()
    }
}

impl Colors {
    /// 空の和集合を作る
    pub fn new() -> Self {
        Colors {
            table: Table::new(),
            entries: Vec::new(),
            exceeded: false,
        }
    }

    /// 数えた色の種類数
    ///
    /// 上限を超えた後は数えないため、超えていない間だけ意味を持つ。
    pub fn count(&self) -> u16 {
        self.entries.len() as u16
    }

    /// 色を1つ数える。上限を超えて入らなければ偽を返す
    ///
    /// 一度入らなかった後は、既に数えた色でも偽を返す。
    pub fn observe_color(&mut self, color: u32) -> bool {
        if self.exceeded {
            return false;
        }
        if !self.insert(color) {
            self.exceeded = true;
            return false;
        }
        true
    }

    /// 数えた色の添字。数えていなければ `None`
    ///
    /// 添字は見つけた順で、色を1つ足すたびに末尾へ伸びる。
    pub fn index_of(&self, color: u32) -> Option<u8> {
        self.table.lookup(color)
    }

    /// 見つけた順に並べた色
    pub fn colors(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.entries.iter().copied()
    }

    /// 色を1つ覚える。上限を超えて入らなければ偽を返す
    fn insert(&mut self, color: u32) -> bool {
        let mut slot = slot_of(color);
        loop {
            if self.table.values[slot] == 0 {
                if self.entries.len() == MAX_COLORS {
                    return false;
                }
                self.table.keys[slot] = color;
                self.table.values[slot] = self.entries.len() as u16 + 1;
                self.entries.push(color);
                return true;
            }
            if self.table.keys[slot] == color {
                return true;
            }
            slot = (slot + 1) & TABLE_MASK;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 色を順に数えた表
    fn counted(colors: impl IntoIterator<Item = u32>) -> Colors {
        let mut counted = Colors::new();
        for color in colors {
            assert!(counted.observe_color(color), "{color:#010X} が入らない");
        }
        counted
    }

    /// 数えていない色を引いても、走査は表の空きで止まる
    ///
    /// 止まらなければ戻り値ではなく無限ループになるため、上限いっぱいまで
    /// 埋めた表でも一周しないことを踏む。
    #[test]
    fn a_color_outside_the_table_is_reported_as_missing() {
        let colors = counted([0xFF03_0201, 0xFF06_0504]);
        assert_eq!(colors.index_of(0xFF03_0201), Some(0));
        assert_eq!(colors.index_of(0xFF06_0504), Some(1));
        for color in 0..8192u32 {
            assert_eq!(colors.index_of(color), None, "{color:#010X}");
        }

        let full = counted((0..MAX_COLORS as u32).map(|i| 0xFF00_0000 | i));
        for color in 0..8192u32 {
            assert_eq!(full.index_of(color), None, "{color:#010X}");
        }
    }

    /// 表の末尾で衝突した色は、先頭へ回り込んだ位置に入る
    ///
    /// [`slot_of`] は色に [`HASH_MULTIPLIER`] を掛けた上位 [`TABLE_BITS`] ビットを
    /// 取るため、この2色はどちらも表の最後の位置を指す。2色目は末尾が埋まっている
    /// ぶん、表の端を越えて先頭から空きを探すことになる。
    #[test]
    fn colors_colliding_at_the_last_slot_wrap_to_the_front() {
        /// 表の最後の位置へ写る色
        const FIRST: u32 = 0x0000_03DB;
        /// 同じ位置へ写るもう1つの色
        const SECOND: u32 = 0x0000_07B6;

        assert_eq!(slot_of(FIRST), TABLE_MASK, "末尾へ写らない色");
        assert_eq!(slot_of(SECOND), TABLE_MASK, "末尾へ写らない色");

        let colors = counted([FIRST, SECOND]);
        assert_eq!(colors.count(), 2, "2色目を数えていない");
        assert_eq!(colors.table.keys[TABLE_MASK], FIRST);
        assert_eq!(
            colors.table.keys[0], SECOND,
            "2色目が先頭へ回り込んでいない"
        );
        assert_ne!(colors.table.values[0], 0, "回り込んだ位置が空のまま");
        assert_eq!(colors.index_of(FIRST), Some(0));
        assert_eq!(colors.index_of(SECOND), Some(1));
    }

    /// 1色ずつ数えた色は、見つけた順の添字で引ける
    #[test]
    fn a_color_counted_on_its_own_is_looked_up_by_the_order_it_was_found() {
        let mut colors = Colors::new();
        for color in [0x30u32, 0x10, 0x30, 0x20] {
            assert!(colors.observe_color(color));
        }

        assert_eq!(colors.count(), 3, "同じ色を二重に数えている");
        assert_eq!(colors.index_of(0x30), Some(0));
        assert_eq!(colors.index_of(0x10), Some(1));
        assert_eq!(colors.index_of(0x20), Some(2));
        assert_eq!(colors.index_of(0x40), None);
        assert_eq!(colors.colors().collect::<Vec<u32>>(), [0x30, 0x10, 0x20]);
    }

    /// 上限を超えた色は入らず、そこまでに数えた色は引けたまま残る
    #[test]
    fn a_color_beyond_the_limit_is_refused() {
        let mut colors = Colors::new();
        for index in 0..MAX_COLORS as u32 {
            assert!(colors.observe_color(index), "{index} 色目が入らない");
        }

        assert!(!colors.observe_color(MAX_COLORS as u32));
        assert!(!colors.observe_color(0), "溢れた後に数えている");
        assert_eq!(colors.index_of(MAX_COLORS as u32), None);
        assert_eq!(colors.index_of(0), Some(0));
        assert_eq!(
            colors.index_of(MAX_COLORS as u32 - 1),
            Some((MAX_COLORS - 1) as u8)
        );
    }
}
