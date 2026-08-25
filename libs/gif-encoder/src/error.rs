//! エンコード時のエラー

use std::fmt;

/// GIFエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 幅または高さが0、または論理画面の上限65535を超えている
    InvalidDimensions { width: u32, height: u32 },
    /// フレーム数が0
    InvalidFrameCount,
    /// フレーム数が1でない
    UnsupportedFrameCount(u32),
    /// 1フレームのバイト数が `usize` で表現できない
    ImageTooLarge { width: u32, height: u32 },
    /// フレーム遅延の分母が0
    InvalidFrameDelay,
    /// フレームのバイト数が `幅 * 高さ * チャンネル数` と一致しない
    FrameSizeMismatch { expected: usize, actual: usize },
    /// 投入されたフレーム数が宣言したフレーム数と一致しない
    FrameCountMismatch { expected: u32, actual: u32 },
    /// フレームに現れた色がカラーテーブルの上限256を超えている
    TooManyColors,
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
            Error::UnsupportedFrameCount(count) => {
                write!(f, "フレーム数 {count} は扱えません: 1フレームのみ扱えます")
            }
            Error::ImageTooLarge { width, height } => {
                write!(f, "画像が大きすぎます: {width}x{height}")
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
            Error::TooManyColors => {
                write!(f, "色数がカラーテーブルの上限256を超えています")
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
