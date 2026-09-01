//! Wu量子化 (6-6-6ヒストグラム → 最大256色) と最近傍写像

use crate::distance::AXIS_WEIGHTS;
use crate::layout::Layout;
use crate::normalize::{TRANSPARENT, pack};

/// 1軸あたりのビン数 (6bit)
const BINS: usize = 64;
/// 画素の実値からビンの添字を取り出すシフト量
const BIN_SHIFT: u32 = 2;
/// 累積モーメントの1軸あたりの要素数
///
/// 添字0は包除で引くための0の面で、ビンは1から始まる。
const SIDE: usize = BINS + 1;
/// 累積モーメントの要素数
const CELLS: usize = SIDE * SIDE * SIDE;
/// ビンの総数
const BIN_COUNT: usize = BINS * BINS * BINS;

/// 累積モーメントの添字
fn at(r: usize, g: usize, b: usize) -> usize {
    (r * SIDE + g) * SIDE + b
}

/// 累積モーメントに積める値
trait Moment: Copy + Default + std::ops::AddAssign {
    /// 包除の引き算に使う符号付きの値
    fn widen(self) -> i128;
}

impl Moment for u64 {
    fn widen(self) -> i128 {
        i128::from(self)
    }
}

impl Moment for u128 {
    fn widen(self) -> i128 {
        i128::try_from(self).expect("二乗和が符号付きの範囲を超えた")
    }
}

/// 窓のフレームの全画素から量子化の材料を積む
///
/// 透過標識は色を持たないため積まない。`window` は書き出し位置から順に
/// 並んだフレーム。
pub(crate) fn material<'a>(layout: &Layout, window: impl Iterator<Item = &'a [u8]>) -> Histogram {
    let bpp = layout.bytes_per_pixel;
    let mut histogram = Histogram::new();
    for frame in window {
        for pixel in frame.chunks_exact(bpp) {
            let color = pack(pixel, bpp);
            if color == TRANSPARENT {
                continue;
            }
            histogram.observe_color(color, 1);
        }
    }
    histogram
}

/// ビンごとの値を3次元の累積和へ置き換える
fn accumulate<T: Moment>(values: &mut [T]) {
    let mut area = [T::default(); SIDE];
    for r in 1..SIDE {
        area.fill(T::default());
        for g in 1..SIDE {
            let mut line = T::default();
            for b in 1..SIDE {
                line += values[at(r, g, b)];
                area[b] += line;
                let mut cell = values[at(r - 1, g, b)];
                cell += area[b];
                values[at(r, g, b)] = cell;
            }
        }
    }
}

/// 箱の中のモーメントの総和
fn volume<T: Moment>(cube: Cube, moment: &[T]) -> i128 {
    let value = |r: usize, g: usize, b: usize| moment[at(r, g, b)].widen();
    value(cube.r1, cube.g1, cube.b1)
        - value(cube.r1, cube.g1, cube.b0)
        - value(cube.r1, cube.g0, cube.b1)
        + value(cube.r1, cube.g0, cube.b0)
        - value(cube.r0, cube.g1, cube.b1)
        + value(cube.r0, cube.g1, cube.b0)
        + value(cube.r0, cube.g0, cube.b1)
        - value(cube.r0, cube.g0, cube.b0)
}

/// 軸を切った下側のモーメントのうち、切る位置に依らない部分
fn bottom<T: Moment>(cube: Cube, axis: Axis, moment: &[T]) -> i128 {
    let value = |r: usize, g: usize, b: usize| moment[at(r, g, b)].widen();
    match axis {
        Axis::Red => {
            -value(cube.r0, cube.g1, cube.b1)
                + value(cube.r0, cube.g1, cube.b0)
                + value(cube.r0, cube.g0, cube.b1)
                - value(cube.r0, cube.g0, cube.b0)
        }
        Axis::Green => {
            -value(cube.r1, cube.g0, cube.b1)
                + value(cube.r1, cube.g0, cube.b0)
                + value(cube.r0, cube.g0, cube.b1)
                - value(cube.r0, cube.g0, cube.b0)
        }
        Axis::Blue => {
            -value(cube.r1, cube.g1, cube.b0)
                + value(cube.r1, cube.g0, cube.b0)
                + value(cube.r0, cube.g1, cube.b0)
                - value(cube.r0, cube.g0, cube.b0)
        }
    }
}

/// 軸を `position` で切った下側のモーメントのうち、位置に依る部分
fn top<T: Moment>(cube: Cube, axis: Axis, position: usize, moment: &[T]) -> i128 {
    let value = |r: usize, g: usize, b: usize| moment[at(r, g, b)].widen();
    match axis {
        Axis::Red => {
            value(position, cube.g1, cube.b1)
                - value(position, cube.g1, cube.b0)
                - value(position, cube.g0, cube.b1)
                + value(position, cube.g0, cube.b0)
        }
        Axis::Green => {
            value(cube.r1, position, cube.b1)
                - value(cube.r1, position, cube.b0)
                - value(cube.r0, position, cube.b1)
                + value(cube.r0, position, cube.b0)
        }
        Axis::Blue => {
            value(cube.r1, cube.g1, position)
                - value(cube.r1, cube.g0, position)
                - value(cube.r0, cube.g1, position)
                + value(cube.r0, cube.g0, position)
        }
    }
}

/// 色が落ちるビンの添字
fn bin_of(color: u32) -> usize {
    let [r, g, b, _] = color.to_le_bytes();
    ((r >> BIN_SHIFT) as usize * BINS + (g >> BIN_SHIFT) as usize) * BINS
        + (b >> BIN_SHIFT) as usize
}

/// 分割する軸
#[derive(Clone, Copy)]
enum Axis {
    Red,
    Green,
    Blue,
}

/// ビンの範囲で表した箱
///
/// 下限は排他、上限は包含で、累積モーメントの添字にそのまま使う。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Cube {
    r0: usize,
    r1: usize,
    g0: usize,
    g1: usize,
    b0: usize,
    b1: usize,
}

impl Cube {
    /// 箱が覆うビンの数
    fn bins(&self) -> usize {
        (self.r1 - self.r0) * (self.g1 - self.g0) * (self.b1 - self.b0)
    }
}

/// 箱全体の一次モーメントと重み
#[derive(Clone, Copy)]
struct Whole {
    red: i128,
    green: i128,
    blue: i128,
    weight: i128,
}

/// 6-6-6に丸めた色のヒストグラム
///
/// ビンには画素数だけでなく実値の和と二乗和も積む。パレットの色が箱内の平均に
/// なるのはこのためで、丸めが効くのは箱の境界の粒度だけになる。
///
/// 一次モーメントは実値のまま積む。代表色は軸ごとに独立な最小化で決まり、
/// [`AXIS_WEIGHTS`] を掛けても動かないため。
pub(crate) struct Histogram {
    weight: Box<[u64]>,
    red: Box<[u64]>,
    green: Box<[u64]>,
    blue: Box<[u64]>,
    /// 実値の重み付き二乗和
    squared: Box<[u128]>,
    /// 重みが載ったビンの数
    distinct: usize,
}

impl Histogram {
    /// 何も積んでいないヒストグラム
    pub(crate) fn new() -> Self {
        let zeros = || vec![0u64; CELLS].into_boxed_slice();
        Histogram {
            weight: zeros(),
            red: zeros(),
            green: zeros(),
            blue: zeros(),
            squared: vec![0u128; CELLS].into_boxed_slice(),
            distinct: 0,
        }
    }

    /// 重みが載ったビンの数
    ///
    /// 分割はビンの境界にしか置けないため、この数より多くの色は割り出せない。
    pub(crate) fn distinct(&self) -> usize {
        self.distinct
    }

    /// 色を `count` 画素ぶん積む
    pub(crate) fn observe_color(&mut self, color: u32, count: u64) {
        let [r, g, b, _] = color.to_le_bytes();
        self.add(r, g, b, count);
    }

    /// 色を `count` 画素ぶん積む
    fn add(&mut self, r: u8, g: u8, b: u8, count: u64) {
        let cell = at(
            (r >> BIN_SHIFT) as usize + 1,
            (g >> BIN_SHIFT) as usize + 1,
            (b >> BIN_SHIFT) as usize + 1,
        );
        let (r, g, b) = (u64::from(r), u64::from(g), u64::from(b));
        if self.weight[cell] == 0 {
            self.distinct += 1;
        }
        self.weight[cell] += count;
        self.red[cell] += count * r;
        self.green[cell] += count * g;
        self.blue[cell] += count * b;
        let weighted = u64::from(AXIS_WEIGHTS[0]) * r * r
            + u64::from(AXIS_WEIGHTS[1]) * g * g
            + u64::from(AXIS_WEIGHTS[2]) * b * b;
        self.squared[cell] += u128::from(count) * u128::from(weighted);
    }

    /// 積んだ色を `target` 色以下へ割り、箱ごとの平均色を添字順に返す
    ///
    /// 分散が0になるまで割り切れた場合は `target` より少ない色を返す。
    ///
    /// # Panics
    /// `target` が0のとき。
    pub(crate) fn quantize(mut self, target: usize) -> Vec<u32> {
        assert!(target > 0, "目標色数は1以上");
        self.accumulate();

        let mut cubes = vec![Cube::default(); target];
        cubes[0] = Cube {
            r0: 0,
            r1: BINS,
            g0: 0,
            g1: BINS,
            b0: 0,
            b1: BINS,
        };
        let mut variances = vec![0.0f64; target];
        let mut created = 1;
        let mut next = 0;

        while created < target {
            let mut left = cubes[next];
            let mut right = Cube::default();
            if self.cut(&mut left, &mut right) {
                cubes[next] = left;
                cubes[created] = right;
                variances[next] = self.split_gain(left);
                variances[created] = self.split_gain(right);
                created += 1;
            } else {
                // 割れなかった箱は二度と選ばない
                variances[next] = 0.0;
            }

            let mut best = variances[0];
            next = 0;
            for (index, &variance) in variances[..created].iter().enumerate().skip(1) {
                if variance > best {
                    best = variance;
                    next = index;
                }
            }
            if best <= 0.0 {
                break;
            }
        }

        cubes[..created]
            .iter()
            .filter_map(|&cube| self.average(cube))
            .collect()
    }

    /// 5つの面をそれぞれ3次元の累積和へ置き換える
    fn accumulate(&mut self) {
        accumulate(&mut self.weight);
        accumulate(&mut self.red);
        accumulate(&mut self.green);
        accumulate(&mut self.blue);
        accumulate(&mut self.squared);
    }

    /// 箱の中の画素が平均色から離れている量 (重み付き二乗誤差の総和)
    ///
    /// `m2 - Σ_a w_a d_a^2 / wt`。`m2` は積むときに [`AXIS_WEIGHTS`] が
    /// 掛かっているので、一次モーメントの側だけをここで掛ける。
    fn variance(&self, cube: Cube) -> f64 {
        let weight = volume(cube, &self.weight);
        if weight <= 0 {
            return 0.0;
        }

        let red = volume(cube, &self.red);
        let green = volume(cube, &self.green);
        let blue = volume(cube, &self.blue);
        let deviation = i128::from(AXIS_WEIGHTS[0]) * red * red
            + i128::from(AXIS_WEIGHTS[1]) * green * green
            + i128::from(AXIS_WEIGHTS[2]) * blue * blue;
        volume(cube, &self.squared) as f64 - deviation as f64 / weight as f64
    }

    /// もう一度割る値打ち
    ///
    /// ビンが1つしかない箱はどの軸でも切れないため、分散を見るまでもなく0。
    fn split_gain(&self, cube: Cube) -> f64 {
        if cube.bins() > 1 {
            self.variance(cube)
        } else {
            0.0
        }
    }

    /// 軸方向に切る位置と、そのときの評価値
    ///
    /// 切ってできる2つの箱の一次モーメントの重み付き二乗和を画素数で割ったものを
    /// 最大化する。どちらかが空になる位置は候補にしない。
    ///
    /// 重みは [`Self::variance`] と同じ [`AXIS_WEIGHTS`]。同じ箱の同じ量を別の形で
    /// 見ているので、順位を付けた基準と切る位置を選んだ基準が揃う。
    fn maximize(
        &self,
        cube: Cube,
        axis: Axis,
        range: (usize, usize),
        whole: Whole,
    ) -> (Option<usize>, f64) {
        let base_red = bottom(cube, axis, &self.red);
        let base_green = bottom(cube, axis, &self.green);
        let base_blue = bottom(cube, axis, &self.blue);
        let base_weight = bottom(cube, axis, &self.weight);

        let mut cut = None;
        let mut best = 0.0f64;
        for position in range.0..range.1 {
            let red = base_red + top(cube, axis, position, &self.red);
            let green = base_green + top(cube, axis, position, &self.green);
            let blue = base_blue + top(cube, axis, position, &self.blue);
            let weight = base_weight + top(cube, axis, position, &self.weight);
            if weight == 0 {
                continue;
            }
            let rest = whole.weight - weight;
            if rest == 0 {
                break;
            }

            let square = |r: i128, g: i128, b: i128| {
                (i128::from(AXIS_WEIGHTS[0]) * r * r
                    + i128::from(AXIS_WEIGHTS[1]) * g * g
                    + i128::from(AXIS_WEIGHTS[2]) * b * b) as f64
            };
            let gain = square(red, green, blue) / weight as f64
                + square(whole.red - red, whole.green - green, whole.blue - blue) / rest as f64;
            if gain > best {
                best = gain;
                cut = Some(position);
            }
        }
        (cut, best)
    }

    /// 分散が最も減る軸で `cube` を2つに割る
    ///
    /// どの軸でも切れなければ偽を返し、`cube` と `rest` は変えない。
    fn cut(&self, cube: &mut Cube, rest: &mut Cube) -> bool {
        let whole = Whole {
            red: volume(*cube, &self.red),
            green: volume(*cube, &self.green),
            blue: volume(*cube, &self.blue),
            weight: volume(*cube, &self.weight),
        };

        let candidates = [
            (
                Axis::Red,
                self.maximize(*cube, Axis::Red, (cube.r0 + 1, cube.r1), whole),
            ),
            (
                Axis::Green,
                self.maximize(*cube, Axis::Green, (cube.g0 + 1, cube.g1), whole),
            ),
            (
                Axis::Blue,
                self.maximize(*cube, Axis::Blue, (cube.b0 + 1, cube.b1), whole),
            ),
        ];

        let mut chosen = &candidates[0];
        for candidate in &candidates[1..] {
            if candidate.1.1 > chosen.1.1 {
                chosen = candidate;
            }
        }
        let Some(position) = chosen.1.0 else {
            return false;
        };

        *rest = Cube {
            r1: cube.r1,
            g1: cube.g1,
            b1: cube.b1,
            ..*cube
        };
        match chosen.0 {
            Axis::Red => {
                cube.r1 = position;
                rest.r0 = position;
            }
            Axis::Green => {
                cube.g1 = position;
                rest.g0 = position;
            }
            Axis::Blue => {
                cube.b1 = position;
                rest.b0 = position;
            }
        }
        true
    }

    /// 箱の中の画素の平均色。1画素も入っていない箱は `None`
    fn average(&self, cube: Cube) -> Option<u32> {
        let weight = volume(cube, &self.weight);
        if weight <= 0 {
            return None;
        }

        let mean = |moment: &[u64]| {
            let sum = volume(cube, moment);
            ((sum * 2 + weight) / (weight * 2)) as u8
        };
        Some(u32::from_le_bytes([
            mean(&self.red),
            mean(&self.green),
            mean(&self.blue),
            u8::MAX,
        ]))
    }
}

/// 最近傍で写す先の添字を、ビン単位で覚える表
///
/// 覚えるのはビンの中心色に対する最近傍で、これはビンだけの関数なので走査順に
/// 依存しない。量子化器を決定的にした意味が写像でも保たれる。
pub(crate) struct Nearest {
    /// 写す先の候補 (2倍した色, 添字)
    candidates: Vec<([i32; 3], u8)>,
    /// ビンごとの最近傍の添字
    cache: Box<[u8]>,
    /// [`Self::cache`] のどの要素が求まっているか
    known: Box<[u64]>,
}

/// ビンの中心色を2倍した座標
///
/// ビン `bin` が覆う実値は `4*bin ..= 4*bin+3` なので、中心は `4*bin + 1.5`。
/// 整数のまま比べるため全体を2倍した座標を使う。
fn center_doubled(bin: usize) -> i32 {
    bin as i32 * 8 + 3
}

impl Nearest {
    /// 添字順に並べた `entries` のうち、透過でないものを写す先にする
    ///
    /// 透過標識は色として近似する相手にならない。2値透過に中間が無く、透過ラン用に
    /// 足したスロットも色を持たないため。
    pub(crate) fn new(entries: &[u32]) -> Self {
        let candidates = entries
            .iter()
            .enumerate()
            .filter(|&(_, &color)| color != TRANSPARENT)
            .map(|(index, &color)| {
                let [r, g, b, _] = color.to_le_bytes();
                (
                    [i32::from(r) * 2, i32::from(g) * 2, i32::from(b) * 2],
                    index as u8,
                )
            })
            .collect();

        Nearest {
            candidates,
            cache: vec![0u8; BIN_COUNT].into_boxed_slice(),
            known: vec![0u64; BIN_COUNT / u64::BITS as usize].into_boxed_slice(),
        }
    }

    /// `color` を写す先の添字
    ///
    /// # Panics
    /// 写す先の候補が1つも無いとき。
    pub(crate) fn index_of(&mut self, color: u32) -> u8 {
        let bin = bin_of(color);
        let (word, bit) = (bin / u64::BITS as usize, bin % u64::BITS as usize);
        if self.known[word] >> bit & 1 == 0 {
            self.cache[bin] = self.search(bin);
            self.known[word] |= 1 << bit;
        }
        self.cache[bin]
    }

    /// ビンの中心色に最も近い候補の添字。等距離なら添字の小さい方
    ///
    /// 近さは2倍座標での [`AXIS_WEIGHTS`] 付き二乗距離で測る。
    fn search(&self, bin: usize) -> u8 {
        let center = [
            center_doubled(bin >> 12),
            center_doubled(bin >> 6 & (BINS - 1)),
            center_doubled(bin & (BINS - 1)),
        ];

        let mut best = None;
        for (color, index) in &self.candidates {
            let distance: i32 = (0..3)
                .map(|c| AXIS_WEIGHTS[c] as i32 * (center[c] - color[c]).pow(2))
                .sum();
            if best.is_none_or(|(shortest, _)| distance < shortest) {
                best = Some((distance, *index));
            }
        }
        best.expect("写す先の候補が無い").1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distance::distance;

    /// 色を1画素ぶん積む
    fn observe_color(histogram: &mut Histogram, color: [u8; 3], count: u64) {
        histogram.add(color[0], color[1], color[2], count);
    }

    /// ビン全体を覆う箱
    fn whole() -> Cube {
        Cube {
            r0: 0,
            r1: BINS,
            g0: 0,
            g1: BINS,
            b0: 0,
            b1: BINS,
        }
    }

    fn pack(color: [u8; 3]) -> u32 {
        u32::from_le_bytes([color[0], color[1], color[2], u8::MAX])
    }

    /// 目標色数まで割ると、離れた集団がそれぞれ自分の箱を持つ
    ///
    /// 箱の色は中に入った画素の平均なので、集団が1色ずつなら元の色がそのまま戻る。
    #[test]
    fn well_separated_clusters_each_keep_their_own_color() {
        const CLUSTERS: [[u8; 3]; 4] = [[10, 20, 30], [200, 30, 40], [40, 210, 50], [60, 70, 220]];

        let mut histogram = Histogram::new();
        for color in CLUSTERS {
            observe_color(&mut histogram, color, 100);
        }

        let mut palette = histogram.quantize(4);
        palette.sort_unstable();
        let mut expected: Vec<u32> = CLUSTERS.iter().map(|&color| pack(color)).collect();
        expected.sort_unstable();
        assert_eq!(palette, expected);
    }

    /// 箱の色は中の画素の重み付き平均になる
    #[test]
    fn the_color_of_a_box_is_the_weighted_mean_of_its_pixels() {
        let mut histogram = Histogram::new();
        // 同じビン (実値 40..=43) の2色。どう割っても同じ箱に入る
        observe_color(&mut histogram, [40, 40, 40], 3);
        observe_color(&mut histogram, [43, 40, 40], 1);

        assert_eq!(histogram.quantize(16), vec![pack([41, 40, 40])]);
    }

    /// 箱の代表色は軸ごとの重みで動かない
    ///
    /// 丸めた重心の重み付き二乗誤差が、軸ごとに独立へ最小化した誤差の和に
    /// 一致する。軸ごとの最小値は重みを掛ける前に決まるので、一致するかぎり
    /// 代表色はどの重みでも同じ色になる。
    #[test]
    fn the_color_of_a_box_does_not_move_with_the_axis_weights() {
        const SAMPLES: [([u8; 3], u64); 3] =
            [([0, 20, 255], 3), ([7, 220, 40], 2), ([255, 90, 4], 5)];

        let mut histogram = Histogram::new();
        for (color, count) in SAMPLES {
            observe_color(&mut histogram, color, count);
        }
        histogram.accumulate();
        let average = histogram.average(whole()).expect("箱に画素が無い");

        let centroid: u64 = SAMPLES
            .iter()
            .map(|&(color, count)| count * u64::from(distance(pack(color), average)))
            .sum();
        let independent: u64 = AXIS_WEIGHTS
            .iter()
            .enumerate()
            .map(|(axis, &weight)| {
                let smallest = (0..=u64::from(u8::MAX))
                    .map(|candidate| {
                        SAMPLES
                            .iter()
                            .map(|&(color, count)| {
                                let difference = u64::from(color[axis]).abs_diff(candidate);
                                count * difference * difference
                            })
                            .sum::<u64>()
                    })
                    .min()
                    .expect("候補が無い");
                u64::from(weight) * smallest
            })
            .sum();

        assert_eq!(centroid, independent, "重心が軸ごとの最小値を外している");
    }

    /// 目標色数が集団の数に満たないときは、近い集団から1つの箱へまとまる
    #[test]
    fn a_target_below_the_cluster_count_merges_the_closest_ones() {
        let mut histogram = Histogram::new();
        observe_color(&mut histogram, [0, 0, 0], 100);
        observe_color(&mut histogram, [4, 0, 0], 100);
        observe_color(&mut histogram, [252, 252, 252], 200);

        let mut palette = histogram.quantize(2);
        palette.sort_unstable();
        assert_eq!(palette, vec![pack([2, 0, 0]), pack([252, 252, 252])]);
    }

    /// 割り切れたら目標色数に満たなくても止まる
    #[test]
    fn splitting_stops_once_every_box_holds_a_single_color() {
        let mut histogram = Histogram::new();
        observe_color(&mut histogram, [0, 0, 0], 1);
        observe_color(&mut histogram, [128, 128, 128], 1);

        assert_eq!(histogram.quantize(64).len(), 2);
    }

    /// 箱の順位付けは総二乗誤差で決まり、画素数では決まらない
    ///
    /// 画素の多い散らばりの小さい箱が、画素の少ない散らばりの大きい箱を
    /// 追い越さないこと。二乗誤差に画素数を掛けると順位が入れ替わり、
    /// 色が画素の多い箱へ偏る。
    #[test]
    fn boxes_are_ranked_by_their_total_squared_error() {
        let mut histogram = Histogram::new();
        observe_color(&mut histogram, [0, 0, 0], 1000);
        observe_color(&mut histogram, [4, 0, 0], 1000);
        observe_color(&mut histogram, [0, 4, 0], 1);
        observe_color(&mut histogram, [0, 252, 0], 1);
        histogram.accumulate();

        let heavy = Cube {
            r0: 0,
            r1: BINS,
            g0: 0,
            g1: 1,
            b0: 0,
            b1: 1,
        };
        let light = Cube {
            r0: 0,
            r1: 1,
            g0: 1,
            g1: BINS,
            b0: 0,
            b1: 1,
        };
        assert_eq!(volume(heavy, &histogram.weight), 2000);
        assert_eq!(volume(light, &histogram.weight), 2);

        // 赤に散らばる箱は重み 5、緑に散らばる箱は重み 8 が掛かる
        assert_eq!(histogram.variance(heavy), 40_000.0);
        assert_eq!(histogram.variance(light), 246_016.0);
    }

    /// 分割する軸は軸ごとの重みで入れ替わる
    ///
    /// 赤と緑に同じ広がりを持つ箱では、重みを掛けなければ2軸の利得が並び、
    /// 先に評価する赤で切れる。緑の重みが大きいぶん、切る軸は緑へ移る。
    #[test]
    fn the_axis_that_is_cut_follows_the_axis_weights() {
        let mut histogram = Histogram::new();
        for color in [[0, 0, 0], [252, 0, 0], [0, 252, 0], [252, 252, 0]] {
            observe_color(&mut histogram, color, 1);
        }

        let mut palette = histogram.quantize(2);
        palette.sort_unstable();
        assert_eq!(
            palette,
            vec![pack([126, 0, 0]), pack([126, 252, 0])],
            "赤で切っている"
        );
    }

    /// 分散の評価は u64 で表せない大きさを扱う
    ///
    /// 同じ画素数の黒と白では重み付き二乗誤差の総和が `571536 * 画素数` になる。
    /// ここで積む画素数では総和が 1.14e20、二乗和が 2.29e20 で、どちらも u64 の
    /// 上限 1.84e19 を超える。割る前の分子はさらに大きく 4.57e34 になる。
    #[test]
    fn the_variance_is_evaluated_beyond_the_range_of_u64() {
        const COUNT: u64 = 200_000_000_000_000;

        let mut histogram = Histogram::new();
        observe_color(&mut histogram, [0, 0, 0], COUNT);
        observe_color(&mut histogram, [252, 252, 252], COUNT);
        histogram.accumulate();

        let expected = 571_536.0 * COUNT as f64;
        assert!(expected > u64::MAX as f64, "u64 に収まる大きさになっている");
        assert!(
            volume(whole(), &histogram.squared) as f64 > u64::MAX as f64,
            "二乗和が u64 に収まる大きさになっている"
        );

        let variance = histogram.variance(whole());
        assert!(
            (variance - expected).abs() <= expected * 1e-12,
            "{variance} が {expected} から離れている"
        );
    }

    /// キャッシュはビンの中心色に対する最近傍を持つ
    ///
    /// ビン25が覆う実値は 100..=103 で、中心は 101.5。実値 100 の画素に近いのは
    /// `LOW` (距離0) だが、中心に近いのは `HIGH`。最初に落ちた色で覚えると
    /// `LOW` の添字が入る。
    #[test]
    fn the_cache_holds_the_nearest_color_to_the_center_of_the_bin() {
        const LOW: u32 = 0xFF64_6464;
        const HIGH: u32 = 0xFF66_6666;

        let mut nearest = Nearest::new(&[LOW, HIGH]);
        assert_eq!(nearest.index_of(LOW), 1, "中心色で引いていない");
        // 同じビンのどの色を引いても同じ添字になる
        assert_eq!(nearest.index_of(0xFF67_6767), 1);
    }

    /// ビンの中心の写す先は軸ごとの重みで入れ替わる
    ///
    /// ビン (32,32,32) の中心 129.5 に対し、緑へ約8ずれた候補と赤へ約9ずれた
    /// 候補を置く。2倍座標の差の二乗和は 291 対 363 で緑の側が近いが、
    /// 重みを掛けると 2322 対 1818 になって赤の側へ移る。
    #[test]
    fn the_target_of_a_bin_follows_the_axis_weights() {
        let mut nearest = Nearest::new(&[pack([130, 138, 130]), pack([139, 130, 130])]);

        assert_eq!(
            nearest.index_of(pack([129, 129, 129])),
            1,
            "緑のずれを近いと見ている"
        );
    }

    /// 透過標識のエントリは写す先にならない
    #[test]
    fn the_transparent_entry_is_never_a_target() {
        let mut nearest = Nearest::new(&[TRANSPARENT, 0xFF80_8080]);
        assert_eq!(nearest.index_of(0xFF01_0101), 1);
    }

    fn layout() -> Layout {
        Layout::new(2, 1, anim_core::ColorType::Rgb8).unwrap()
    }

    /// 窓の後続フレームで持ち越した画素も積む
    ///
    /// 色 `10` は2枚に跨って3画素あり、色 `30` は1画素。重みが3対1なら平均は 15 で、
    /// 持ち越した1画素を落とすと2対1になって 17 へずれる。
    #[test]
    fn pixels_carried_over_within_the_window_are_counted_again() {
        let first = [10u8, 10, 10, 10, 10, 10];
        let second = [10u8, 10, 10, 30, 30, 30];
        let frames = [&first[..], &second[..]];

        let histogram = material(&layout(), frames.into_iter());
        assert_eq!(histogram.quantize(1), vec![pack([15, 15, 15])]);
    }

    /// 透過標識は積まない
    #[test]
    fn the_transparent_marker_is_not_counted() {
        let layout = Layout::new(2, 1, anim_core::ColorType::Rgba8).unwrap();
        let frame = [0u8, 0, 0, 0, 200, 200, 200, 255];

        let histogram = material(&layout, std::iter::once(&frame[..]));
        assert_eq!(histogram.distinct(), 1);
    }

    /// RGB8の入力に透過は無く、黒はそのまま積む
    ///
    /// 透過標識は詰めた色が (0,0,0,0) で、アルファを持たない画素の黒とは別物。
    /// 取り違えると、黒い領域が量子化の材料から落ちる。
    #[test]
    fn black_is_counted_when_the_input_has_no_alpha() {
        let frame = [0u8, 0, 0, 40, 40, 40];

        let histogram = material(&layout(), std::iter::once(&frame[..]));
        assert_eq!(histogram.distinct(), 2, "黒を積んでいない");
        assert_eq!(histogram.quantize(1), vec![0xFF14_1414], "黒に重みが無い");
    }

    /// 等距離の候補は添字の小さい方へ写る
    #[test]
    fn a_tie_goes_to_the_lower_index() {
        // ビン0の中心は 1.5。0 と 3 はどちらも 1.5 から等距離
        let mut nearest = Nearest::new(&[0xFF00_0000, 0xFF03_0303]);
        assert_eq!(nearest.index_of(0xFF00_0000), 0);
    }
}
