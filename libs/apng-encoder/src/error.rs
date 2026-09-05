//! エンコード時のエラー

use std::fmt;

/// APNGエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 幅または高さが0
    InvalidDimensions { width: u32, height: u32 },
    /// フレーム数が0
    InvalidFrameCount,
    /// 1フレームのバイト数が `usize` で表現できない
    ImageTooLarge { width: u32, height: u32 },
    /// 圧縮レベルが 1..=9 の範囲外
    InvalidCompressionLevel(u32),
    /// フレーム遅延の分母が0
    InvalidFrameDelay,
    /// フレームのバイト数が `幅 * 高さ * チャンネル数` と一致しない
    FrameSizeMismatch { expected: usize, actual: usize },
    /// 投入されたフレーム数が宣言したフレーム数と一致しない
    FrameCountMismatch { expected: u32, actual: u32 },
    /// チャンク長がPNGの上限を超えた
    ChunkTooLarge { len: usize },
    /// 書き出し先のI/Oエラー
    Io(std::io::Error),
    /// 書き出しに失敗したエンコーダを再利用しようとした
    Poisoned,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidDimensions { width, height } => {
                write!(f, "画像サイズが不正です: {width}x{height}")
            }
            Error::InvalidFrameCount => write!(f, "フレーム数は1以上である必要があります"),
            Error::ImageTooLarge { width, height } => {
                write!(f, "画像が大きすぎます: {width}x{height}")
            }
            Error::InvalidCompressionLevel(level) => {
                write!(f, "圧縮レベル {level} は 1..=9 の範囲外です")
            }
            Error::InvalidFrameDelay => write!(f, "フレーム遅延の分母が0です"),
            Error::FrameSizeMismatch { expected, actual } => {
                write!(
                    f,
                    "フレームのバイト数が一致しません: {expected} バイト必要ですが {actual} バイトです"
                )
            }
            Error::FrameCountMismatch { expected, actual } => {
                write!(
                    f,
                    "フレーム数が一致しません: 宣言 {expected}、投入 {actual}"
                )
            }
            Error::ChunkTooLarge { len } => {
                write!(f, "チャンク長がPNGの上限を超えました: {len} バイト")
            }
            Error::Io(e) => write!(f, "書き出しに失敗しました: {e}"),
            Error::Poisoned => write!(f, "書き出しに失敗したエンコーダは再利用できません"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<anim_core::Error> for Error {
    fn from(e: anim_core::Error) -> Self {
        match e {
            anim_core::Error::InvalidFrameDelay => Error::InvalidFrameDelay,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
