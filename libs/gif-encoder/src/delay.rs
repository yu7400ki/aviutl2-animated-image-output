//! 遅延時間の1/100秒への変換

use anim_core::FrameDelay;

/// 遅延時間の下限 (1/100秒)
///
/// 0と1は多くのデコーダが10へ引き上げるため、それを避ける下限を置く。
pub(crate) const MIN_DELAY: u64 = 2;

/// フレーム遅延を1/100秒へ丸め、下限で切り上げたかどうかを添える
///
/// 上限での飽和は、素材のレートで再生できないことを表さないため数えない。
pub(crate) fn hundredths(delay: FrameDelay) -> (u16, bool) {
    let numerator = u64::from(delay.numerator()) * 100;
    let denominator = u64::from(delay.denominator());
    let rounded = (numerator + denominator / 2) / denominator;
    (
        rounded.clamp(MIN_DELAY, u64::from(u16::MAX)) as u16,
        rounded < MIN_DELAY,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_is_rounded_to_hundredths_of_a_second() {
        for (numerator, denominator, expected) in [
            (1, 30, 3),
            (2, 30, 7),
            (1, 1, 100),
            (1001, 30000, 3),
            (1, 10, 10),
            (3, 40, 8),
        ] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            assert_eq!(
                hundredths(delay),
                (expected, false),
                "{numerator}/{denominator}"
            );
        }
    }

    /// 0と1へ丸まる遅延だけが下限まで切り上げられる
    #[test]
    fn a_delay_below_the_lower_bound_is_raised_and_reported() {
        for (numerator, denominator) in [(0, 30), (1, 1000), (1, 100)] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            assert_eq!(
                hundredths(delay),
                (MIN_DELAY as u16, true),
                "{numerator}/{denominator}"
            );
        }

        // 下限そのものへ丸まる遅延は切り上げていない
        let delay = FrameDelay::new(1, 60).unwrap();
        assert_eq!(hundredths(delay), (MIN_DELAY as u16, false));
    }

    /// 上限を超える遅延は飽和させるが、切り上げとしては数えない
    #[test]
    fn a_long_delay_saturates_at_the_field_width() {
        let delay = FrameDelay::new(1000, 1).unwrap();
        assert_eq!(hundredths(delay), (u16::MAX, false));
    }
}
