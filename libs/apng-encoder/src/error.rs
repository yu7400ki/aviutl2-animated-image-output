//! エンコード時のエラー

use anim_core::InputError;
use std::fmt;

/// APNGエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 入力の検査に失敗した
    Input(InputError),
    /// 1フレームのバイト数が `usize` で表現できない
    ImageTooLarge { width: u32, height: u32 },
    /// 圧縮レベルが 1..=9 の範囲外
    InvalidCompressionLevel(u32),
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
            Error::Input(e) => e.fmt(f),
            Error::ImageTooLarge { width, height } => {
                write!(f, "画像が大きすぎます: {width}x{height}")
            }
            Error::InvalidCompressionLevel(level) => {
                write!(f, "圧縮レベル {level} は 1..=9 の範囲外です")
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
