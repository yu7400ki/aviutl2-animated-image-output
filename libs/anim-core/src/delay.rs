//! フレームの表示時間

use crate::error::Error;

/// フレームの表示時間 (秒)
///
/// `numerator / denominator` 秒を表す。動画のフレームレートから作る場合は
/// 分子にスケール、分母にレートを渡す。値は与えられた分数のまま保つので、
/// 書き出す単位への変換は受け取った側で行う。
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

    /// `numerator / denominator` 秒の分子
    pub fn numerator(&self) -> u32 {
        self.numerator
    }

    /// `numerator / denominator` 秒の分母
    pub fn denominator(&self) -> u32 {
        self.denominator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_denominator_is_rejected() {
        assert!(matches!(
            FrameDelay::new(1, 0),
            Err(Error::InvalidFrameDelay)
        ));
    }

    #[test]
    fn the_fraction_is_kept_as_it_was_given() {
        let delay = FrameDelay::new(1001, 30000).unwrap();
        assert_eq!((delay.numerator(), delay.denominator()), (1001, 30000));
    }
}
