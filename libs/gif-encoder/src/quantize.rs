//! Wu量子化 (6-6-6ヒストグラム → 最大256色) と最近傍写像

use crate::normalize::TRANSPARENT;

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
        self as i128
    }
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
pub(crate) struct Histogram {
    weight: Box<[u64]>,
    red: Box<[u64]>,
    green: Box<[u64]>,
    blue: Box<[u64]>,
    /// 実値の二乗和
    ///
    /// 1画素あたり最大 195,075 積まれるうえ、滞在時間ぶんの重みが掛かるため、
    /// 他の4面より2桁ほど早く u64 を使い切る。
    squared: Box<[u128]>,
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
        }
    }

    /// 画素列の色を、1画素あたり `weight` の重みで積む
    ///
    /// `pixels` は1画素 `bpp` バイトが隙間なく並んでいること。`bpp` は3か4であること。
    ///
    /// 透過標識は積まない。RGBが (0,0,0) なので、積むと透過の面積だけ黒が重くなり、
    /// 分割が黒の周りに箱を割いて非透過色の表現力を削る。
    pub(crate) fn observe(&mut self, pixels: &[u8], bpp: usize, weight: u64) {
        match bpp {
            3 => {
                for pixel in pixels.chunks_exact(3) {
                    self.add(pixel[0], pixel[1], pixel[2], weight);
                }
            }
            4 => {
                for pixel in pixels.chunks_exact(4) {
                    let color = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                    if color == TRANSPARENT {
                        continue;
                    }
                    self.add(pixel[0], pixel[1], pixel[2], weight);
                }
            }
            other => panic!("1画素あたり3バイトか4バイトのみ扱える: {other}"),
        }
    }

    /// 色を `count` 画素ぶん積む
    fn add(&mut self, r: u8, g: u8, b: u8, count: u64) {
        let cell = at(
            (r >> BIN_SHIFT) as usize + 1,
            (g >> BIN_SHIFT) as usize + 1,
            (b >> BIN_SHIFT) as usize + 1,
        );
        let (r, g, b) = (u64::from(r), u64::from(g), u64::from(b));
        self.weight[cell] += count;
        self.red[cell] += count * r;
        self.green[cell] += count * g;
        self.blue[cell] += count * b;
        self.squared[cell] += u128::from(count) * u128::from(r * r + g * g + b * b);
    }

    /// 色が落ちるビンに積まれた画素数
    #[cfg(test)]
    pub(crate) fn weight_of(&self, color: u32) -> u64 {
        let [r, g, b, _] = color.to_le_bytes();
        self.weight[at(
            (r >> BIN_SHIFT) as usize + 1,
            (g >> BIN_SHIFT) as usize + 1,
            (b >> BIN_SHIFT) as usize + 1,
        )]
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

    /// 箱の中の画素が平均色から離れている量 (二乗誤差の総和)
    ///
    /// `m2 - (dr^2 + dg^2 + db^2) / wt`。`dr` は積んだ画素数に比例するので、
    /// その二乗は u64 に収まらない。割る前の分子を i128 で持つ。
    fn variance(&self, cube: Cube) -> f64 {
        let weight = volume(cube, &self.weight);
        if weight <= 0 {
            return 0.0;
        }

        let red = volume(cube, &self.red);
        let green = volume(cube, &self.green);
        let blue = volume(cube, &self.blue);
        let deviation = red * red + green * green + blue * blue;
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
    /// 切ってできる2つの箱の一次モーメントの二乗和を重みで割ったものを最大化する。
    /// どちらかが空になる位置は候補にしない。
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

            let square = |r: i128, g: i128, b: i128| (r * r + g * g + b * b) as f64;
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
    fn search(&self, bin: usize) -> u8 {
        let center = [
            center_doubled(bin >> 12),
            center_doubled(bin >> 6 & (BINS - 1)),
            center_doubled(bin & (BINS - 1)),
        ];

        let mut best = None;
        for (color, index) in &self.candidates {
            let distance: i32 = (0..3).map(|c| (center[c] - color[c]).pow(2)).sum();
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

    /// 色を1画素ぶん積む
    fn observe_color(histogram: &mut Histogram, color: [u8; 3], count: u64) {
        histogram.add(color[0], color[1], color[2], count);
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

    /// 箱の順位付けは総二乗誤差で決まり、重みでは決まらない
    ///
    /// 重いが散らばりの小さい箱が、軽いが散らばりの大きい箱を追い越さないこと。
    /// 二乗誤差に重みを掛けると順位が入れ替わり、色が重い箱へ偏る。
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

        assert_eq!(histogram.variance(heavy), 8_000.0);
        assert_eq!(histogram.variance(light), 30_752.0);
    }

    /// 分散の評価は u64 で表せない大きさを扱う
    ///
    /// 同じ画素数の黒と白では二乗誤差の総和が `95256 * 画素数` になる。ここで積む
    /// 画素数では総和が 1.91e19、二乗和が 3.81e19 で、どちらも u64 の上限
    /// 1.84e19 を超える。割る前の分子はさらに大きく 7.62e33 になる。
    #[test]
    fn the_variance_is_evaluated_beyond_the_range_of_u64() {
        const COUNT: u64 = 200_000_000_000_000;

        let mut histogram = Histogram::new();
        observe_color(&mut histogram, [0, 0, 0], COUNT);
        observe_color(&mut histogram, [252, 252, 252], COUNT);
        histogram.accumulate();

        let whole = Cube {
            r0: 0,
            r1: BINS,
            g0: 0,
            g1: BINS,
            b0: 0,
            b1: BINS,
        };
        let expected = 95_256.0 * COUNT as f64;
        assert!(expected > u64::MAX as f64, "u64 に収まる大きさになっている");
        assert!(
            volume(whole, &histogram.squared) as f64 > u64::MAX as f64,
            "二乗和が u64 に収まる大きさになっている"
        );

        let variance = histogram.variance(whole);
        assert!(
            (variance - expected).abs() <= expected * 1e-12,
            "{variance} が {expected} から離れている"
        );
    }

    /// 透過標識は積まない
    #[test]
    fn the_transparent_marker_is_not_counted() {
        let mut histogram = Histogram::new();
        histogram.observe(&[0, 0, 0, 0, 0, 0, 0, 0, 10, 20, 30, 255], 4, 1);

        // 標識を積んでいれば黒が重みを持ち、平均は黒へ寄る
        assert_eq!(histogram.quantize(1), vec![pack([10, 20, 30])]);
    }

    /// RGB8の入力に透過は無く、黒はそのまま数える
    #[test]
    fn black_is_counted_when_the_input_has_no_alpha() {
        let mut histogram = Histogram::new();
        histogram.observe(&[0, 0, 0, 40, 40, 40], 3, 1);

        assert_eq!(histogram.quantize(1), vec![pack([20, 20, 20])]);
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

    /// 透過標識のエントリは写す先にならない
    #[test]
    fn the_transparent_entry_is_never_a_target() {
        let mut nearest = Nearest::new(&[TRANSPARENT, 0xFF80_8080]);
        assert_eq!(nearest.index_of(0xFF01_0101), 1);
    }

    /// 等距離の候補は添字の小さい方へ写る
    #[test]
    fn a_tie_goes_to_the_lower_index() {
        // ビン0の中心は 1.5。0 と 3 はどちらも 1.5 から等距離
        let mut nearest = Nearest::new(&[0xFF00_0000, 0xFF03_0303]);
        assert_eq!(nearest.index_of(0xFF00_0000), 0);
    }
}
