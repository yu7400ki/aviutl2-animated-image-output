//! 現在のカラーテーブルで足りなくなったときの据え直し

use crate::layout::Layout;
use crate::normalize::{TRANSPARENT, pack};
use crate::quantize::Histogram;
use crate::table::{Kept, Palette, QUANTIZED_COLORS};

/// 書き出し位置から先の窓を見てカラーテーブルを据え直す
///
/// 維持は現在のテーブルのうち直近 `keep_window` フレームの出力で使ったエントリ。
/// 残差は窓の中で入力が変わり、維持したエントリへ写しても誤差が `tolerance` を
/// 超える画素だけを積んだヒストグラムから割る。変わった画素でも維持したエントリ
/// で足りる色には空きを費やさない。
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
    let window: Vec<&[u8]> = window.collect();
    let mut kept = current.recently_used(keep_window);
    let mut histogram = residual(layout, &kept, tolerance, base, &window);

    // 割り出せる色はヒストグラムが覆うビンの数まで
    let demand = histogram.distinct().min(QUANTIZED_COLORS);
    let free = QUANTIZED_COLORS - kept.len();
    if demand > free {
        // 解いたエントリが覆っていた色は写す先を失うので、残差へ積み直す
        release_oldest(&mut kept, demand - free);
        histogram = residual(layout, &kept, tolerance, base, &window);
    }

    let free = QUANTIZED_COLORS - kept.len();
    let residual = if free > 0 && histogram.distinct() > 0 {
        histogram.quantize(free)
    } else {
        Vec::new()
    };
    Palette::from_rebuilt(&kept, &residual)
}

/// 窓の中で、維持したエントリでは足りない画素を積む
///
/// 入力が変わっていない画素は写し直さないため積まない。透過標識は色を持たない
/// ため積まない。
fn residual<'a>(
    layout: &Layout,
    kept: &[Kept],
    tolerance: u32,
    base: &'a [u8],
    window: &[&'a [u8]],
) -> Histogram {
    let bpp = layout.bytes_per_pixel;
    let mut covered = (!kept.is_empty())
        .then(|| Palette::from_entries(kept.iter().map(|entry| entry.color).collect()));

    let mut histogram = Histogram::new();
    let mut previous = base;
    for &frame in window {
        for (at, pixel) in frame.chunks_exact(bpp).enumerate() {
            let at = at * bpp;
            if !previous.is_empty() && previous[at..at + bpp] == *pixel {
                continue;
            }
            let color = pack(pixel, bpp);
            if color == TRANSPARENT {
                continue;
            }
            if let Some(covered) = &mut covered
                && covered.map(pixel, bpp).error <= tolerance
            {
                continue;
            }
            histogram.observe_color(color, 1);
        }
        previous = frame;
    }
    histogram
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

    /// 維持したエントリで足りる色は残差に積まない
    #[test]
    fn colors_the_kept_entries_already_cover_stay_out_of_the_residual() {
        let kept = kept_of(&[(0xFF00_0000, 1)]);
        let frame = [1u8, 1, 1, 200, 200, 200];

        let histogram = residual(&layout(), &kept, 64, &[], &[&frame[..]]);
        // 黒の近くの画素だけが維持で足り、遠い画素が残差になる
        assert_eq!(histogram.distinct(), 1);
    }

    /// 入力が変わっていない画素は残差に積まない
    #[test]
    fn unchanged_pixels_stay_out_of_the_residual() {
        let base = [1u8, 1, 1, 200, 200, 200];
        let frame = [1u8, 1, 1, 9, 9, 9];

        let histogram = residual(&layout(), &[], 0, &base, &[&frame[..]]);
        assert_eq!(histogram.distinct(), 1);
    }

    /// 窓の中の後続フレームは、その1つ前のフレームとの差分で積む
    #[test]
    fn later_frames_in_the_window_are_compared_with_the_one_before() {
        let first = [1u8, 1, 1, 200, 200, 200];
        let second = [1u8, 1, 1, 100, 100, 100];
        let frames = [&first[..], &second[..]];

        let histogram = residual(&layout(), &[], 0, &[], &frames);
        // 先頭は2色、続くフレームは変わった1色だけ
        assert_eq!(histogram.distinct(), 3);
    }

    /// 透過標識は残差に積まない
    #[test]
    fn the_transparent_marker_stays_out_of_the_residual() {
        let layout = Layout::new(2, 1, ColorType::Rgba8).unwrap();
        let frame = [0u8, 0, 0, 0, 200, 200, 200, 255];

        let histogram = residual(&layout, &[], 0, &[], &[&frame[..]]);
        assert_eq!(histogram.distinct(), 1);
    }
}
