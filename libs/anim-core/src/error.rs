//! 共有する部品が返すエラー

use std::fmt;

/// 引数が受け付けられる値の範囲から外れている
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// フレーム遅延の分母が0
    InvalidFrameDelay,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidFrameDelay => write!(f, "フレーム遅延の分母が0です"),
        }
    }
}

impl std::error::Error for Error {}
