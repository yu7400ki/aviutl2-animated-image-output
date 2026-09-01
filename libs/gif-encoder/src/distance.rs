//! 色の隔たりの測り方

/// 二乗距離を軸ごとに重み付ける係数 (R, G, B)
///
/// 人の目は緑の差に最も敏感で、赤と青はその半分強しか効かない。軸ごとの固定の
/// 係数は RGB 空間を軸方向へ伸縮するだけなので、軸に平行な箱を扱う量子化の
/// 累積モーメントも、箱の代表色を決める重心も形を変えない。
pub(crate) const AXIS_WEIGHTS: [u32; 3] = [5, 8, 5];

/// [`AXIS_WEIGHTS`] の総和
///
/// 3軸が等しく `d` だけずれた画素の二乗距離は `WEIGHT_SUM * d^2` になる。
pub(crate) const WEIGHT_SUM: u32 = AXIS_WEIGHTS[0] + AXIS_WEIGHTS[1] + AXIS_WEIGHTS[2];

/// 2色のRGBの重み付き二乗距離
pub(crate) fn distance(a: u32, b: u32) -> u32 {
    let [ar, ag, ab, _] = a.to_le_bytes();
    let [br, bg, bb, _] = b.to_le_bytes();
    let squared = |weight: u32, x: u8, y: u8| {
        let difference = i32::from(x) - i32::from(y);
        weight * (difference * difference) as u32
    };
    squared(AXIS_WEIGHTS[0], ar, br)
        + squared(AXIS_WEIGHTS[1], ag, bg)
        + squared(AXIS_WEIGHTS[2], ab, bb)
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

    /// 3軸が等しくずれた画素の二乗距離は、ずれの二乗の [`WEIGHT_SUM`] 倍
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
