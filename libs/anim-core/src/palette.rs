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

/// 色を `R | G<<8 | B<<16 | A<<24` へ詰める
///
/// `BPP` が3の画素はアルファを255とみなす。
fn pack<const BPP: usize>(pixel: &[u8]) -> u32 {
    let alpha = if BPP == 4 { pixel[3] } else { u8::MAX };
    u32::from_le_bytes([pixel[0], pixel[1], pixel[2], alpha])
}

/// 色が最初に占める表の位置
fn slot_of(color: u32) -> usize {
    (color.wrapping_mul(HASH_MULTIPLIER) >> (u32::BITS - TABLE_BITS)) as usize
}

/// 見つけた色1つ
#[derive(Debug, Clone, Copy)]
struct Entry {
    /// [`pack`] で詰めた色
    color: u32,
    /// 表の上でこの色が占める位置
    slot: usize,
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
/// 見つけた色が [`MAX_COLORS`] を超えた時点で走査をやめ、以降は何も数えない。
pub struct Colors {
    table: Table,
    /// 見つけた順の色
    entries: Vec<Entry>,
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

    /// 色数が上限を超えたか
    ///
    /// 一度真になったら戻らないため、以降の走査は要らない。
    pub fn exceeded(&self) -> bool {
        self.exceeded
    }

    /// 数えた色の種類数
    ///
    /// 上限を超えた後は数えないため、超えていない間だけ意味を持つ。
    pub fn count(&self) -> u16 {
        self.entries.len() as u16
    }

    /// 画素列に現れる色を数える
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並んでいること。`bpp` は3か4であること。
    pub fn observe(&mut self, pixels: &[u8], bpp: usize) {
        if self.exceeded {
            return;
        }

        match bpp {
            3 => self.scan::<3>(pixels),
            4 => self.scan::<4>(pixels),
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        }
    }

    fn scan<const BPP: usize>(&mut self, pixels: &[u8]) {
        for pixel in pixels.chunks_exact(BPP) {
            if !self.insert(pack::<BPP>(pixel)) {
                self.exceeded = true;
                return;
            }
        }
    }

    /// 色を1つ数える。上限を超えて入らなければ偽を返す
    ///
    /// 入らなかった時点で [`Colors::exceeded`] が立つ。
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
    /// 添字は見つけた順で、[`Colors::observe_color`] が色を足すたびに末尾へ伸びる。
    /// [`Colors::into_indexed`] が振り直す添字とは並べ替えのぶん違う。
    pub fn index_of(&self, color: u32) -> Option<u8> {
        self.table.lookup(color)
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
                self.entries.push(Entry { color, slot });
                return true;
            }
            if self.table.keys[slot] == color {
                return true;
            }
            slot = (slot + 1) & TABLE_MASK;
        }
    }

    /// 数えた色に `key` の昇順で添字を振る
    ///
    /// 並べ替えは安定で、`key` が等しい色は見つけた順に残る。
    ///
    /// # Panics
    /// 色数が上限を超えているとき。
    pub fn into_indexed<K: Ord>(mut self, key: impl Fn(u32) -> K) -> Indexed {
        assert!(!self.exceeded, "色数が上限を超えている");

        self.entries.sort_by_key(|entry| key(entry.color));

        let mut table = self.table;
        for (index, entry) in self.entries.iter().enumerate() {
            table.values[entry.slot] = index as u16 + 1;
        }

        Indexed {
            colors: self.entries.iter().map(|entry| entry.color).collect(),
            table,
        }
    }
}

/// 添字と色の対応
pub struct Indexed {
    /// 添字順に並べた色
    colors: Vec<u32>,
    table: Table,
}

impl Indexed {
    /// 添字順に並べた色
    pub fn colors(&self) -> &[u32] {
        &self.colors
    }

    /// 画素列を添字へ写して `out` へ追記する
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並び、その色がすべてこの対応に
    /// 含まれていること。`bpp` は3か4であること。
    pub fn append_indices(&self, pixels: &[u8], bpp: usize, out: &mut Vec<u8>) {
        out.reserve(pixels.len() / bpp);
        match bpp {
            3 => self.map::<3>(pixels, out),
            4 => self.map::<4>(pixels, out),
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        }
    }

    /// 画素の色の添字。この対応に無ければ `None`
    ///
    /// `pixel` は1画素 `bpp` バイトが並んでいること。`bpp` は3か4であること。
    /// 対応に無い色を写せない呼び出し元が、写す時点でそれを知るために使う。
    pub fn index_of(&self, pixel: &[u8], bpp: usize) -> Option<u8> {
        match bpp {
            3 => self.lookup(pack::<3>(pixel)),
            4 => self.lookup(pack::<4>(pixel)),
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        }
    }

    fn map<const BPP: usize>(&self, pixels: &[u8], out: &mut Vec<u8>) {
        for pixel in pixels.chunks_exact(BPP) {
            out.push(self.lookup(pack::<BPP>(pixel)).expect("対応に無い色"));
        }
    }

    /// 色の添字。この対応に無ければ `None`
    fn lookup(&self, color: u32) -> Option<u8> {
        self.table.lookup(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RGBA8の画素列を作る
    fn rgba(pixels: &[[u8; 4]]) -> Vec<u8> {
        pixels.iter().flatten().copied().collect()
    }

    /// 全画素が違う色になるRGB8の画素列を作る
    fn distinct_rgb(count: usize) -> Vec<u8> {
        (0..count)
            .flat_map(|i| [i as u8, (i >> 8) as u8, (i >> 16) as u8])
            .collect()
    }

    /// 見つけた順のまま添字を振る
    fn indexed_of(pixels: &[u8], bpp: usize) -> Indexed {
        let mut colors = Colors::new();
        colors.observe(pixels, bpp);
        colors.into_indexed(|_| ())
    }

    /// 対応に無い色を引いても、走査は表の空きで止まる
    ///
    /// 止まらなければ戻り値ではなく無限ループになるため、上限いっぱいまで
    /// 埋めた表でも一周しないことを踏む。
    #[test]
    fn a_color_outside_the_table_is_reported_as_missing() {
        let palette = indexed_of(&[1, 2, 3, 4, 5, 6], 3);
        assert_eq!(palette.lookup(pack::<3>(&[1, 2, 3])), Some(0));
        assert_eq!(palette.lookup(pack::<3>(&[4, 5, 6])), Some(1));
        for color in 0..8192u32 {
            assert_eq!(palette.lookup(color), None, "{color:#010X}");
        }

        let full = indexed_of(&distinct_rgb(MAX_COLORS), 3);
        for color in 0..8192u32 {
            assert_eq!(full.lookup(color), None, "{color:#010X}");
        }
    }

    /// 表の末尾で衝突した色は、先頭へ回り込んだ位置に入る
    ///
    /// [`slot_of`] は色に [`HASH_MULTIPLIER`] を掛けた上位 [`TABLE_BITS`] ビットを
    /// 取るため、この2色はどちらも表の最後の位置を指す。2色目は末尾が埋まっている
    /// ぶん、表の端を越えて先頭から空きを探すことになる。
    #[test]
    fn colors_colliding_at_the_last_slot_wrap_to_the_front() {
        /// 表の最後の位置へ写る色 (詰めると `0x0000_03DB`)
        const FIRST: [u8; 4] = [0xDB, 0x03, 0x00, 0x00];
        /// 同じ位置へ写るもう1つの色 (詰めると `0x0000_07B6`)
        const SECOND: [u8; 4] = [0xB6, 0x07, 0x00, 0x00];

        assert_eq!(slot_of(pack::<4>(&FIRST)), TABLE_MASK, "末尾へ写らない色");
        assert_eq!(slot_of(pack::<4>(&SECOND)), TABLE_MASK, "末尾へ写らない色");

        let pixels = rgba(&[FIRST, SECOND]);
        let mut colors = Colors::new();
        colors.observe(&pixels, 4);
        let slots: Vec<usize> = colors.entries.iter().map(|entry| entry.slot).collect();
        assert_eq!(slots, [TABLE_MASK, 0], "2色目が先頭へ回り込んでいない");

        let indexed = colors.into_indexed(|_| ());
        assert_eq!(indexed.colors().len(), 2);

        let mut indices = Vec::new();
        indexed.append_indices(&pixels, 4, &mut indices);
        assert_ne!(indices[0], indices[1]);
        for (index, pixel) in indices.iter().zip(pixels.chunks_exact(4)) {
            assert_eq!(indexed.colors()[*index as usize], pack::<4>(pixel));
        }
    }

    /// 並べ替えた後の添字でも、色は一対一に引ける
    ///
    /// 並べ替えは表に振り直した添字を通してしか反映されない。恒等でない鍵で
    /// 上限いっぱいまで埋め、振り直しを踏んだ経路が元の色へ戻ることを確かめる。
    #[test]
    fn a_reordered_table_still_maps_every_color_to_its_index() {
        let pixels = distinct_rgb(MAX_COLORS);
        let mut colors = Colors::new();
        colors.observe(&pixels, 3);
        let indexed = colors.into_indexed(std::cmp::Reverse);

        let mut indices = Vec::new();
        indexed.append_indices(&pixels, 3, &mut indices);
        assert_eq!(indices.len(), MAX_COLORS);
        assert_eq!(
            indices[0],
            (MAX_COLORS - 1) as u8,
            "並べ替えが恒等になっている"
        );
        for (index, pixel) in indices.iter().zip(pixels.chunks_exact(3)) {
            assert_eq!(indexed.colors()[*index as usize], pack::<3>(pixel));
        }
    }

    /// 画素から添字を引ける。対応に無い色は `None`
    ///
    /// 1画素あたりのバイト数が違っても、同じ色は同じ添字へ落ちる。
    #[test]
    fn a_pixel_is_looked_up_by_its_bytes() {
        let indexed = indexed_of(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0xFF]]), 4);

        assert_eq!(indexed.index_of(&[1, 2, 3, 0xFF], 4), Some(0));
        assert_eq!(indexed.index_of(&[1, 2, 3], 3), Some(0));
        assert_eq!(indexed.index_of(&[4, 5, 6], 3), Some(1));
        assert_eq!(indexed.index_of(&[1, 2, 3, 0x80], 4), None);
        assert_eq!(indexed.index_of(&[7, 8, 9], 3), None);
    }

    /// 鍵の昇順に添字を振り、鍵が等しい色は見つけた順に残る
    #[test]
    fn the_key_orders_the_indices_and_ties_keep_their_order() {
        let pixels = rgba(&[
            [0x30, 0, 0, 0xFF],
            [0x10, 0, 0, 0xFF],
            [0x10, 0, 0, 0x80],
            [0x20, 0, 0, 0xFF],
        ]);
        let mut colors = Colors::new();
        colors.observe(&pixels, 4);
        let indexed = colors.into_indexed(|color| color & 0xFF);

        let reds: Vec<u32> = indexed.colors().iter().map(|&color| color & 0xFF).collect();
        assert_eq!(reds, [0x10, 0x10, 0x20, 0x30]);
        assert_eq!(indexed.colors()[0] >> 24, 0xFF);
        assert_eq!(indexed.colors()[1] >> 24, 0x80);
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
    }

    /// 上限を超えた色は入らず、そこまでに数えた色は引けたまま残る
    #[test]
    fn a_color_beyond_the_limit_is_refused() {
        let mut colors = Colors::new();
        for index in 0..MAX_COLORS as u32 {
            assert!(colors.observe_color(index), "{index} 色目が入らない");
        }

        assert!(!colors.observe_color(MAX_COLORS as u32));
        assert!(colors.exceeded());
        assert_eq!(colors.index_of(MAX_COLORS as u32), None);
        assert_eq!(colors.index_of(0), Some(0));
        assert_eq!(
            colors.index_of(MAX_COLORS as u32 - 1),
            Some((MAX_COLORS - 1) as u8)
        );
    }

    /// 画素列から数えた色も、1色ずつ数えた色と同じ表に載る
    #[test]
    fn the_two_ways_of_counting_share_one_table() {
        let mut colors = Colors::new();
        colors.observe(&rgba(&[[1, 2, 3, 0xFF]]), 4);
        assert!(colors.observe_color(pack::<3>(&[4, 5, 6])));

        assert_eq!(colors.index_of(pack::<3>(&[1, 2, 3])), Some(0));
        assert_eq!(colors.index_of(pack::<3>(&[4, 5, 6])), Some(1));
        assert_eq!(colors.into_indexed(|_| ()).colors().len(), 2);
    }
}
