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

/// エンコーダへ投入された入力が検査を通らなかった
///
/// 受け付けられる寸法の範囲は [`InputError::InvalidDimensions`] を返す側が持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    /// 幅または高さが受け付けられる範囲の外
    InvalidDimensions { width: u32, height: u32 },
    /// フレーム数が0
    InvalidFrameCount,
    /// フレームのバイト数が `幅 * 高さ * チャンネル数` と一致しない
    FrameSizeMismatch { expected: usize, actual: usize },
    /// 投入されたフレーム数が宣言したフレーム数と一致しない
    FrameCountMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::InvalidDimensions { width, height } => {
                write!(f, "画像サイズが不正です: {width}x{height}")
            }
            InputError::InvalidFrameCount => write!(f, "フレーム数は1以上である必要があります"),
            InputError::FrameSizeMismatch { expected, actual } => {
                write!(
                    f,
                    "フレームのバイト数が一致しません: {expected} バイト必要ですが {actual} バイトです"
                )
            }
            InputError::FrameCountMismatch { expected, actual } => {
                write!(
                    f,
                    "フレーム数が一致しません: 宣言 {expected}、投入 {actual}"
                )
            }
        }
    }
}

impl std::error::Error for InputError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5つのエンコーダが同じ文面を出す
    #[test]
    fn each_input_error_keeps_its_wording() {
        assert_eq!(
            InputError::InvalidDimensions {
                width: 0,
                height: 4
            }
            .to_string(),
            "画像サイズが不正です: 0x4"
        );
        assert_eq!(
            InputError::InvalidFrameCount.to_string(),
            "フレーム数は1以上である必要があります"
        );
        assert_eq!(
            InputError::FrameSizeMismatch {
                expected: 48,
                actual: 47
            }
            .to_string(),
            "フレームのバイト数が一致しません: 48 バイト必要ですが 47 バイトです"
        );
        assert_eq!(
            InputError::FrameCountMismatch {
                expected: 3,
                actual: 1
            }
            .to_string(),
            "フレーム数が一致しません: 宣言 3、投入 1"
        );
    }
}
