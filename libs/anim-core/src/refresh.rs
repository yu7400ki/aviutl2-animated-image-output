//! 画面に出ている値と入力の隔たりから決まる、書き直す画素

/// 品質に対して、画面に出ている値が入力から離れてよい量
///
/// `quality` は0以上100以下。品質が高いほど小さく、100で1になる。
pub fn tolerance(quality: f32) -> u8 {
    let root = (quality / 100.0).sqrt();
    (31.0 * (1.0 - root) + root).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_sets_the_tolerance() {
        for (quality, expected) in [(50.0, 10), (75.0, 5), (90.0, 3), (100.0, 1)] {
            assert_eq!(tolerance(quality), expected, "品質 {quality}");
        }
    }

    #[test]
    fn the_tolerance_falls_as_quality_rises() {
        let values: Vec<u8> = (0..=100).map(|quality| tolerance(quality as f32)).collect();
        assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
        assert_eq!((values[0], values[100]), (31, 1));
    }
}
