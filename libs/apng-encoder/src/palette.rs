//! 見つけた順に添字を振る色表と、後から書き戻すPLTE・tRNSの置き場

use anim_core::{Colors, MAX_COLORS};

/// PLTEのデータ部の長さ
///
/// 添字を振り終える前にフレームを書き出すため、色数が決まる前に場所を確保する。
/// エントリ数は上限いっぱいで固定し、載せた色の後ろは詰め物になる。
pub(crate) const PLTE_LEN: usize = MAX_COLORS * 3;

/// tRNSのデータ部の長さ
///
/// 理由は [`PLTE_LEN`] と同じ。
pub(crate) const TRNS_LEN: usize = MAX_COLORS;

/// PLTEとtRNSを引く添字の表
///
/// 添字は色を見つけた順に振る。全フレームを見終わる前に添字を焼くため、
/// 並べ替えはできない。
pub(crate) struct Palette {
    colors: Colors,
    /// 確保したPLTEのファイル上の位置
    plte: u64,
    /// 確保したtRNSのファイル上の位置
    ///
    /// アルファを持たない入力にはアルファが現れないため、tRNS自体を書かない。
    trns: Option<u64>,
}

impl Palette {
    /// 先頭フレームで数えた色から始める
    pub(crate) fn new(colors: Colors, plte: u64, trns: Option<u64>) -> Self {
        Palette { colors, plte, trns }
    }

    /// 添字を振った色数
    pub(crate) fn colors(&self) -> u16 {
        self.colors.count()
    }

    /// 確保したPLTEのファイル上の位置
    pub(crate) fn plte_at(&self) -> u64 {
        self.plte
    }

    /// 確保したtRNSのファイル上の位置
    pub(crate) fn trns_at(&self) -> Option<u64> {
        self.trns
    }

    /// 完全に透明な色の添字。まだ現れていなければ `None`
    ///
    /// この添字はアルファが0なので、blend_op=OVERで重ねるとキャンバスが残る。
    pub(crate) fn transparent(&self) -> Option<u8> {
        let index = self.colors.colors().position(|color| color >> 24 == 0)?;
        Some(index as u8)
    }

    /// 画素の色の添字。まだ振っていなければ `None`
    ///
    /// 引数の条件は [`Colors::index_of_pixel`] と同じ。
    pub(crate) fn index_of(&self, pixel: &[u8], bpp: usize) -> Option<u8> {
        self.colors.index_of_pixel(pixel, bpp)
    }

    /// 画素列を添字へ写して `out` へ追記する
    ///
    /// 引数の条件は [`Colors::append_indices`] と同じ。載せられる色数を超えたら
    /// `out` を呼び出し前の長さへ戻して偽を返す。
    pub(crate) fn append_indices(&mut self, pixels: &[u8], bpp: usize, out: &mut Vec<u8>) -> bool {
        self.colors.append_indices(pixels, bpp, out)
    }

    /// PLTEのデータ部
    ///
    /// 添字順に3バイトのR,G,Bを並べ、載せた色の後ろは0で埋める。
    pub(crate) fn plte(&self) -> [u8; PLTE_LEN] {
        let mut plte = [0u8; PLTE_LEN];
        for (entry, color) in plte.chunks_exact_mut(3).zip(self.colors.colors()) {
            entry.copy_from_slice(&[color as u8, (color >> 8) as u8, (color >> 16) as u8]);
        }
        plte
    }

    /// tRNSのデータ部
    ///
    /// 添字順に1バイトのアルファを並べ、載せた色の後ろは不透明で埋める。
    pub(crate) fn trns(&self) -> [u8; TRNS_LEN] {
        let mut trns = [u8::MAX; TRNS_LEN];
        for (entry, color) in trns.iter_mut().zip(self.colors.colors()) {
            *entry = (color >> 24) as u8;
        }
        trns
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

    /// 先頭フレームを数えた表を、位置を持たせずに作る
    fn palette_of(pixels: &[u8], bpp: usize) -> Palette {
        let mut colors = Colors::new();
        colors.observe(pixels, bpp);
        Palette::new(colors, 0, Some(0))
    }

    /// 添字は色を見つけた順に振られる
    #[test]
    fn indices_follow_the_order_the_colors_were_found() {
        let mut palette = palette_of(&[0x30, 0, 0], 3);
        let mut indices = Vec::new();
        assert!(palette.append_indices(&[0x30, 0, 0, 0x10, 0, 0, 0x20, 0, 0], 3, &mut indices));

        assert_eq!(indices, [0, 1, 2]);
        assert_eq!(palette.plte()[..9], [0x30, 0, 0, 0x10, 0, 0, 0x20, 0, 0]);
    }

    /// 同じ色は同じ添字へ写る
    #[test]
    fn a_repeated_color_keeps_its_index() {
        let pixels = rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0x80], [1, 2, 3, 0xFF]]);
        let mut palette = palette_of(&pixels, 4);

        let mut indices = Vec::new();
        assert!(palette.append_indices(&pixels, 4, &mut indices));
        assert_eq!(indices, [0, 1, 0]);
        assert_eq!(palette.colors(), 2);
    }

    /// アルファだけが違う色は別の添字になる
    #[test]
    fn colors_differing_only_in_alpha_are_distinct() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [1, 2, 3, 0x80]]), 4);
        assert_eq!(palette.colors(), 2);
        assert_eq!(palette.trns()[..2], [0xFF, 0x80]);
    }

    /// PLTEとtRNSは長さが固定で、載せた色の後ろは詰め物になる
    #[test]
    fn the_chunks_are_padded_to_a_fixed_length() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0x40]]), 4);

        let plte = palette.plte();
        assert_eq!(plte.len(), PLTE_LEN);
        assert_eq!(plte[..3], [1, 2, 3]);
        assert!(plte[3..].iter().all(|&b| b == 0));

        let trns = palette.trns();
        assert_eq!(trns.len(), TRNS_LEN);
        assert_eq!(trns[0], 0x40);
        assert!(trns[1..].iter().all(|&b| b == u8::MAX));
    }

    /// RGB8の画素はアルファ255の色として数える
    #[test]
    fn rgb_pixels_are_counted_as_opaque_colors() {
        let palette = palette_of(&[1, 2, 3, 1, 2, 3, 4, 5, 6], 3);
        assert_eq!(palette.colors(), 2);
        assert!(palette.trns().iter().all(|&a| a == u8::MAX));
        assert_eq!(palette.transparent(), None);
    }

    /// 完全に透明な色があれば、その添字が引ける
    #[test]
    fn a_fully_transparent_color_is_found_by_its_index() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0x00]]), 4);
        assert_eq!(palette.transparent(), Some(1));

        // 半透明はキャンバスと混ざるため、重ねる先を残せない
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0x01]]), 4);
        assert_eq!(palette.transparent(), None);
    }

    /// 画素から添字を引ける。振っていない色は `None`
    #[test]
    fn a_pixel_is_looked_up_by_its_bytes() {
        let palette = palette_of(&rgba(&[[1, 2, 3, 0xFF], [4, 5, 6, 0xFF]]), 4);

        assert_eq!(palette.index_of(&[1, 2, 3, 0xFF], 4), Some(0));
        assert_eq!(palette.index_of(&[4, 5, 6], 3), Some(1));
        assert_eq!(palette.index_of(&[7, 8, 9], 3), None);
    }

    /// ちょうど上限までの色は写せて、1つ超えると写せない
    #[test]
    fn the_limit_is_reached_before_it_is_exceeded() {
        let full = distinct_rgb(MAX_COLORS);
        let mut palette = palette_of(&full, 3);
        let mut indices = Vec::new();
        assert!(palette.append_indices(&full, 3, &mut indices));
        assert_eq!(palette.colors(), MAX_COLORS as u16);

        let over = distinct_rgb(MAX_COLORS + 1);
        let mut palette = palette_of(&[0, 0, 0], 3);
        let mut indices = Vec::new();
        assert!(!palette.append_indices(&over, 3, &mut indices));
        assert!(indices.is_empty(), "写せない画素列の添字が残っている");
    }
}
