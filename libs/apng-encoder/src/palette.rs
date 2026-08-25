//! 明るさで並べたパレットの組み立て

use anim_core::{Colors, Indexed};

/// 色の明るさの目安
///
/// 緑を重く青を軽く見る整数の重み付けで、絶対値ではなく色どうしの前後だけを使う。
fn luminance(color: u32) -> u32 {
    let (r, g, b) = (color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF);
    r * 2 + g * 5 + b
}

/// 数えた色をパレットへ落とす
pub(crate) trait ColorsExt {
    /// 数えた色を並べてパレットにする
    ///
    /// 明るさの順に置く。隣り合う画素の色が近いほど添字も数として近くなるため、
    /// 行ごとの適応フィルタが添字の面でも効く。明るさが同じ色は見つけた順に残る。
    ///
    /// # Panics
    /// 色数が上限を超えているとき。
    fn into_palette(self) -> Palette;
}

impl ColorsExt for Colors {
    fn into_palette(self) -> Palette {
        Palette {
            indexed: self.into_indexed(luminance),
        }
    }
}

/// PLTEとtRNSへ落とせるパレット
pub(crate) struct Palette {
    indexed: Indexed,
}

impl Palette {
    /// PLTEチャンクのデータ部
    ///
    /// 添字順に3バイトのR,G,Bを並べたもの。
    pub(crate) fn plte(&self) -> Vec<u8> {
        self.indexed
            .colors()
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
        let colors = self.indexed.colors();
        let opaque = colors
            .iter()
            .rev()
            .take_while(|&&color| color >> 24 == u32::from(u8::MAX))
            .count();

        colors[..colors.len() - opaque]
            .iter()
            .map(|&color| (color >> 24) as u8)
            .collect()
    }

    /// 画素列を添字へ写して `out` へ追記する
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並び、その色がすべてこのパレットに
    /// 含まれていること。`bpp` は3か4であること。
    pub(crate) fn append_indices(&self, pixels: &[u8], bpp: usize, out: &mut Vec<u8>) {
        self.indexed.append_indices(pixels, bpp, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::MAX_COLORS;

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

    /// 1画素だけの入力は1エントリのパレットになる
    #[test]
    fn a_single_pixel_yields_a_one_entry_palette() {
        let palette = palette_of(&[1, 2, 3, 0x80], 4);
        assert_eq!(palette.plte(), [1, 2, 3]);
        assert_eq!(palette.trns(), [0x80]);
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

    /// 表の末尾で衝突した色も、それぞれの添字でパレットを引ける
    ///
    /// この2色は開放アドレス法の表で同じ位置を指し、2色目は表の端を越えて
    /// 先頭から空きを探すことになる。
    #[test]
    fn colors_colliding_at_the_last_slot_keep_distinct_indices() {
        /// 表の最後の位置へ写る色 (詰めると `0x0000_03DB`)
        const FIRST: [u8; 4] = [0xDB, 0x03, 0x00, 0x00];
        /// 同じ位置へ写るもう1つの色 (詰めると `0x0000_07B6`)
        const SECOND: [u8; 4] = [0xB6, 0x07, 0x00, 0x00];

        let pixels = rgba(&[FIRST, SECOND]);
        let palette = palette_of(&pixels, 4);
        let plte = palette.plte();
        assert_eq!(plte.len() / 3, 2);

        let mut indices = Vec::new();
        palette.append_indices(&pixels, 4, &mut indices);
        assert_ne!(indices[0], indices[1]);
        for (index, pixel) in indices.iter().zip(pixels.chunks_exact(4)) {
            let at = *index as usize;
            assert_eq!(&plte[at * 3..at * 3 + 3], &pixel[..3]);
        }
    }

    /// 添字は既にある内容の後ろへ足される
    #[test]
    fn indices_are_appended_after_the_existing_content() {
        let palette = palette_of(&[1, 2, 3], 3);
        let mut out = vec![0xAA];
        palette.append_indices(&[1, 2, 3], 3, &mut out);
        assert_eq!(out, [0xAA, 0]);
    }

    /// 暗い色ほど前に並ぶ
    #[test]
    fn darker_colors_come_first() {
        let palette = palette_of(&[0x30, 0, 0, 0x10, 0, 0, 0x20, 0, 0], 3);
        assert_eq!(palette.plte(), [0x10, 0, 0, 0x20, 0, 0, 0x30, 0, 0]);
    }

    /// 明るさが同じ色は見つけた順に並ぶ
    #[test]
    fn colors_of_the_same_luminance_keep_their_order() {
        // 重み付けは (2, 5, 1) なので、この3色の明るさは等しい
        let palette = palette_of(&[5, 0, 0, 0, 2, 0, 0, 0, 10], 3);
        assert_eq!(palette.plte(), [5, 0, 0, 0, 2, 0, 0, 0, 10]);
    }

    /// アルファは並べ替えた後の添字と対応する
    #[test]
    fn the_trns_follows_the_palette_order() {
        let pixels = rgba(&[
            [0x30, 0x30, 0x30, 0x80],
            [0x10, 0x10, 0x10, 0xFF],
            [0x20, 0x20, 0x20, 0x00],
        ]);
        let palette = palette_of(&pixels, 4);

        assert_eq!(
            palette.plte(),
            [0x10, 0x10, 0x10, 0x20, 0x20, 0x20, 0x30, 0x30, 0x30]
        );
        assert_eq!(palette.trns(), [0xFF, 0x00, 0x80]);
    }

    /// tRNSの末尾に並ぶ255は省かれる
    #[test]
    fn the_trailing_opaque_entries_are_dropped_from_the_trns() {
        let pixels = rgba(&[
            [0x10, 0x10, 0x10, 0x00],
            [0x40, 0x40, 0x40, 0xFF],
            [0x80, 0x80, 0x80, 0xFF],
        ]);
        let palette = palette_of(&pixels, 4);

        assert_eq!(palette.trns(), [0x00]);
    }

    /// すべて不透明ならtRNSは空になる
    #[test]
    fn an_opaque_palette_needs_no_trns() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0xFF]]), 4);
        assert!(palette.trns().is_empty());
    }
}
