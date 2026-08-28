//! 遅延時間のミリ秒への変換

use anim_core::FrameDelay;

/// 表示時間の下限 (ms)
///
/// 0の解釈はデコーダ間で揃わないため、それを避ける下限を置く。
const MIN_DURATION: u32 = 1;

/// ANMFの表示時間の欄に収まる上限 (ms)
const MAX_DURATION: u32 = 0x00FF_FFFF;

/// フレーム遅延をミリ秒へ累積で丸める
///
/// フレーム0..N-1 の遅延の総和を `T_N` として、N番目のフレームへ
/// `round(T_{N+1} * 1000) - round(T_N * 1000)` を割り当てる。丸めは
/// `floor(x + 1/2)` で、整数の平行移動で不変なので、総和そのものを持たずに
/// 「総和を丸めたときの残差」だけで同じ列が出せる。
///
/// 連続するフレームの割り当てを足すと中間の境界が消え、両端の丸めだけが残る。
/// フレームを併合しても、丸めの区間は書き出すフレームの境界で取られる。
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

    /// 次のフレームの遅延をミリ秒へ変換する
    pub(crate) fn next(&mut self, delay: FrameDelay) -> u64 {
        let denominator = u64::from(delay.denominator());
        let common = self.align(denominator);
        let scaled = i128::from(self.numerator) * i128::from(common / self.denominator)
            + 1000 * i128::from(delay.numerator()) * i128::from(common / denominator);

        let common = i128::from(common);
        let rounded = (scaled * 2 + common).div_euclid(common * 2);
        self.keep(scaled - rounded * common, common);

        // 残差は 1/2 未満なので、非負の遅延を丸めた値が負になることはない
        rounded as u64
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

/// 表示時間をANMFの欄に収まる列へ分ける
///
/// 先頭を書き出すフレームが載せ、続きはキャンバスを書き換えないフレームへ回す。
/// 列は必ず1つ以上になる。
pub(crate) struct Durations {
    /// まだ載せていない表示時間 (ms)
    remaining: u64,
    /// 下限まで切り上げたか
    pub(crate) raised: bool,
}

impl Durations {
    /// `total` ミリ秒を分ける
    ///
    /// 下限に満たない時間は下限まで切り上げる。切り上げたぶんは累積へ戻さない。
    pub(crate) fn new(total: u64) -> Self {
        Durations {
            remaining: total.max(u64::from(MIN_DURATION)),
            raised: total < u64::from(MIN_DURATION),
        }
    }
}

impl Iterator for Durations {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.remaining == 0 {
            return None;
        }
        let duration = self.remaining.min(u64::from(MAX_DURATION));
        self.remaining -= duration;
        Some(duration as u32)
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
    fn once(numerator: u32, denominator: u32) -> u64 {
        Milliseconds::new().next(FrameDelay::new(numerator, denominator).unwrap())
    }

    /// 同じ遅延を `count` フレーム分変換する
    fn repeated(numerator: u32, denominator: u32, count: usize) -> Vec<u64> {
        let delay = FrameDelay::new(numerator, denominator).unwrap();
        let mut milliseconds = Milliseconds::new();
        (0..count).map(|_| milliseconds.next(delay)).collect()
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
                expected,
                "{numerator}/{denominator}"
            );
        }
    }

    /// ミリ秒に満たない遅延は0のまま返り、下限は分ける側が置く
    #[test]
    fn a_delay_below_a_millisecond_rounds_to_zero() {
        for (numerator, denominator) in [(0, 30), (1, 10000), (1, 3000)] {
            assert_eq!(once(numerator, denominator), 0, "{numerator}/{denominator}");
        }
        assert_eq!(once(1, 1000), 1);
        assert_eq!(once(1, 2000), 1);
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
        assert_eq!(repeated(1001, 30000, 10).iter().sum::<u64>(), 334);
        assert_eq!(repeated(1, 30, 10).iter().sum::<u64>(), 333);
    }

    /// 連続するフレームの割り当ての和は、区間をまとめて丸めたものと一致する
    #[test]
    fn a_run_of_frames_rounds_at_its_ends() {
        let delay = FrameDelay::new(1, 30).unwrap();
        let mut milliseconds = Milliseconds::new();
        let run: u64 = (0..7).map(|_| milliseconds.next(delay)).sum();
        assert_eq!(run, once(7, 30));
        assert_eq!(run, 233);
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
                total += i128::from(milliseconds.next(delay));
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

    /// 欄に収まる表示時間は1つのまま
    #[test]
    fn a_duration_within_the_field_width_stays_on_one_frame() {
        for total in [1, 40, u64::from(MAX_DURATION)] {
            let durations = Durations::new(total);
            assert!(!durations.raised, "{total}");
            assert_eq!(durations.collect::<Vec<_>>(), [total as u32], "{total}");
        }
    }

    /// 欄に収まらない表示時間は、収まる列へ分けて総和を保つ
    #[test]
    fn a_duration_beyond_the_field_width_is_split_without_losing_time() {
        for total in [
            u64::from(MAX_DURATION) + 1,
            u64::from(MAX_DURATION) * 2,
            u64::from(MAX_DURATION) * 3 + 5,
        ] {
            let split: Vec<u32> = Durations::new(total).collect();
            assert!(split.len() > 1, "{total}");
            assert!(split.iter().all(|&d| d <= MAX_DURATION), "{total}");
            assert_eq!(split.iter().map(|&d| u64::from(d)).sum::<u64>(), total);
        }
        assert_eq!(
            Durations::new(u64::from(MAX_DURATION) * 2 + 5).collect::<Vec<_>>(),
            [MAX_DURATION, MAX_DURATION, 5]
        );
    }

    /// 0へ丸まった表示時間だけが下限まで切り上げられる
    #[test]
    fn a_duration_below_the_lower_bound_is_raised_and_reported() {
        let durations = Durations::new(0);
        assert!(durations.raised);
        assert_eq!(durations.collect::<Vec<_>>(), [MIN_DURATION]);

        assert!(!Durations::new(u64::from(MIN_DURATION)).raised);
    }

    /// 下限までの切り上げは累積の外で起き、残差へ戻らない
    #[test]
    fn the_raise_to_the_lower_bound_is_not_fed_back() {
        let mut milliseconds = Milliseconds::new();
        let increments = [(1, 10000), (1, 10), (1, 10)].map(|(numerator, denominator)| {
            milliseconds.next(FrameDelay::new(numerator, denominator).unwrap())
        });
        assert_eq!(increments, [0, 100, 100]);
    }
}
