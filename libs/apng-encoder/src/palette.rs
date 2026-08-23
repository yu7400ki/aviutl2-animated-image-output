//! 色の和集合の集計とパレットの組み立て

/// パレットに収められる色数の上限
pub(crate) const MAX_COLORS: usize = 256;

/// 表の添字に使うビット数
const TABLE_BITS: u32 = 10;
/// 開放アドレス法の表の大きさ
///
/// [`MAX_COLORS`] より十分に大きく取るため、表には必ず空きが残り、探索は必ず止まる。
const TABLE_LEN: usize = 1 << TABLE_BITS;
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
    /// この色だった画素の数
    count: u64,
}

/// 色から添字を引く表
///
/// [`Self::values`] が0の位置は空で、それ以外はパレットの添字に1を足した値が入る。
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
}

/// 全フレームに現れた色の和集合
///
/// 見つけた色が [`MAX_COLORS`] を超えた時点で走査をやめ、以降は何も数えない。
pub(crate) struct Colors {
    table: Table,
    /// 見つけた順の色
    entries: Vec<Entry>,
    /// 上限を超えたか
    exceeded: bool,
}

impl Colors {
    pub(crate) fn new() -> Self {
        Colors {
            table: Table::new(),
            entries: Vec::new(),
            exceeded: false,
        }
    }

    /// 色数が上限を超えたか
    ///
    /// 一度真になったら戻らないため、以降の走査は要らない。
    pub(crate) fn exceeded(&self) -> bool {
        self.exceeded
    }

    /// 画素列に現れる色を数える
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並んでいること。`bpp` は3か4であること。
    pub(crate) fn observe(&mut self, pixels: &[u8], bpp: usize) {
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
            if !self.count(pack::<BPP>(pixel)) {
                self.exceeded = true;
                return;
            }
        }
    }

    /// 色を1つ数える。上限を超えて入らなければ偽を返す
    fn count(&mut self, color: u32) -> bool {
        let mut slot = slot_of(color);
        loop {
            let value = self.table.values[slot];
            if value == 0 {
                if self.entries.len() == MAX_COLORS {
                    return false;
                }
                self.table.keys[slot] = color;
                self.table.values[slot] = self.entries.len() as u16 + 1;
                self.entries.push(Entry {
                    color,
                    slot,
                    count: 1,
                });
                return true;
            }
            if self.table.keys[slot] == color {
                self.entries[value as usize - 1].count += 1;
                return true;
            }
            slot = (slot + 1) & TABLE_MASK;
        }
    }

    /// 数えた色を並べてパレットにする
    ///
    /// 透過する色を前へ、その中では画素の多い色を前へ置く。前者はtRNSの末尾を
    /// 省ける長さを伸ばし、後者は符号長の短い添字を若い値へ寄せる。並びが同じ色は
    /// 見つけた順に残る。
    ///
    /// # Panics
    /// 色数が上限を超えているとき。
    pub(crate) fn into_palette(mut self) -> Palette {
        assert!(!self.exceeded, "色数が上限を超えている");

        self.entries.sort_by_key(|entry| {
            (
                entry.color >> 24 == u32::from(u8::MAX),
                std::cmp::Reverse(entry.count),
            )
        });

        let mut table = self.table;
        for (index, entry) in self.entries.iter().enumerate() {
            table.values[entry.slot] = index as u16 + 1;
        }

        Palette {
            colors: self.entries.iter().map(|entry| entry.color).collect(),
            table,
        }
    }
}

/// 添字と色の対応
pub(crate) struct Palette {
    /// 添字順に並べた色
    colors: Vec<u32>,
    table: Table,
}

impl Palette {
    /// PLTEチャンクのデータ部
    ///
    /// 添字順に3バイトのR,G,Bを並べたもの。
    pub(crate) fn plte(&self) -> Vec<u8> {
        self.colors
            .iter()
            .flat_map(|&color| [color as u8, (color >> 8) as u8, (color >> 16) as u8])
            .collect()
    }

    /// tRNSチャンクのデータ部
    ///
    /// 添字順に1バイトのアルファを並べたもの。エントリ数がパレットに満たない
    /// ぶんは255とみなされるため、末尾の255は省く。すべて不透明なら空になり、
    /// この場合はチャンク自体が要らない。
    pub(crate) fn trns(&self) -> Vec<u8> {
        let opaque = self
            .colors
            .iter()
            .rev()
            .take_while(|&&color| color >> 24 == u32::from(u8::MAX))
            .count();

        self.colors[..self.colors.len() - opaque]
            .iter()
            .map(|&color| (color >> 24) as u8)
            .collect()
    }

    /// 画素列を添字へ写して `out` へ追記する
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並び、その色がすべてこのパレットに
    /// 含まれていること。`bpp` は3か4であること。
    pub(crate) fn append_indices(&self, pixels: &[u8], bpp: usize, out: &mut Vec<u8>) {
        out.reserve(pixels.len() / bpp);
        match bpp {
            3 => self.map::<3>(pixels, out),
            4 => self.map::<4>(pixels, out),
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        }
    }

    fn map<const BPP: usize>(&self, pixels: &[u8], out: &mut Vec<u8>) {
        for pixel in pixels.chunks_exact(BPP) {
            out.push(self.index_of(pack::<BPP>(pixel)));
        }
    }

    /// 色の添字
    ///
    /// `color` がこのパレットに含まれていること。
    fn index_of(&self, color: u32) -> u8 {
        let mut slot = slot_of(color);
        loop {
            let value = self.table.values[slot];
            debug_assert_ne!(value, 0, "パレットに無い色");
            if self.table.keys[slot] == color {
                return (value - 1) as u8;
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

    fn palette_of(pixels: &[u8], bpp: usize) -> Palette {
        let mut colors = Colors::new();
        colors.observe(pixels, bpp);
        colors.into_palette()
    }

    #[test]
    fn an_empty_input_yields_an_empty_palette() {
        let palette = palette_of(&[], 4);
        assert_eq!(palette.plte().len() / 3, 0);
        assert!(palette.plte().is_empty());
        assert!(palette.trns().is_empty());
    }

    /// 同じ色の繰り返しは1つにまとまる
    #[test]
    fn repeated_colors_collapse_into_one_entry() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF]; 16]), 4);
        assert_eq!(palette.plte().len() / 3, 1);
        assert_eq!(palette.plte(), [1, 2, 3]);
    }

    /// アルファだけが違う色は別の色として数える
    #[test]
    fn colors_differing_only_in_alpha_are_distinct() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [1, 2, 3, 0x80]]), 4);
        assert_eq!(palette.plte().len() / 3, 2);
    }

    /// RGB8の画素はアルファ255の色として数える
    #[test]
    fn rgb_pixels_are_counted_as_opaque_colors() {
        let palette = palette_of(&[1, 2, 3, 1, 2, 3, 4, 5, 6], 3);
        assert_eq!(palette.plte().len() / 3, 2);
        assert!(palette.trns().is_empty());
    }

    /// ちょうど上限までの色は数え切り、1つ超えると打ち切る
    #[test]
    fn the_limit_is_reached_before_it_is_exceeded() {
        let mut colors = Colors::new();
        colors.observe(&distinct_rgb(MAX_COLORS), 3);
        assert!(!colors.exceeded());
        assert_eq!(colors.into_palette().plte().len() / 3, MAX_COLORS);

        let mut colors = Colors::new();
        colors.observe(&distinct_rgb(MAX_COLORS + 1), 3);
        assert!(colors.exceeded());
    }

    /// 上限を超えるのは呼び出しをまたいでも同じで、超えた後は何も数えない
    #[test]
    fn the_union_spans_every_call() {
        let mut colors = Colors::new();
        for chunk in distinct_rgb(MAX_COLORS + 1).chunks(3 * 8) {
            colors.observe(chunk, 3);
        }
        assert!(colors.exceeded());

        let mut colors = Colors::new();
        colors.observe(&distinct_rgb(MAX_COLORS), 3);
        colors.observe(&distinct_rgb(MAX_COLORS), 3);
        assert!(!colors.exceeded());
    }

    /// 添字は画素の色と一対一に対応する
    #[test]
    fn indices_map_back_to_the_colors_they_came_from() {
        let pixels = rgba(&[
            [0x10, 0x20, 0x30, 0xFF],
            [0x40, 0x50, 0x60, 0x80],
            [0x10, 0x20, 0x30, 0xFF],
            [0x00, 0x00, 0x00, 0x00],
        ]);
        let palette = palette_of(&pixels, 4);

        let mut indices = Vec::new();
        palette.append_indices(&pixels, 4, &mut indices);
        assert_eq!(indices.len(), 4);
        assert_eq!(indices[0], indices[2]);

        let plte = palette.plte();
        let trns = palette.trns();
        for (index, pixel) in indices.iter().zip(pixels.chunks_exact(4)) {
            let at = *index as usize;
            assert_eq!(&plte[at * 3..at * 3 + 3], &pixel[..3]);
            let alpha = trns.get(at).copied().unwrap_or(u8::MAX);
            assert_eq!(alpha, pixel[3]);
        }
    }

    /// 上限いっぱいの色でも添字は一対一に対応する
    #[test]
    fn a_full_palette_still_maps_one_to_one() {
        let pixels = distinct_rgb(MAX_COLORS);
        let palette = palette_of(&pixels, 3);

        let mut indices = Vec::new();
        palette.append_indices(&pixels, 3, &mut indices);
        indices.sort_unstable();
        indices.dedup();
        assert_eq!(indices.len(), MAX_COLORS);
    }

    /// 添字は既にある内容の後ろへ足される
    #[test]
    fn indices_are_appended_after_the_existing_content() {
        let palette = palette_of(&[1, 2, 3], 3);
        let mut out = vec![0xAA];
        palette.append_indices(&[1, 2, 3], 3, &mut out);
        assert_eq!(out, [0xAA, 0]);
    }

    /// 透過する色が前に並び、tRNSの末尾の255が省かれる
    #[test]
    fn transparent_colors_come_first_and_shorten_the_trns() {
        let pixels = rgba(&[
            [0x10, 0x10, 0x10, 0xFF],
            [0x20, 0x20, 0x20, 0x00],
            [0x30, 0x30, 0x30, 0x80],
        ]);
        let palette = palette_of(&pixels, 4);

        assert_eq!(
            palette.plte(),
            [0x20, 0x20, 0x20, 0x30, 0x30, 0x30, 0x10, 0x10, 0x10]
        );
        assert_eq!(palette.trns(), [0x00, 0x80]);
    }

    /// 画素の多い色ほど前に並ぶ
    #[test]
    fn frequent_colors_come_first() {
        let mut pixels = rgba(&[[0x10, 0x10, 0x10, 0xFF]]);
        pixels.extend(rgba(&[[0x20, 0x20, 0x20, 0xFF]; 3]));
        pixels.extend(rgba(&[[0x30, 0x30, 0x30, 0xFF]; 2]));

        let palette = palette_of(&pixels, 4);
        assert_eq!(
            palette.plte(),
            [0x20, 0x20, 0x20, 0x30, 0x30, 0x30, 0x10, 0x10, 0x10]
        );
    }

    /// 画素の数が同じ色は見つけた順に並ぶ
    #[test]
    fn colors_seen_the_same_number_of_times_keep_their_order() {
        let palette = palette_of(&[0x30, 0, 0, 0x10, 0, 0, 0x20, 0, 0], 3);
        assert_eq!(palette.plte(), [0x30, 0, 0, 0x10, 0, 0, 0x20, 0, 0]);
    }

    /// すべて不透明ならtRNSは空になる
    #[test]
    fn an_opaque_palette_needs_no_trns() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0xFF]]), 4);
        assert!(palette.trns().is_empty());
    }
}
