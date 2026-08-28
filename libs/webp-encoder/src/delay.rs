//! 遅延時間のミリ秒への変換

use anim_core::FrameDelay;

/// 遅延時間の下限 (ms)
///
/// 0の解釈はデコーダ間で揃わないため、それを避ける下限を置く。
const MIN_DELAY: u32 = 1;

/// ANMFの表示時間が取りうる上限 (ms)
const MAX_DELAY: u32 = 0x00FF_FFFF;

/// フレーム遅延をミリ秒へ累積で丸める
///
/// フレーム0..N-1 の遅延の総和を `T_N` として、N番目のフレームへ
/// `round(T_{N+1} * 1000) - round(T_N * 1000)` を割り当てる。丸めは
/// `floor(x + 1/2)` で、整数の平行移動で不変なので、総和そのものを持たずに
/// 「総和を丸めたときの残差」だけで同じ列が出せる。
///
/// 残差は既約分数で持つ。絶対値は常に 1/2 未満なので、フレーム数がいくら
/// 増えても分子・分母は分母の最小公倍数より大きくならない。
pub(crate) struct Milliseconds {
    /// 残差の分子 (ms)。絶対値は `denominator / 2` 以下
    numerator: i64,
    /// 残差の分母
    denominator: u64,
}

impl Milliseconds {
    pub(crate) fn new() -> Self {
        Milliseconds {
            numerator: 0,
            denominator: 1,
        }
    }

    /// 次のフレームの遅延をミリ秒へ変換し、下限で切り上げたかどうかを添える
    ///
    /// 下限での切り上げも上限での飽和も残差へは戻さない。
    /// 上限での飽和は素材のレートで再生できないことを表さないため数えない。
    pub(crate) fn next(&mut self, delay: FrameDelay) -> (u32, bool) {
        let denominator = u64::from(delay.denominator());
        let common = self.align(denominator);
        let scaled = i128::from(self.numerator) * i128::from(common / self.denominator)
            + 1000 * i128::from(delay.numerator()) * i128::from(common / denominator);

        let common = i128::from(common);
        let rounded = (scaled * 2 + common).div_euclid(common * 2);
        self.keep(scaled - rounded * common, common);

        let floor = i128::from(MIN_DELAY);
        (
            rounded.clamp(floor, i128::from(MAX_DELAY)) as u32,
            rounded < floor,
        )
    }

    /// 残差と `denominator` に共通の分母を取る
    ///
    /// 最小公倍数が `u64` に収まらないときは、残差を `denominator` の刻みへ
    /// 丸め直してからその分母を返す。丸めの誤差は 1/(2000 * `denominator`) 秒で、
    /// 分母が互いに素で大きいときにしか起きない。
    fn align(&mut self, denominator: u64) -> u64 {
        let common = u128::from(self.denominator) / gcd(self.denominator, denominator)
            * u128::from(denominator);
        if let Ok(common) = u64::try_from(common) {
            return common;
        }

        let scaled = i128::from(self.numerator) * i128::from(denominator);
        let previous = i128::from(self.denominator);
        let rounded = (scaled * 2 + previous).div_euclid(previous * 2);
        self.keep(rounded, i128::from(denominator));
        denominator
    }

    /// `numerator / denominator` を既約分数にして残差に据える
    fn keep(&mut self, numerator: i128, denominator: i128) {
        let divisor = gcd(numerator.unsigned_abs(), denominator as u128) as i128;
        self.numerator = (numerator / divisor) as i64;
        self.denominator = (denominator / divisor) as u64;
    }
}

fn gcd(a: impl Into<u128>, b: impl Into<u128>) -> u128 {
    let (mut a, mut b) = (a.into(), b.into());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1フレームだけを変換する
    fn once(numerator: u32, denominator: u32) -> (u32, bool) {
        Milliseconds::new().next(FrameDelay::new(numerator, denominator).unwrap())
    }

    /// 同じ遅延を `count` フレーム分変換する
    fn repeated(numerator: u32, denominator: u32, count: usize) -> Vec<u32> {
        let delay = FrameDelay::new(numerator, denominator).unwrap();
        let mut milliseconds = Milliseconds::new();
        (0..count).map(|_| milliseconds.next(delay).0).collect()
    }

    #[test]
    fn the_delay_is_rounded_to_milliseconds() {
        for (numerator, denominator, expected) in [
            (1, 30, 33),
            (2, 30, 67),
            (1, 1, 1000),
            (1001, 30000, 33),
            (1, 10, 100),
            (3, 40, 75),
        ] {
            assert_eq!(
                once(numerator, denominator),
                (expected, false),
                "{numerator}/{denominator}"
            );
        }
    }

    /// 0へ丸まる遅延だけが下限まで切り上げられる
    #[test]
    fn a_delay_below_the_lower_bound_is_raised_and_reported() {
        for (numerator, denominator) in [(0, 30), (1, 10000), (1, 3000)] {
            assert_eq!(
                once(numerator, denominator),
                (MIN_DELAY, true),
                "{numerator}/{denominator}"
            );
        }

        // 下限そのものへ丸まる遅延は切り上げていない
        assert_eq!(once(1, 1000), (MIN_DELAY, false));
        assert_eq!(once(1, 2000), (MIN_DELAY, false));
    }

    /// 30fps は3フレームで100msになる
    ///
    /// 総和は 33.3, 66.7, 100.0, … と進むので、その丸めの差は 33, 34, 33 を繰り返す。
    #[test]
    fn thirty_frames_per_second_cycles_over_three_frames() {
        assert_eq!(repeated(1, 30, 9), [33, 34, 33, 33, 34, 33, 33, 34, 33]);
    }

    /// 30000/1001 fps はミリ秒の刻みで30fpsとずれる
    ///
    /// 総和は 33.37, 66.73, 100.10, … と進み、10フレームで1ms多く積まれる。
    #[test]
    fn the_broadcast_rate_drifts_from_thirty_frames_per_second() {
        assert_eq!(
            repeated(1001, 30000, 9),
            [33, 34, 33, 33, 34, 33, 34, 33, 33]
        );
        assert_eq!(repeated(1001, 30000, 10).iter().sum::<u32>(), 334);
        assert_eq!(repeated(1, 30, 10).iter().sum::<u32>(), 333);
    }

    /// 累積の誤差はフレーム数に依らず1/2msを超えない
    ///
    /// どの区間を取っても総再生時間が素材の時間から半刻み以上ずれない。
    #[test]
    fn the_error_never_grows_with_the_number_of_frames() {
        for (numerator, denominator) in [
            (1, 30),
            (1001, 30000),
            (1, 24),
            (1001, 24000),
            (1, 25),
            (7, 99),
            (1, 3),
        ] {
            let delay = FrameDelay::new(numerator, denominator).unwrap();
            let mut milliseconds = Milliseconds::new();
            let mut total: i128 = 0;
            for frames in 1..=2000i128 {
                let (millisecond, clamped) = milliseconds.next(delay);
                assert!(
                    !clamped,
                    "{numerator}/{denominator} が下限で切り上がっている"
                );
                total += i128::from(millisecond);
                // |total - 1000 * frames * numerator / denominator| <= 1/2
                let ideal = 1000 * frames * i128::from(numerator);
                let denominator = i128::from(denominator);
                assert!(
                    (total * denominator - ideal).abs() * 2 <= denominator,
                    "{numerator}/{denominator} の {frames} フレーム目で誤差が積まれている"
                );
            }
        }
    }

    /// 切り上げたぶんは累積へ戻さない
    #[test]
    fn the_raise_to_the_lower_bound_is_not_fed_back() {
        let mut milliseconds = Milliseconds::new();
        let delays = [(1, 10000), (1, 10), (1, 10)].map(|(numerator, denominator)| {
            milliseconds
                .next(FrameDelay::new(numerator, denominator).unwrap())
                .0
        });
        assert_eq!(delays, [MIN_DELAY, 100, 100]);
    }

    /// 上限を超える遅延は飽和させるが、切り上げとしては数えず、累積へも戻さない
    #[test]
    fn a_long_delay_saturates_at_the_field_width() {
        let mut milliseconds = Milliseconds::new();
        assert_eq!(
            milliseconds.next(FrameDelay::new(100_000, 1).unwrap()),
            (MAX_DELAY, false)
        );
        assert_eq!(
            milliseconds.next(FrameDelay::new(1, 10).unwrap()),
            (100, false)
        );
    }

    /// 分母の最小公倍数が u64 に収まる限り、残差はそのまま持ち越す
    #[test]
    fn the_residual_keeps_its_exact_value() {
        let mut milliseconds = Milliseconds {
            numerator: 1,
            denominator: 30,
        };
        assert_eq!(milliseconds.align(100), 300);
        assert_eq!((milliseconds.numerator, milliseconds.denominator), (1, 30));
    }

    /// 最小公倍数が u64 を超えたら、残差を新しい分母の刻みへ丸め直す
    #[test]
    fn a_residual_beyond_the_common_multiple_is_regrided() {
        // u64::MAX は 7 を約数に持たないので、最小公倍数は7倍になって収まらない
        let mut milliseconds = Milliseconds {
            numerator: (u64::MAX / 2) as i64,
            denominator: u64::MAX,
        };
        assert_eq!(milliseconds.align(7), 7);
        // 1/2 をわずかに下回る残差は 3/7 が最も近い
        assert_eq!((milliseconds.numerator, milliseconds.denominator), (3, 7));
    }
}
