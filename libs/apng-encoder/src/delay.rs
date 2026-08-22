//! フレーム遅延と、fcTLが要求する16bit分数への変換

use crate::error::Error;

/// fcTLのdelay_num・delay_denが取りうる最大値
const MAX: u64 = u16::MAX as u64;

/// フレームの表示時間 (秒)
///
/// `numerator / denominator` 秒を表す。動画のフレームレートから作る場合は
/// 分子にスケール、分母にレートを渡す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDelay {
    numerator: u32,
    denominator: u32,
}

impl FrameDelay {
    /// `numerator / denominator` 秒の遅延を作る
    ///
    /// 分子0は「可能な限り速く」を意味する。
    ///
    /// # Errors
    /// 分母が0のとき [`Error::InvalidFrameDelay`]。
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, Error> {
        if denominator == 0 {
            return Err(Error::InvalidFrameDelay);
        }
        Ok(FrameDelay {
            numerator,
            denominator,
        })
    }

    /// fcTLへ書ける16bit分数 (分子, 分母) へ変換する
    ///
    /// 約分してもu16に収まらない場合は、秒数の誤差が最小になる分数で近似する。
    /// 分子が0でない限り近似後の分子も0にはならない。
    pub fn to_parts(&self) -> (u16, u16) {
        let g = gcd(self.numerator as u64, self.denominator as u64);
        let (num, den) = (self.numerator as u64 / g, self.denominator as u64 / g);

        if num <= MAX && den <= MAX {
            return (num as u16, den as u16);
        }
        approximate(num, den)
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// `num / den` を、分子・分母ともに [`MAX`] 以下の分数で近似する
///
/// 連分数展開の収束分数と半収束分数を候補とし、誤差が最小のものを選ぶ。
/// 有理数をこの制約下で近似する最良解はこの候補集合に含まれる。
fn approximate(num: u64, den: u64) -> (u16, u16) {
    let (mut n, mut d) = (num, den);
    // 直前の2つの収束分数
    let (mut p0, mut q0) = (0u64, 1u64);
    let (mut p1, mut q1) = (1u64, 0u64);
    let mut best: Option<(u64, u64)> = None;

    while d != 0 {
        let a = n / d;
        let (p2, q2) = (a * p1 + p0, a * q1 + q0);

        if p2 > MAX || q2 > MAX {
            // 上限に収まる範囲で最大の半収束分数
            let limit_p = (MAX - p0).checked_div(p1).unwrap_or(a);
            let limit_q = (MAX - q0).checked_div(q1).unwrap_or(a);
            let t = a.min(limit_p).min(limit_q);
            best = better(best, (p0 + t * p1, q0 + t * q1), num, den);
            break;
        }

        best = better(best, (p2, q2), num, den);
        (p0, q0, p1, q1) = (p1, q1, p2, q2);
        (n, d) = (d, n % d);
    }

    match best {
        // 分子0への丸めは「可能な限り速く」の意味になってしまうため最小値へ寄せる
        Some((p, q)) if p > 0 => (p as u16, q as u16),
        _ => (1, MAX as u16),
    }
}

/// 2つの候補のうち `num / den` に近い方を返す
fn better(best: Option<(u64, u64)>, cand: (u64, u64), num: u64, den: u64) -> Option<(u64, u64)> {
    if cand.1 == 0 || cand.0 == 0 {
        return best;
    }
    let Some(best) = best else { return Some(cand) };

    // |p/q - num/den| の比較を、共通の分母を払った整数比較へ落とす
    let error = |(p, q): (u64, u64)| {
        let diff = (p as u128 * den as u128).abs_diff(num as u128 * q as u128);
        (diff, q as u128)
    };
    let (be, bq) = error(best);
    let (ce, cq) = error(cand);

    if ce * bq < be * cq {
        Some(cand)
    } else {
        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(num: u32, den: u32) -> (u16, u16) {
        FrameDelay::new(num, den).unwrap().to_parts()
    }

    #[test]
    fn zero_denominator_is_rejected() {
        assert!(matches!(
            FrameDelay::new(1, 0),
            Err(Error::InvalidFrameDelay)
        ));
    }

    #[test]
    fn values_within_range_are_kept() {
        assert_eq!(parts(1, 30), (1, 30));
        assert_eq!(parts(1001, 30000), (1001, 30000));
        assert_eq!(parts(0, 1), (0, 1));
    }

    #[test]
    fn fractions_are_reduced() {
        assert_eq!(parts(30, 60), (1, 2));
        assert_eq!(parts(1000, 120000), (1, 120));
        assert_eq!(parts(0, 30000), (0, 1));
    }

    /// 60000/1001 fps や 120000/1001 fps の遅延は約分できず、分母がu16を超える
    #[test]
    fn out_of_range_fractions_are_approximated() {
        assert_eq!(parts(1001, 120000), (342, 40999));

        for (num, den) in [(1001, 60000), (1001, 120000)] {
            let (p, q) = parts(num, den);
            let exact = num as f64 / den as f64;
            let approx = p as f64 / q as f64;
            assert!(
                (approx - exact).abs() / exact < 1e-6,
                "{num}/{den} -> {p}/{q}"
            );
        }
    }

    #[test]
    fn approximation_is_the_best_available() {
        // 総当たりで求めた最良の分数と一致すること
        for (num, den) in [(1001u32, 120000u32), (12345, 987654), (65536, 65537)] {
            let (p, q) = parts(num, den);
            let err = |p: u64, q: u64| {
                (p as u128 * den as u128).abs_diff(num as u128 * q as u128) as f64 / q as f64
            };
            let mine = err(p as u64, q as u64);
            for q in 1..=MAX {
                let p = ((num as u128 * q as u128 + den as u128 / 2) / den as u128) as u64;
                if p == 0 || p > MAX {
                    continue;
                }
                assert!(err(p, q) >= mine - 1e-9, "{num}/{den}: {p}/{q} is better");
            }
        }
    }

    #[test]
    fn nonzero_delay_never_rounds_to_zero() {
        assert_eq!(parts(1, u32::MAX), (1, MAX as u16));
        let (p, _) = parts(1, 4_000_000_000);
        assert!(p > 0);
    }

    #[test]
    fn huge_values_are_clamped_to_the_maximum() {
        assert_eq!(parts(u32::MAX, 1), (MAX as u16, 1));
    }
}
