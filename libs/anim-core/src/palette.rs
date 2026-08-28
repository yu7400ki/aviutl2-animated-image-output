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
    /// [`pack`] で詰めた色を見つけた順に並べたもの
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
    /// 添字は見つけた順で、色を1つ足すたびに末尾へ伸びる。
    pub fn index_of(&self, color: u32) -> Option<u8> {
        self.table.lookup(color)
    }

    /// 見つけた順に並べた色
    pub fn colors(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.entries.iter().copied()
    }

    /// 画素の色の添字。数えていなければ `None`
    ///
    /// `pixel` は1画素 `bpp` バイトが並んでいること。`bpp` は3か4であること。
    pub fn index_of_pixel(&self, pixel: &[u8], bpp: usize) -> Option<u8> {
        match bpp {
            3 => self.table.lookup(pack::<3>(pixel)),
            4 => self.table.lookup(pack::<4>(pixel)),
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        }
    }

    /// 画素列を添字へ写して `out` へ追記する
    ///
    /// まだ数えていない色は見つけた順に数える。`pixels` は1画素 `bpp` バイトが
    /// 隙間なく並んでいること。`bpp` は3か4であること。
    ///
    /// 上限を超えて数えられない色に当たったら、`out` を呼び出し前の長さへ戻して
    /// 偽を返す。
    pub fn append_indices(&mut self, pixels: &[u8], bpp: usize, out: &mut Vec<u8>) -> bool {
        out.reserve(pixels.len() / bpp);
        let start = out.len();
        let mapped = match bpp {
            3 => self.map::<3>(pixels, out),
            4 => self.map::<4>(pixels, out),
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        };
        if !mapped {
            out.truncate(start);
        }
        mapped
    }

    fn map<const BPP: usize>(&mut self, pixels: &[u8], out: &mut Vec<u8>) -> bool {
        for pixel in pixels.chunks_exact(BPP) {
            let color = pack::<BPP>(pixel);
            let index = match self.table.lookup(color) {
                Some(index) => index,
                None => {
                    if !self.insert(color) {
                        self.exceeded = true;
                        return false;
                    }
                    (self.entries.len() - 1) as u8
                }
            };
            out.push(index);
        }
        true
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

    /// 画素列を数えた表
    fn counted(pixels: &[u8], bpp: usize) -> Colors {
        let mut colors = Colors::new();
        colors.observe(pixels, bpp);
        colors
    }

    /// 数えていない色を引いても、走査は表の空きで止まる
    ///
    /// 止まらなければ戻り値ではなく無限ループになるため、上限いっぱいまで
    /// 埋めた表でも一周しないことを踏む。
    #[test]
    fn a_color_outside_the_table_is_reported_as_missing() {
        let colors = counted(&[1, 2, 3, 4, 5, 6], 3);
        assert_eq!(colors.index_of(pack::<3>(&[1, 2, 3])), Some(0));
        assert_eq!(colors.index_of(pack::<3>(&[4, 5, 6])), Some(1));
        for color in 0..8192u32 {
            assert_eq!(colors.index_of(color), None, "{color:#010X}");
        }

        let full = counted(&distinct_rgb(MAX_COLORS), 3);
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
        /// 表の最後の位置へ写る色 (詰めると `0x0000_03DB`)
        const FIRST: [u8; 4] = [0xDB, 0x03, 0x00, 0x00];
        /// 同じ位置へ写るもう1つの色 (詰めると `0x0000_07B6`)
        const SECOND: [u8; 4] = [0xB6, 0x07, 0x00, 0x00];

        assert_eq!(slot_of(pack::<4>(&FIRST)), TABLE_MASK, "末尾へ写らない色");
        assert_eq!(slot_of(pack::<4>(&SECOND)), TABLE_MASK, "末尾へ写らない色");

        let colors = counted(&rgba(&[FIRST, SECOND]), 4);
        assert_eq!(colors.count(), 2, "2色目を数えていない");
        assert_eq!(colors.table.keys[TABLE_MASK], pack::<4>(&FIRST));
        assert_eq!(
            colors.table.keys[0],
            pack::<4>(&SECOND),
            "2色目が先頭へ回り込んでいない"
        );
        assert_ne!(colors.table.values[0], 0, "回り込んだ位置が空のまま");
        assert_eq!(colors.index_of(pack::<4>(&FIRST)), Some(0));
        assert_eq!(colors.index_of(pack::<4>(&SECOND)), Some(1));
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

    /// 写しながら数えると、添字は色を見つけた順に振られる
    #[test]
    fn appending_indices_assigns_them_in_the_order_the_colors_are_found() {
        let pixels = rgba(&[
            [0x30, 0, 0, 0xFF],
            [0x10, 0, 0, 0xFF],
            [0x30, 0, 0, 0xFF],
            [0x10, 0, 0, 0x80],
        ]);
        let mut colors = Colors::new();
        let mut indices = Vec::new();
        assert!(colors.append_indices(&pixels, 4, &mut indices));

        assert_eq!(indices, [0, 1, 0, 2]);
        assert_eq!(
            colors.colors().collect::<Vec<u32>>(),
            [
                pack::<4>(&[0x30, 0, 0, 0xFF]),
                pack::<4>(&[0x10, 0, 0, 0xFF]),
                pack::<4>(&[0x10, 0, 0, 0x80]),
            ]
        );
    }

    /// 既に数えた色は、写すときも同じ添字を引く
    #[test]
    fn already_counted_colors_keep_their_indices_when_mapped() {
        let mut colors = Colors::new();
        colors.observe(&distinct_rgb(3), 3);

        let mut indices = Vec::new();
        assert!(colors.append_indices(&distinct_rgb(3), 3, &mut indices));
        assert_eq!(indices, [0, 1, 2]);
        assert_eq!(colors.count(), 3);
    }

    /// 上限を超える画素列は写せず、追記した添字も残らない
    #[test]
    fn a_pixel_beyond_the_limit_leaves_the_output_untouched() {
        let mut colors = Colors::new();
        let mut indices = vec![0xAA];
        assert!(!colors.append_indices(&distinct_rgb(MAX_COLORS + 1), 3, &mut indices));

        assert_eq!(indices, [0xAA]);
        assert!(colors.exceeded());
    }

    /// 画素から添字を引ける。数えていない色は `None`
    #[test]
    fn a_counted_pixel_is_looked_up_by_its_bytes() {
        let mut colors = Colors::new();
        colors.observe(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0x80]]), 4);

        assert_eq!(colors.index_of_pixel(&[1, 2, 3, 0xFF], 4), Some(0));
        assert_eq!(colors.index_of_pixel(&[1, 2, 3], 3), Some(0));
        assert_eq!(colors.index_of_pixel(&[4, 5, 6, 0x80], 4), Some(1));
        assert_eq!(colors.index_of_pixel(&[4, 5, 6], 3), None);
    }

    /// 画素列から数えた色も、1色ずつ数えた色と同じ表に載る
    #[test]
    fn the_two_ways_of_counting_share_one_table() {
        let mut colors = Colors::new();
        colors.observe(&rgba(&[[1, 2, 3, 0xFF]]), 4);
        assert!(colors.observe_color(pack::<3>(&[4, 5, 6])));

        assert_eq!(colors.index_of(pack::<3>(&[1, 2, 3])), Some(0));
        assert_eq!(colors.index_of(pack::<3>(&[4, 5, 6])), Some(1));
        assert_eq!(colors.count(), 2);
    }
}
