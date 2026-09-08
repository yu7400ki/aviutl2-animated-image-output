//! フレーム遅延の整数の目盛りへの累積丸め

use crate::delay::FrameDelay;

/// フレーム遅延を1秒あたり `scale` 目盛りの整数へ累積で丸める
///
/// フレーム0..N-1 の遅延の総和を `T_N` として、N番目のフレームへ
/// `round(T_{N+1} * scale) - round(T_N * scale)` を割り当てる。丸めは
/// `floor(x + 1/2)` で、整数の平行移動で不変なので、総和そのものを持たずに
/// 「総和を丸めたときの残差」だけで同じ列が出せる。連続するフレームの
/// 割り当てを足したものは、その区間をまとめて丸めた値と一致する。
///
/// 残差は既約分数で持つ。絶対値は常に 1/2 未満なので、フレーム数がいくら
/// 増えても分子・分母は分母の最小公倍数より大きくならない。
pub struct Accumulator {
    /// 1秒あたりの目盛り数
    scale: u32,
    /// 残差の分子 (1目盛り)。絶対値は `denominator / 2` 以下
    numerator: i64,
    /// 残差の分母
    denominator: u64,
}

impl Accumulator {
    /// 1秒を `scale` 目盛りに刻む累積を作る
    pub fn new(scale: u32) -> Self {
        Accumulator {
            scale,
            numerator: 0,
            denominator: 1,
        }
    }

    /// 次のフレームの遅延を目盛りの数へ丸める
    pub fn next(&mut self, delay: FrameDelay) -> u64 {
        let denominator = u64::from(delay.denominator());
        let common = self.align(denominator);
        let scaled = i128::from(self.numerator) * i128::from(common / self.denominator)
            + i128::from(self.scale)
                * i128::from(delay.numerator())
                * i128::from(common / denominator);

        let common = i128::from(common);
        let rounded = (scaled * 2 + common).div_euclid(common * 2);
        self.keep(scaled - rounded * common, common);

        // 残差は 1/2 未満なので、非負の遅延を丸めた値が負になることはない
        rounded as u64
    }

    /// 残差と `denominator` に共通の分母を取る
    ///
    /// 最小公倍数が `u64` に収まらないときは、残差を `denominator` の刻みへ
    /// 丸め直してからその分母を返す。丸めの誤差は `1 / (2 * denominator)`
    /// 目盛りで、分母が互いに素で大きいときにしか起きない。
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

/// `a` と `b` の最大公約数
///
/// 片方が0なら他方、どちらも0なら0を返す。
pub fn gcd(a: impl Into<u128>, b: impl Into<u128>) -> u128 {
    let (mut a, mut b) = (a.into(), b.into());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1秒を `scale` 目盛りに刻み、同じ遅延を `count` フレーム分丸める
    fn repeated(scale: u32, numerator: u32, denominator: u32, count: usize) -> Vec<u64> {
        let delay = FrameDelay::new(numerator, denominator).unwrap();
        let mut accumulator = Accumulator::new(scale);
        (0..count).map(|_| accumulator.next(delay)).collect()
    }

    /// 30fps は3フレームで1/10秒になる
    ///
    /// 総和は 3.33, 6.67, 10.00, … 目盛りと進むので、その丸めの差は 3, 4, 3 を繰り返す。
    #[test]
    fn thirty_frames_per_second_cycles_over_three_frames() {
        for (numerator, denominator) in [(1, 30), (1001, 30000)] {
            assert_eq!(
                repeated(100, numerator, denominator, 9),
                [3, 4, 3, 3, 4, 3, 3, 4, 3],
                "{numerator}/{denominator}"
            );
        }
        assert_eq!(
            repeated(1000, 1, 30, 9),
            [33, 34, 33, 33, 34, 33, 33, 34, 33]
        );
    }

    /// 30000/1001 fps はミリ秒の刻みで30fpsとずれる
    ///
    /// 総和は 33.37, 66.73, 100.10, … と進み、10フレームで1目盛り多く積まれる。
    #[test]
    fn the_broadcast_rate_drifts_from_thirty_frames_per_second() {
        assert_eq!(
            repeated(1000, 1001, 30000, 9),
            [33, 34, 33, 33, 34, 33, 34, 33, 33]
        );
        assert_eq!(repeated(1000, 1001, 30000, 10).iter().sum::<u64>(), 334);
        assert_eq!(repeated(1000, 1, 30, 10).iter().sum::<u64>(), 333);
    }

    /// 連続するフレームの割り当ての和は、区間をまとめて丸めたものと一致する
    #[test]
    fn a_run_of_frames_rounds_at_its_ends() {
        let run: u64 = repeated(1000, 1, 30, 7).iter().sum();
        assert_eq!(run, repeated(1000, 7, 30, 1)[0]);
        assert_eq!(run, 233);
    }

    /// 累積の誤差はフレーム数に依らず半目盛りを超えない
    ///
    /// どの区間を取っても総再生時間が素材の時間から半刻み以上ずれない。
    #[test]
    fn the_error_never_grows_with_the_number_of_frames() {
        for scale in [100u32, 1000] {
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
                let mut accumulator = Accumulator::new(scale);
                let mut total: i128 = 0;
                for frames in 1..=2000i128 {
                    total += i128::from(accumulator.next(delay));
                    // |total - scale * frames * numerator / denominator| <= 1/2
                    let ideal = i128::from(scale) * frames * i128::from(numerator);
                    let denominator = i128::from(denominator);
                    assert!(
                        (total * denominator - ideal).abs() * 2 <= denominator,
                        "目盛り {scale} の {numerator}/{denominator} の {frames} フレーム目で誤差が積まれている"
                    );
                }
            }
        }
    }

    /// 分母の最小公倍数が u64 に収まる限り、残差はそのまま持ち越す
    #[test]
    fn the_residual_keeps_its_exact_value() {
        let mut accumulator = Accumulator {
            scale: 100,
            numerator: 1,
            denominator: 30,
        };
        assert_eq!(accumulator.align(100), 300);
        assert_eq!((accumulator.numerator, accumulator.denominator), (1, 30));
    }

    /// 最小公倍数が u64 を超えたら、残差を新しい分母の刻みへ丸め直す
    #[test]
    fn a_residual_beyond_the_common_multiple_is_regrided() {
        // u64::MAX は 7 を約数に持たないので、最小公倍数は7倍になって収まらない
        let mut accumulator = Accumulator {
            scale: 100,
            numerator: (u64::MAX / 2) as i64,
            denominator: u64::MAX,
        };
        assert_eq!(accumulator.align(7), 7);
        // 1/2 をわずかに下回る残差は 3/7 が最も近い
        assert_eq!((accumulator.numerator, accumulator.denominator), (3, 7));
    }

    #[test]
    fn the_greatest_common_divisor_is_taken() {
        assert_eq!(gcd(12u32, 18u32), 6);
        assert_eq!(gcd(1000u32, 120000u32), 1000);
        assert_eq!(gcd(1001u32, 30000u32), 1);
        assert_eq!(gcd(u64::MAX, 7u64), 1);
        assert_eq!(gcd(u64::MAX, u64::MAX), u128::from(u64::MAX));
    }

    /// 0との最大公約数は相手そのもので、0どうしでは0になる
    #[test]
    fn zero_yields_the_other_side() {
        assert_eq!(gcd(0u32, 30u32), 30);
        assert_eq!(gcd(30u32, 0u32), 30);
        assert_eq!(gcd(0u32, 0u32), 0);
    }
}
