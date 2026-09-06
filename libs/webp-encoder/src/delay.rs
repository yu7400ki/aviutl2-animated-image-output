//! 遅延時間のミリ秒への変換

use anim_core::{Accumulator, FrameDelay};

/// 表示時間の下限 (ms)
const MIN_DURATION: u32 = 1;

/// ANMFの表示時間の欄に収まる上限 (ms)
pub(crate) const MAX_DURATION: u32 = 0x00FF_FFFF;

/// フレーム遅延をミリ秒へ累積で丸める
pub(crate) struct Milliseconds(Accumulator);

impl Milliseconds {
    pub(crate) fn new() -> Self {
        Milliseconds(Accumulator::new(1000))
    }

    /// 次のフレームの遅延をミリ秒へ変換する
    pub(crate) fn next(&mut self, delay: FrameDelay) -> u64 {
        self.0.next(delay)
    }
}

/// 表示時間をANMFの欄に収まる列へ分ける
///
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
    /// 下限に満たない時間は下限まで切り上げる。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 1フレームだけを変換する
    fn once(numerator: u32, denominator: u32) -> u64 {
        Milliseconds::new().next(FrameDelay::new(numerator, denominator).unwrap())
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
