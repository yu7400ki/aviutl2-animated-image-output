//! 色の隔たりの測り方

use std::ops::{AddAssign, Mul};

/// 二乗距離を軸ごとに重み付ける係数 (R, G, B)
pub(crate) const AXIS_WEIGHTS: [u32; 3] = [5, 8, 5];

/// [`AXIS_WEIGHTS`] の総和
///
/// 3軸が等しく `d` だけずれた画素の二乗距離は `WEIGHT_SUM * d^2` になる。
pub(crate) const WEIGHT_SUM: u32 = AXIS_WEIGHTS[0] + AXIS_WEIGHTS[1] + AXIS_WEIGHTS[2];

/// RGB順に並べた軸ごとの値の、[`AXIS_WEIGHTS`] を掛けた二乗和
///
/// 軸と係数は添字で対応する。
pub(crate) fn weighted_square<T>(axes: [T; 3]) -> T
where
    T: Copy + Default + From<u32> + Mul<Output = T> + AddAssign,
{
    let mut total = T::default();
    for (axis, &weight) in AXIS_WEIGHTS.iter().enumerate() {
        total += T::from(weight) * axes[axis] * axes[axis];
    }
    total
}

/// 2色のRGBの重み付き二乗距離
pub(crate) fn distance(a: u32, b: u32) -> u32 {
    let (left, right) = (a.to_le_bytes(), b.to_le_bytes());
    weighted_square(std::array::from_fn(|axis| {
        u32::from(left[axis].abs_diff(right[axis]))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(color: [u8; 3]) -> u32 {
        u32::from_le_bytes([color[0], color[1], color[2], u8::MAX])
    }

    /// 距離は軸ごとに重みを掛ける
    ///
    /// 緑へ 8 ずれた色は、赤へ 9 ずれた色より遠い。差の二乗だけを足すと
    /// 64 < 81 で向きが逆になる。
    #[test]
    fn each_axis_carries_its_own_weight() {
        const BASE: [u8; 3] = [130, 130, 130];

        let green = distance(pack(BASE), pack([130, 138, 130]));
        let red = distance(pack(BASE), pack([139, 130, 130]));

        assert_eq!(green, 512);
        assert_eq!(red, 405);
        assert!(green > red, "緑のずれを近いと見ている");
    }

    #[test]
    fn a_uniform_drift_costs_the_sum_of_the_weights() {
        let drift = distance(pack([0, 0, 0]), pack([8, 8, 8]));

        assert_eq!(drift, WEIGHT_SUM * 8 * 8);
        assert_eq!(drift, 1_152);
    }

    /// 最も離れた2色でも u32 に収まる
    #[test]
    fn the_widest_pair_stays_within_u32() {
        let widest = distance(pack([0, 0, 0]), pack([255, 255, 255]));

        assert_eq!(widest, 1_170_450);
    }
}
