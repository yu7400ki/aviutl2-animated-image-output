//! `avifResult` の写像と、入力検査のエラー

use anim_core::InputError;
use std::ffi::{CStr, c_int};
use std::fmt;

/// AVIFエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 入力の検査に失敗した
    Input(InputError),
    /// 1秒あたりの時間刻み数が0
    InvalidTimescale,
    /// フレームの表示時間が0
    InvalidDuration,
    /// 品質が値域の外
    InvalidQuality { quality: u8 },
    /// 速度と圧縮率の均衡が値域の外
    InvalidSpeed { speed: u8 },
    /// libavifの符号化が失敗した
    Encode(EncodingError),
    /// 書き出し先のI/Oエラー
    Io(std::io::Error),
}

/// libavifが返した失敗
///
/// `avifResult` の総称名に、符号化器が書いた具体的な原因を添える。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingError {
    code: c_int,
    name: String,
    detail: String,
}

impl EncodingError {
    /// `code` の総称名をlibavifから引き、`detail` を添える
    pub(crate) fn new(code: c_int, detail: String) -> Self {
        let name = unsafe { CStr::from_ptr(avif_sys::avifResultToString(code)) }
            .to_string_lossy()
            .into_owned();
        EncodingError { code, name, detail }
    }

    /// libavifが返した `avifResult`
    pub fn code(&self) -> c_int {
        self.code
    }

    /// `avifResult` の総称名
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 符号化器が書いた原因。書かれていなければ空
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Input(e) => e.fmt(f),
            Error::InvalidTimescale => write!(f, "時間刻み数は1以上である必要があります"),
            Error::InvalidDuration => write!(f, "表示時間は1以上である必要があります"),
            Error::InvalidQuality { quality } => {
                write!(f, "品質は0以上100以下である必要があります: {quality}")
            }
            Error::InvalidSpeed { speed } => {
                write!(f, "速度は0以上10以下である必要があります: {speed}")
            }
            Error::Encode(e) => write!(f, "符号化に失敗しました: {e}"),
            Error::Io(e) => write!(f, "書き出しに失敗しました: {e}"),
        }
    }
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{} ({})", self.name, self.detail)
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

impl std::error::Error for EncodingError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generic_name_comes_from_the_linked_library() {
        let e = EncodingError::new(avif_sys::AVIF_RESULT_INVALID_ARGUMENT, String::new());
        assert_eq!(e.code(), avif_sys::AVIF_RESULT_INVALID_ARGUMENT);
        assert_eq!(e.name(), "Invalid argument");
        assert_eq!(e.to_string(), "Invalid argument");
    }

    /// 総称名だけでは検証失敗の中身が潰れるので、原因を並べて出す
    #[test]
    fn the_detail_is_shown_beside_the_generic_name() {
        let e = EncodingError::new(
            avif_sys::AVIF_RESULT_ENCODE_COLOR_FAILED,
            "aom_codec_encode() failed".to_owned(),
        );
        assert_eq!(
            e.to_string(),
            "Encoding of color planes failed (aom_codec_encode() failed)"
        );
    }

    #[test]
    fn a_code_outside_the_enumeration_still_has_a_name() {
        assert!(!EncodingError::new(9999, String::new()).name().is_empty());
    }
}
