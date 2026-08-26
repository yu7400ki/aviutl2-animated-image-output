//! 現在のカラーテーブルで足りなくなったときの据え直し

use crate::layout::Layout;
use crate::normalize::{TRANSPARENT, pack};
use crate::quantize::{Histogram, Marks};
use crate::table::{Kept, Palette, QUANTIZED_COLORS};

/// 書き出し位置から先の窓を見てカラーテーブルを据え直す
///
/// 維持は現在のテーブルのうち直近 `keep_window` フレームの出力で使ったエントリ、
/// 残差は窓の中で入力が変わった画素のうち維持では誤差が `tolerance` を超えるもの。
///
/// `base` は書き出し位置の1つ前のフレームの正規化した入力で、先頭フレームでは空。
/// `window` は書き出し位置から順に並んだフレーム。
pub(crate) fn rebuild<'a>(
    layout: &Layout,
    current: &Palette,
    keep_window: u32,
    tolerance: u32,
    base: &'a [u8],
    window: impl Iterator<Item = &'a [u8]>,
) -> Palette {
    let mut histogram = changed_colors(layout, base, window);
    let mut kept = current.recently_used(keep_window);
    // 上限を埋めた不透明なテーブルは QUANTIZED_COLORS を超える維持を返す。
    // 以降の空きの引き算が成り立つよう、古い順に解いて収める
    let excess = kept.len().saturating_sub(QUANTIZED_COLORS);
    release_oldest(&mut kept, excess);

    // 割り出せる色はヒストグラムが覆うビンの数まで
    let mut covered = covered_cells(&histogram, &kept, tolerance);
    let demand = (histogram.distinct() - covered.len()).min(QUANTIZED_COLORS);
    let free = QUANTIZED_COLORS - kept.len();
    if demand > free {
        // 解いたエントリが覆っていたビンは残差へ戻る
        release_oldest(&mut kept, demand - free);
        covered = covered_cells(&histogram, &kept, tolerance);
    }
    for &cell in &covered {
        histogram.discard(cell);
    }

    let free = QUANTIZED_COLORS - kept.len();
    let residual = if free > 0 && histogram.distinct() > 0 {
        histogram.quantize(free)
    } else {
        Vec::new()
    };
    Palette::from_rebuilt(&kept, &residual)
}

/// 窓の中で入力が変わった画素の色を積む
///
/// 変わっていない画素は写し直さないため積まない。透過標識は色を持たないため
/// 積まない。
fn changed_colors<'a>(
    layout: &Layout,
    base: &'a [u8],
    window: impl Iterator<Item = &'a [u8]>,
) -> Histogram {
    let bpp = layout.bytes_per_pixel;
    let mut histogram = Histogram::new();
    let mut previous = base;
    for frame in window {
        for (at, pixel) in frame.chunks_exact(bpp).enumerate() {
            let at = at * bpp;
            if !previous.is_empty() && previous[at..at + bpp] == *pixel {
                continue;
            }
            let color = pack(pixel, bpp);
            if color == TRANSPARENT {
                continue;
            }
            histogram.observe_color(color, 1);
        }
        previous = frame;
    }
    histogram
}

/// 維持したエントリへ写せば誤差が `tolerance` に収まるビン
///
/// 写す先はビンごとに決まるので、覆えるかどうかもビンごとに決まる。維持した
/// エントリの周りへ印を付けて拾う方が、ビンごとに写す先を探すより安い。
/// ここで拾ったビンには空きを費やさない。
fn covered_cells(histogram: &Histogram, kept: &[Kept], tolerance: u32) -> Vec<usize> {
    if kept.is_empty() {
        return Vec::new();
    }

    let mut marks = Marks::new();
    for entry in kept {
        marks.mark_within(entry.color, tolerance);
    }
    histogram
        .cells()
        .iter()
        .copied()
        .filter(|&cell| marks.has(cell))
        .collect()
}

/// 最終使用が古い順に `count` 個の維持を解く
///
/// 残った維持は元の並びを保つ。
fn release_oldest(kept: &mut Vec<Kept>, count: usize) {
    let count = count.min(kept.len());
    if count == 0 {
        return;
    }

    let mut order: Vec<usize> = (0..kept.len()).collect();
    order.sort_unstable_by_key(|&index| (kept[index].last_used, index));

    let mut released = vec![false; kept.len()];
    for &index in &order[..count] {
        released[index] = true;
    }
    let survivors = kept
        .iter()
        .zip(&released)
        .filter(|&(_, &released)| !released)
        .map(|(&entry, _)| entry)
        .collect();
    *kept = survivors;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ColorType;
    use anim_core::{Colors, MAX_COLORS};

    fn kept_of(entries: &[(u32, u32)]) -> Vec<Kept> {
        entries
            .iter()
            .map(|&(color, last_used)| Kept { color, last_used })
            .collect()
    }

    fn colors_of(kept: &[Kept]) -> Vec<u32> {
        kept.iter().map(|entry| entry.color).collect()
    }

    /// 解くのは最終使用が古いものからで、残ったものの並びは変わらない
    #[test]
    fn the_least_recently_used_entries_are_released_first() {
        let mut kept = kept_of(&[(10, 5), (11, 1), (12, 9), (13, 3)]);
        release_oldest(&mut kept, 2);
        assert_eq!(colors_of(&kept), [10, 12]);
    }

    /// 維持しているより多くを解こうとしても、空にするまでで止まる
    #[test]
    fn releasing_more_than_kept_empties_the_set() {
        let mut kept = kept_of(&[(10, 5), (11, 1)]);
        release_oldest(&mut kept, 9);
        assert!(kept.is_empty());
    }

    /// 1つも解かないときは何も動かさない
    #[test]
    fn releasing_nothing_leaves_the_set_alone() {
        let mut kept = kept_of(&[(10, 5), (11, 1)]);
        release_oldest(&mut kept, 0);
        assert_eq!(colors_of(&kept), [10, 11]);
    }

    fn layout() -> Layout {
        Layout::new(2, 1, ColorType::Rgb8).unwrap()
    }

    /// 維持したエントリで足りる色は残差から外れる
    #[test]
    fn colors_the_kept_entries_already_cover_are_left_out() {
        let kept = kept_of(&[(0xFF00_0000, 1)]);
        let frame = [1u8, 1, 1, 200, 200, 200];

        let histogram = changed_colors(&layout(), &[], std::iter::once(&frame[..]));
        assert_eq!(histogram.distinct(), 2);
        // 黒の近くの画素だけが維持で足り、遠い画素が残差になる
        assert_eq!(covered_cells(&histogram, &kept, 64).len(), 1);
    }

    /// 維持が空なら、外れるビンは無い
    #[test]
    fn nothing_is_covered_without_kept_entries() {
        let frame = [1u8, 1, 1, 200, 200, 200];
        let histogram = changed_colors(&layout(), &[], std::iter::once(&frame[..]));
        assert!(covered_cells(&histogram, &[], 64).is_empty());
    }

    /// 入力が変わっていない画素は積まない
    #[test]
    fn unchanged_pixels_are_not_counted() {
        let base = [1u8, 1, 1, 200, 200, 200];
        let frame = [1u8, 1, 1, 9, 9, 9];

        let histogram = changed_colors(&layout(), &base, std::iter::once(&frame[..]));
        assert_eq!(histogram.distinct(), 1);
    }

    /// 窓の中の後続フレームは、その1つ前のフレームとの差分で積む
    ///
    /// 比較相手を書き出し位置の1つ前に固定すると、窓の中で元の色へ戻った画素が
    /// 「変わっていない」と読める。
    #[test]
    fn later_frames_in_the_window_are_compared_with_the_one_before() {
        let base = [1u8, 1, 1, 10, 10, 10];
        let first = [1u8, 1, 1, 200, 200, 200];
        let second = base;
        let frames = [&first[..], &second[..]];

        let histogram = changed_colors(&layout(), &base, frames.into_iter());
        // 先頭で変わった色と、次のフレームで戻った色
        assert_eq!(histogram.distinct(), 2);
    }

    /// 透過標識は積まない
    #[test]
    fn the_transparent_marker_is_not_counted() {
        let layout = Layout::new(2, 1, ColorType::Rgba8).unwrap();
        let frame = [0u8, 0, 0, 0, 200, 200, 200, 255];

        let histogram = changed_colors(&layout, &[], std::iter::once(&frame[..]));
        assert_eq!(histogram.distinct(), 1);
    }

    /// 上限を埋めた不透明なテーブルからでも、維持は空きの数に収まる
    ///
    /// 収まらないと空きの引き算が桁あふれし、あふれた数がそのまま量子化の
    /// 目標色数になる。
    #[test]
    fn the_kept_entries_are_capped_at_the_slots_of_a_rebuilt_table() {
        let pixels: Vec<u8> = (0..MAX_COLORS)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 0])
            .collect();
        let mut colors = Colors::new();
        colors.observe(&pixels, 3);
        let mut current = Palette::from_colors(colors, false);
        assert_eq!(current.transparent(), None, "透過スロットが取れている");

        current.set_frame(1);
        for index in 0..MAX_COLORS {
            current.mark_used(index as u8);
        }
        assert_eq!(current.recently_used(8).len(), MAX_COLORS);

        let frame = [1u8, 1, 1, 200, 200, 200];
        let rebuilt = rebuild(&layout(), &current, 8, 64, &[], std::iter::once(&frame[..]));
        assert!(rebuilt.colors() as usize <= QUANTIZED_COLORS);
    }

    /// 維持で足りる色には空きを費やさない
    #[test]
    fn the_residual_leaves_out_the_colors_the_kept_entries_cover() {
        let mut colors = Colors::new();
        colors.observe(&[0u8, 0, 0], 3);
        let mut current = Palette::from_colors(colors, false);
        current.set_frame(1);
        current.mark_used(0);

        // 黒の近くの画素は維持で足り、遠い画素だけが残差になる
        let frame = [1u8, 1, 1, 200, 200, 200];
        let rebuilt = rebuild(&layout(), &current, 8, 64, &[], std::iter::once(&frame[..]));
        assert_eq!(rebuilt.colors(), 2, "維持と重なる色に空きを費やしている");
    }

    /// 残差はテーブルの空きの数だけ量子化する
    #[test]
    fn the_residual_is_quantized_into_exactly_the_free_slots() {
        const COLORS: u32 = 300;
        let layout = Layout::new(COLORS, 1, ColorType::Rgb8).unwrap();
        let frame: Vec<u8> = (0..COLORS)
            .flat_map(|i| [(i % 60) as u8 * 4, (i / 60) as u8 * 4, 0])
            .collect();

        // 維持が空なので、空きはテーブルの非透過スロットすべて
        let mut colors = Colors::new();
        colors.observe(&[0u8, 0, 0], 3);
        let current = Palette::from_colors(colors, false);
        assert!(current.recently_used(8).is_empty());

        let expected =
            changed_colors(&layout, &[], std::iter::once(&frame[..])).quantize(QUANTIZED_COLORS);
        assert_eq!(expected.len(), QUANTIZED_COLORS);

        let rebuilt = rebuild(&layout, &current, 8, 64, &[], std::iter::once(&frame[..]));
        let actual: Vec<u32> = (0..rebuilt.colors())
            .map(|index| rebuilt.color_at(index as u8))
            .collect();
        assert_eq!(actual, expected, "空きの数と残差の色数が食い違っている");
    }

    /// 外したビンを捨てると、残差の数がそのぶん減る
    #[test]
    fn discarding_a_covered_bin_shrinks_the_residual() {
        let frame = [1u8, 1, 1, 200, 200, 200];
        let mut histogram = changed_colors(&layout(), &[], std::iter::once(&frame[..]));
        let covered = covered_cells(&histogram, &kept_of(&[(0xFF00_0000, 1)]), 64);

        for cell in covered {
            histogram.discard(cell);
        }
        assert_eq!(histogram.distinct(), 1);
        assert_eq!(histogram.quantize(8), vec![0xFFC8_C8C8]);
    }
}
