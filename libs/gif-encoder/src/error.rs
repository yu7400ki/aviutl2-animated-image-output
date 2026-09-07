//! エンコード時のエラー

use anim_core::InputError;
use std::fmt;

/// GIFエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 入力の検査に失敗した
    Input(InputError),
    /// 1フレームのバイト数が `usize` で表現できない
    ImageTooLarge { width: u32, height: u32 },
    /// 書き出し先のI/Oエラー
    Io(std::io::Error),
    /// 書き出しに失敗したエンコーダを再利用しようとした
    Poisoned,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Input(e) => e.fmt(f),
            Error::ImageTooLarge { width, height } => {
                write!(f, "画像が大きすぎます: {width}x{height}")
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

impl From<InputError> for Error {
    fn from(e: InputError) -> Self {
        Error::Input(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
