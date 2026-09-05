//! `JxlEncoderStatus` / `JxlEncoderError` の写像と、入力検査のエラー

use jxl_sys::{
    JXL_ENC_ERR_API_USAGE, JXL_ENC_ERR_BAD_INPUT, JXL_ENC_ERR_GENERIC, JXL_ENC_ERR_JBRD,
    JXL_ENC_ERR_NOT_SUPPORTED, JXL_ENC_ERR_OK, JXL_ENC_ERR_OOM,
};
use std::ffi::c_int;
use std::fmt;

/// JPEG XLエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 幅または高さが0
    InvalidDimensions { width: u32, height: u32 },
    /// フレーム数が0
    InvalidFrameCount,
    /// 1秒あたりのtick数が0を含むか、約した比が書ける値域の外
    InvalidTps { numerator: u32, denominator: u32 },
    /// フレームの表示時間が0
    InvalidDuration,
    /// 品質が値域の外
    InvalidQuality { quality: f32 },
    /// 速度と圧縮率の均衡が値域の外
    InvalidEffort { effort: u8 },
    /// フレームのバイト数が `幅 * 高さ * チャンネル数` と一致しない
    FrameSizeMismatch { expected: usize, actual: usize },
    /// 投入されたフレーム数が宣言したフレーム数と一致しない
    FrameCountMismatch { expected: u32, actual: u32 },
    /// libjxlの符号化が失敗した
    Encode(EncodingError),
    /// 書き出し先のI/Oエラー
    Io(std::io::Error),
}

/// libjxlが保持する失敗の内訳
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Ok,
    Generic,
    OutOfMemory,
    JpegBitstreamReconstruction,
    BadInput,
    NotSupported,
    ApiUsage,
    /// 列挙に無い値
    Unknown(c_int),
}

impl ErrorCode {
    /// `JxlEncoderError` の値を写す
    fn from_raw(code: c_int) -> Self {
        match code {
            JXL_ENC_ERR_OK => ErrorCode::Ok,
            JXL_ENC_ERR_GENERIC => ErrorCode::Generic,
            JXL_ENC_ERR_OOM => ErrorCode::OutOfMemory,
            JXL_ENC_ERR_JBRD => ErrorCode::JpegBitstreamReconstruction,
            JXL_ENC_ERR_BAD_INPUT => ErrorCode::BadInput,
            JXL_ENC_ERR_NOT_SUPPORTED => ErrorCode::NotSupported,
            JXL_ENC_ERR_API_USAGE => ErrorCode::ApiUsage,
            code => ErrorCode::Unknown(code),
        }
    }
}

/// libjxlが返した失敗
///
/// `JxlEncoderStatus` に、符号化器が保持する内訳を添える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingError {
    status: c_int,
    code: ErrorCode,
}

impl EncodingError {
    /// 失敗した `JxlEncoderStatus` と `JxlEncoderError` を組にする
    pub(crate) fn new(status: c_int, code: c_int) -> Self {
        EncodingError {
            status,
            code: ErrorCode::from_raw(code),
        }
    }

    /// libjxlが返した `JxlEncoderStatus`
    pub fn status(&self) -> c_int {
        self.status
    }

    /// 失敗の内訳
    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidDimensions { width, height } => {
                write!(f, "画像サイズが不正です: {width}x{height}")
            }
            Error::InvalidFrameCount => write!(f, "フレーム数は1以上である必要があります"),
            Error::InvalidTps {
                numerator,
                denominator,
            } => write!(
                f,
                "1秒あたりのtick数を書けません: {numerator}/{denominator} (約分した分子が1〜1073741824、分母が1〜1024に収まる必要があります)"
            ),
            Error::InvalidDuration => write!(f, "表示時間は1以上である必要があります"),
            Error::InvalidQuality { quality } => {
                write!(f, "品質は0.0以上100.0以下である必要があります: {quality}")
            }
            Error::InvalidEffort { effort } => {
                write!(f, "均衡は1以上10以下である必要があります: {effort}")
            }
            Error::FrameSizeMismatch { expected, actual } => write!(
                f,
                "フレームのバイト数が一致しません: {expected} バイト必要ですが {actual} バイトです"
            ),
            Error::FrameCountMismatch { expected, actual } => {
                write!(
                    f,
                    "フレーム数が一致しません: 宣言 {expected}、投入 {actual}"
                )
            }
            Error::Encode(e) => write!(f, "符号化に失敗しました: {e}"),
            Error::Io(e) => write!(f, "書き出しに失敗しました: {e}"),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::Ok => f.write_str("JXL_ENC_ERR_OK"),
            ErrorCode::Generic => f.write_str("JXL_ENC_ERR_GENERIC"),
            ErrorCode::OutOfMemory => f.write_str("JXL_ENC_ERR_OOM"),
            ErrorCode::JpegBitstreamReconstruction => f.write_str("JXL_ENC_ERR_JBRD"),
            ErrorCode::BadInput => f.write_str("JXL_ENC_ERR_BAD_INPUT"),
            ErrorCode::NotSupported => f.write_str("JXL_ENC_ERR_NOT_SUPPORTED"),
            ErrorCode::ApiUsage => f.write_str("JXL_ENC_ERR_API_USAGE"),
            ErrorCode::Unknown(code) => write!(f, "未知の内訳 ({code})"),
        }
    }
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (JxlEncoderStatus = {})", self.code, self.status)
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

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jxl_sys::JXL_ENC_ERROR;

    /// 列挙の全域が名前を持つ
    #[test]
    fn every_enumerated_code_maps_to_its_name() {
        let named = [
            (JXL_ENC_ERR_OK, ErrorCode::Ok, "JXL_ENC_ERR_OK"),
            (
                JXL_ENC_ERR_GENERIC,
                ErrorCode::Generic,
                "JXL_ENC_ERR_GENERIC",
            ),
            (JXL_ENC_ERR_OOM, ErrorCode::OutOfMemory, "JXL_ENC_ERR_OOM"),
            (
                JXL_ENC_ERR_JBRD,
                ErrorCode::JpegBitstreamReconstruction,
                "JXL_ENC_ERR_JBRD",
            ),
            (
                JXL_ENC_ERR_BAD_INPUT,
                ErrorCode::BadInput,
                "JXL_ENC_ERR_BAD_INPUT",
            ),
            (
                JXL_ENC_ERR_NOT_SUPPORTED,
                ErrorCode::NotSupported,
                "JXL_ENC_ERR_NOT_SUPPORTED",
            ),
            (
                JXL_ENC_ERR_API_USAGE,
                ErrorCode::ApiUsage,
                "JXL_ENC_ERR_API_USAGE",
            ),
        ];
        for (raw, code, name) in named {
            assert_eq!(ErrorCode::from_raw(raw), code, "{name}");
            assert_eq!(code.to_string(), name);
        }
    }

    /// 列挙から外れた値も潰さずに運ぶ
    #[test]
    fn a_code_outside_the_enumeration_keeps_its_value() {
        let code = ErrorCode::from_raw(9999);
        assert_eq!(code, ErrorCode::Unknown(9999));
        assert_eq!(code.to_string(), "未知の内訳 (9999)");
    }

    #[test]
    fn the_status_is_shown_beside_the_code() {
        let e = EncodingError::new(JXL_ENC_ERROR, JXL_ENC_ERR_BAD_INPUT);
        assert_eq!(e.status(), JXL_ENC_ERROR);
        assert_eq!(e.code(), ErrorCode::BadInput);
        assert_eq!(
            e.to_string(),
            "JXL_ENC_ERR_BAD_INPUT (JxlEncoderStatus = 1)"
        );
    }
}
