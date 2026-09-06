//! 遅延時間の1/100秒への変換

use anim_core::{Accumulator, FrameDelay};

/// 遅延時間の下限 (1/100秒)
///
/// 0と1は多くのデコーダが10へ引き上げるため、それを避ける下限を置く。
pub(crate) const MIN_DELAY: u16 = 2;

/// フレーム遅延を1/100秒へ累積で丸める
pub(crate) struct Hundredths(Accumulator);

impl Hundredths {
    pub(crate) fn new() -> Self {
        Hundredths(Accumulator::new(100))
    }

    /// 次のフレームの遅延を1/100秒へ変換し、下限で切り上げたかどうかを添える
    ///
    /// 下限での切り上げも上限での飽和も残差へは戻さない。
    /// 上限での飽和は素材のレートで再生できないことを表さないため数えない。
    pub(crate) fn next(&mut self, delay: FrameDelay) -> (u16, bool) {
        let rounded = self.0.next(delay);

        let floor = u64::from(MIN_DELAY);
        (
            rounded.clamp(floor, u64::from(u16::MAX)) as u16,
            rounded < floor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1フレームだけを変換する
    fn once(numerator: u32, denominator: u32) -> (u16, bool) {
        Hundredths::new().next(FrameDelay::new(numerator, denominator).unwrap())
    }

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
            assert_eq!(
                once(numerator, denominator),
                (expected, false),
                "{numerator}/{denominator}"
            );
        }
    }

    /// 0と1へ丸まる遅延だけが下限まで切り上げられる
    #[test]
    fn a_delay_below_the_lower_bound_is_raised_and_reported() {
        for (numerator, denominator) in [(0, 30), (1, 1000), (1, 100)] {
            assert_eq!(
                once(numerator, denominator),
                (MIN_DELAY, true),
                "{numerator}/{denominator}"
            );
        }

        // 下限そのものへ丸まる遅延は切り上げていない
        assert_eq!(once(1, 60), (MIN_DELAY, false));
    }

    /// 切り上げたぶんは累積へ戻さない
    #[test]
    fn the_raise_to_the_lower_bound_is_not_fed_back() {
        let mut hundredths = Hundredths::new();
        let delays = [(1, 100), (1, 10), (1, 10)].map(|(numerator, denominator)| {
            hundredths
                .next(FrameDelay::new(numerator, denominator).unwrap())
                .0
        });
        assert_eq!(delays, [MIN_DELAY, 10, 10]);
    }

    /// 上限を超える遅延は飽和させるが、切り上げとしては数えず、累積へも戻さない
    #[test]
    fn a_long_delay_saturates_at_the_field_width() {
        let mut hundredths = Hundredths::new();
        assert_eq!(
            hundredths.next(FrameDelay::new(1000, 1).unwrap()),
            (65535, false)
        );
        assert_eq!(
            hundredths.next(FrameDelay::new(1, 10).unwrap()),
            (10, false)
        );
    }
}
