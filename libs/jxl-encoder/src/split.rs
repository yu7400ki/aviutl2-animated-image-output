//! 書き直す画素を覆う矩形を、書かずに済む画素が固定費を上回るときに割る

use crate::layout::Layout;
use anim_core::{ColorType, Profile, Rect, Span, unchanged_run};

/// 矩形を割るのに要る、書かずに済む画素数の下限
///
/// 増えるフレーム1枚の固定費を、書かずに済ませた画素1つが返すバイト数で割った値。
pub(crate) const THRESHOLD: u64 = 512;

/// `bounds` の中で書き直す画素を、重ならない2つの矩形へ分けて覆う
///
/// 書かずに済む画素が `threshold` に満たなければ `None`。返す2つは合わせて
/// 書き直す画素をすべて覆い、先の1枚が表示時間0の副フレームになる。
///
/// `profile` は `bounds` の中で書き直す画素の分布を引くもので、割る余地のある
/// 面積のときだけ呼ばれる。
pub(crate) fn cut(
    bounds: Rect,
    threshold: u64,
    profile: impl FnOnce() -> Profile,
) -> Option<(Rect, Rect)> {
    if bounds.area() <= threshold {
        return None;
    }
    let best = best_cut(&profile(), bounds)?;
    (best.removed >= threshold).then_some((best.head, best.tail))
}

/// `rect` の中で `previous` と `frame` が食い違う画素の分布
pub(crate) fn exact_profile(previous: &[u8], frame: &[u8], layout: &Layout, rect: Rect) -> Profile {
    match layout.color_type {
        ColorType::Rgb8 => scan::<3>(previous, frame, layout, rect),
        ColorType::Rgba8 => scan::<4>(previous, frame, layout, rect),
    }
}

/// `value` だけを含む範囲
fn at(value: u32) -> Span {
    Span {
        min: value,
        max: value,
    }
}

/// 両方を含むまで広げた範囲
fn union(span: Option<Span>, other: Span) -> Span {
    match span {
        Some(span) => Span {
            min: span.min.min(other.min),
            max: span.max.max(other.max),
        },
        None => other,
    }
}

fn scan<const BPP: usize>(previous: &[u8], frame: &[u8], layout: &Layout, rect: Rect) -> Profile {
    let mut rows: Vec<Option<Span>> = vec![None; rect.height as usize];
    let mut cols: Vec<Option<Span>> = vec![None; rect.width as usize];
    let row_len = rect.width as usize * BPP;

    for (row, span) in rows.iter_mut().enumerate() {
        let start = (rect.y as usize + row) * layout.stride + rect.x as usize * BPP;
        let previous = &previous[start..start + row_len];
        let frame = &frame[start..start + row_len];
        let y = at(row as u32);

        let mut cursor = 0;
        while cursor < row_len {
            let differs = unchanged_run::<BPP>(previous, frame, cursor);
            if differs + BPP > row_len {
                break;
            }
            let column = differs / BPP;
            *span = Some(union(*span, at(column as u32)));
            cols[column] = Some(union(cols[column], y));
            cursor = differs + BPP;
        }
    }

    Profile { rows, cols }
}

/// 帯の並びと、そこで書き直す画素が横断方向に占める範囲
#[derive(Debug, Clone, Copy)]
struct Slab {
    lane: Span,
    cross: Span,
}

impl Slab {
    fn area(self) -> u64 {
        u64::from(self.lane.count()) * u64::from(self.cross.count())
    }

    /// `lane` の帯を足した範囲
    fn extend(acc: Option<Slab>, lane: u32, cross: Option<Span>) -> Option<Slab> {
        let Some(cross) = cross else {
            return acc;
        };
        Some(match acc {
            Some(acc) => Slab {
                lane: union(Some(acc.lane), at(lane)),
                cross: union(Some(acc.cross), cross),
            },
            None => Slab {
                lane: at(lane),
                cross,
            },
        })
    }
}

/// 矩形を1本の直線で割った結果
#[derive(Debug, Clone, Copy)]
struct Cut {
    head: Rect,
    tail: Rect,
    /// 割ることで書かずに済む画素数
    removed: u64,
}

/// 最も多くの画素を書かずに済ませる割り方
///
/// どの位置で割っても書く画素が減らないときは `None`。
fn best_cut(profile: &Profile, rect: Rect) -> Option<Cut> {
    let total = rect.area();
    let horizontal = best_lane_cut(&profile.rows, total).map(|cut| Cut {
        head: rows_rect(rect, cut.head),
        tail: rows_rect(rect, cut.tail),
        removed: cut.removed,
    });
    let vertical = best_lane_cut(&profile.cols, total).map(|cut| Cut {
        head: cols_rect(rect, cut.head),
        tail: cols_rect(rect, cut.tail),
        removed: cut.removed,
    });

    [horizontal, vertical]
        .into_iter()
        .flatten()
        .filter(|cut| cut.removed > 0)
        .max_by_key(|cut| cut.removed)
}

/// 帯の並びを1本の直線で割った結果
struct LaneCut {
    head: Slab,
    tail: Slab,
    removed: u64,
}

/// 帯の並びを1本の直線で割ったとき、最も多くの画素を書かずに済ませる割り方
///
/// `lanes[i]` は帯 `i` で書き直す画素が横断方向に占める範囲。`total` は
/// 割る前の矩形の面積。
fn best_lane_cut(lanes: &[Option<Span>], total: u64) -> Option<LaneCut> {
    let mut suffix = vec![None; lanes.len() + 1];
    for (lane, cross) in lanes.iter().enumerate().rev() {
        suffix[lane] = Slab::extend(suffix[lane + 1], lane as u32, *cross);
    }

    let mut head = None;
    let mut best: Option<LaneCut> = None;
    for (lane, cross) in lanes.iter().enumerate() {
        head = Slab::extend(head, lane as u32, *cross);
        let (Some(head), Some(tail)) = (head, suffix[lane + 1]) else {
            continue;
        };
        let removed = total - head.area() - tail.area();
        if best.as_ref().is_none_or(|best| removed > best.removed) {
            best = Some(LaneCut {
                head,
                tail,
                removed,
            });
        }
    }
    best
}

/// 行を帯にした `slab` が覆う矩形
fn rows_rect(rect: Rect, slab: Slab) -> Rect {
    Rect {
        x: rect.x + slab.cross.min,
        y: rect.y + slab.lane.min,
        width: slab.cross.count(),
        height: slab.lane.count(),
    }
}

/// 列を帯にした `slab` が覆う矩形
fn cols_rect(rect: Rect, slab: Slab) -> Rect {
    Rect {
        x: rect.x + slab.lane.min,
        y: rect.y + slab.cross.min,
        width: slab.lane.count(),
        height: slab.cross.count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::dirty_rect;

    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 48;

    fn layout(color_type: ColorType) -> Layout {
        Layout::new(WIDTH, HEIGHT, color_type).unwrap()
    }

    /// `blocks` の範囲を塗り替えたフレームと、塗り替える前のフレーム
    fn painted(color_type: ColorType, blocks: &[(u32, u32, u32, u32)]) -> (Vec<u8>, Vec<u8>) {
        let bpp = color_type.bytes_per_pixel();
        let previous = vec![0x20; (WIDTH * HEIGHT) as usize * bpp];
        let mut frame = previous.clone();
        for (x, y, width, height) in blocks.iter().copied() {
            for row in 0..height {
                let start = ((y + row) * WIDTH + x) as usize * bpp;
                frame[start..start + width as usize * bpp].fill(0xC0);
            }
        }
        (previous, frame)
    }

    /// `blocks` を塗り替えたフレームの外接矩形を `threshold` で割る
    fn cut_of(
        color_type: ColorType,
        blocks: &[(u32, u32, u32, u32)],
        threshold: u64,
    ) -> (Rect, Option<(Rect, Rect)>) {
        let layout = layout(color_type);
        let (previous, frame) = painted(color_type, blocks);
        let bounds = dirty_rect(&previous, &frame, layout.stride, layout.bytes_per_pixel)
            .expect("変化が無い");
        let cut = cut(bounds, threshold, || {
            exact_profile(&previous, &frame, &layout, bounds)
        });
        (bounds, cut)
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// 離れた2つの塊は、その2つを覆う矩形へ割れる
    #[test]
    fn two_distant_blocks_are_cut_apart() {
        let blocks = [(0, 0, 8, 8), (56, 40, 8, 8)];
        for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
            assert_eq!(
                cut_of(color_type, &blocks, 128).1,
                Some((rect(0, 0, 8, 8), rect(56, 40, 8, 8))),
                "{color_type:?}"
            );
        }
    }

    /// 書かずに済む画素が下限に届かないときは割らない
    #[test]
    fn a_gap_below_the_threshold_is_left_alone() {
        // 外接矩形は 14x20 = 280 画素で、割ると 240 画素になる
        let blocks = [(0, 0, 6, 20), (8, 0, 6, 20)];
        assert_eq!(cut_of(ColorType::Rgba8, &blocks, 128).1, None);
        assert_eq!(
            cut_of(ColorType::Rgba8, &blocks, 32).1,
            Some((rect(0, 0, 6, 20), rect(8, 0, 6, 20)))
        );
    }

    /// 行が重なっていても、縦に割れば書く画素が減る
    ///
    /// 全く変化していない行が無いので、横向きの帯を探すだけでは割れない。
    #[test]
    fn blocks_that_share_rows_are_cut_vertically() {
        let blocks = [(0, 0, 8, 40), (56, 8, 8, 40)];
        assert_eq!(
            cut_of(ColorType::Rgba8, &blocks, 128).1,
            Some((rect(0, 0, 8, 40), rect(56, 8, 8, 40)))
        );
    }

    /// 変化が3つに散っていても、割るのは1度だけ
    ///
    /// 左端の列を切り離すと 2624 画素が減り、上端の行で切るより多く減る。
    /// 残った2つは同じ矩形に束ねられる。
    #[test]
    fn a_third_block_does_not_add_a_third_rect() {
        let blocks = [(0, 0, 8, 8), (56, 0, 8, 8), (0, 40, 8, 8)];
        assert_eq!(
            cut_of(ColorType::Rgba8, &blocks, 128).1,
            Some((rect(0, 0, 8, 48), rect(56, 0, 8, 8)))
        );
    }

    /// 割った矩形は重ならず、書き直す画素をすべて覆う
    #[test]
    fn the_pieces_cover_every_changed_pixel_without_overlapping() {
        let color_type = ColorType::Rgba8;
        let blocks = [(3, 5, 7, 9), (40, 30, 11, 13)];
        let (previous, frame) = painted(color_type, &blocks);
        let (_, cut) = cut_of(color_type, &blocks, 32);
        let (head, tail) = cut.expect("割れていない");

        let mut covered = vec![0u8; (WIDTH * HEIGHT) as usize];
        for piece in [head, tail] {
            for y in piece.y..piece.y + piece.height {
                for x in piece.x..piece.x + piece.width {
                    covered[(y * WIDTH + x) as usize] += 1;
                }
            }
        }
        assert!(
            covered.iter().all(|count| *count <= 1),
            "矩形が重なっている"
        );

        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let at = ((y * WIDTH + x) * 4) as usize;
                let changed = previous[at..at + 4] != frame[at..at + 4];
                assert!(
                    !changed || covered[(y * WIDTH + x) as usize] == 1,
                    "({x}, {y}) が覆われていない: {head:?} {tail:?}"
                );
            }
        }
    }

    /// 変化が1つに固まっているときは割らない
    #[test]
    fn a_solid_block_is_not_cut() {
        let (bounds, cut) = cut_of(ColorType::Rgba8, &[(8, 8, 32, 24)], 8);
        assert_eq!(bounds, rect(8, 8, 32, 24));
        assert_eq!(cut, None);
    }
}
