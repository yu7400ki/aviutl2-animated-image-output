//! エンコード時のエラー

use std::ffi::c_int;
use std::fmt;

/// WebPエンコード中に発生するエラー
#[derive(Debug)]
pub enum Error {
    /// 幅または高さが0、またはフレームの上限16383を超えている
    InvalidDimensions { width: u32, height: u32 },
    /// フレーム数が0
    InvalidFrameCount,
    /// フレームのバイト数が `幅 * 高さ * チャンネル数` と一致しない
    FrameSizeMismatch { expected: usize, actual: usize },
    /// 投入されたフレーム数が宣言したフレーム数と一致しない
    FrameCountMismatch { expected: u32, actual: u32 },
    /// ファイルサイズがRIFFの上限4GiBを超えた
    FileTooLarge,
    /// libwebpの符号化が失敗した
    Encode(EncodingError),
    /// 符号化された単葉の復号が失敗した
    Decode(DecodingError),
    /// 符号化された単葉のチャンク構成を読み取れなかった
    MalformedOutput,
    /// 書き出し先のI/Oエラー
    Io(std::io::Error),
    /// 書き出しに失敗したエンコーダを再利用しようとした
    Poisoned,
}

/// libwebpが `WebPPicture::error_code` に置く失敗の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingError {
    /// 画素や補助構造のメモリが確保できない
    OutOfMemory,
    /// 符号化した内容を溜めるメモリが確保できない
    BitstreamOutOfMemory,
    /// 引数がNULL
    NullParameter,
    /// `WebPConfig` の項目が値域の外
    InvalidConfiguration,
    /// 幅または高さが符号化器の範囲外
    BadDimension,
    /// パーティション0が512KiBを超えた
    Partition0Overflow,
    /// パーティションが16MiBを超えた
    PartitionOverflow,
    /// 書き出しの関数が失敗を返した
    BadWrite,
    /// 符号化した内容が4GiBを超えた
    FileTooBig,
    /// 進捗の関数が中断を返した
    UserAbort,
    /// libwebpが上のいずれでもない値を置いた
    Unknown(c_int),
}

/// 単葉を復号したときの失敗の種別
///
/// 復号器は失敗の内訳を持たないので、返らなかったことと、返った寸法が
/// 求めた矩形と違ったことを分ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodingError {
    /// 復号器が画素を返さなかった
    Refused,
    /// 復号した寸法が求めた矩形と違う
    SizeMismatch {
        expected: (u32, u32),
        actual: (u32, u32),
    },
}

impl EncodingError {
    /// `WebPPicture::error_code` を写す
    pub(crate) fn from_code(code: c_int) -> Self {
        match code {
            webp_sys::VP8_ENC_ERROR_OUT_OF_MEMORY => EncodingError::OutOfMemory,
            webp_sys::VP8_ENC_ERROR_BITSTREAM_OUT_OF_MEMORY => EncodingError::BitstreamOutOfMemory,
            webp_sys::VP8_ENC_ERROR_NULL_PARAMETER => EncodingError::NullParameter,
            webp_sys::VP8_ENC_ERROR_INVALID_CONFIGURATION => EncodingError::InvalidConfiguration,
            webp_sys::VP8_ENC_ERROR_BAD_DIMENSION => EncodingError::BadDimension,
            webp_sys::VP8_ENC_ERROR_PARTITION0_OVERFLOW => EncodingError::Partition0Overflow,
            webp_sys::VP8_ENC_ERROR_PARTITION_OVERFLOW => EncodingError::PartitionOverflow,
            webp_sys::VP8_ENC_ERROR_BAD_WRITE => EncodingError::BadWrite,
            webp_sys::VP8_ENC_ERROR_FILE_TOO_BIG => EncodingError::FileTooBig,
            webp_sys::VP8_ENC_ERROR_USER_ABORT => EncodingError::UserAbort,
            code => EncodingError::Unknown(code),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidDimensions { width, height } => {
                write!(f, "画像サイズが不正です: {width}x{height}")
            }
            Error::InvalidFrameCount => write!(f, "フレーム数は1以上である必要があります"),
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
            Error::FileTooLarge => write!(f, "ファイルサイズが4GiBを超えました"),
            Error::Encode(e) => write!(f, "符号化に失敗しました: {e}"),
            Error::Decode(e) => write!(f, "復号に失敗しました: {e}"),
            Error::MalformedOutput => {
                write!(f, "符号化された画像のチャンク構成を読み取れません")
            }
            Error::Io(e) => write!(f, "書き出しに失敗しました: {e}"),
            Error::Poisoned => write!(f, "書き出しに失敗したエンコーダは再利用できません"),
        }
    }
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodingError::OutOfMemory => write!(f, "メモリが確保できません"),
            EncodingError::BitstreamOutOfMemory => {
                write!(f, "ビットストリームのメモリが確保できません")
            }
            EncodingError::NullParameter => write!(f, "引数がNULLです"),
            EncodingError::InvalidConfiguration => write!(f, "符号化の設定が不正です"),
            EncodingError::BadDimension => write!(f, "画像サイズが符号化器の範囲外です"),
            EncodingError::Partition0Overflow => write!(f, "パーティション0が512KiBを超えました"),
            EncodingError::PartitionOverflow => write!(f, "パーティションが16MiBを超えました"),
            EncodingError::BadWrite => write!(f, "符号化した内容を書き出せません"),
            EncodingError::FileTooBig => write!(f, "符号化した内容が4GiBを超えました"),
            EncodingError::UserAbort => write!(f, "符号化が中断されました"),
            EncodingError::Unknown(code) => write!(f, "未知のエラーです: {code}"),
        }
    }
}

impl fmt::Display for DecodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodingError::Refused => write!(f, "画素を取り出せません"),
            DecodingError::SizeMismatch { expected, actual } => write!(
                f,
                "復号した画像サイズが一致しません: {}x{} のはずが {}x{} です",
                expected.0, expected.1, actual.0, actual.1
            ),
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

impl std::error::Error for DecodingError {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_code_maps_to_its_own_variant() {
        let mapped: Vec<EncodingError> = (1..=10).map(EncodingError::from_code).collect();
        assert_eq!(
            mapped,
            [
                EncodingError::OutOfMemory,
                EncodingError::BitstreamOutOfMemory,
                EncodingError::NullParameter,
                EncodingError::InvalidConfiguration,
                EncodingError::BadDimension,
                EncodingError::Partition0Overflow,
                EncodingError::PartitionOverflow,
                EncodingError::BadWrite,
                EncodingError::FileTooBig,
                EncodingError::UserAbort,
            ]
        );
    }

    #[test]
    fn a_code_outside_the_documented_range_is_kept_as_is() {
        assert_eq!(
            EncodingError::from_code(webp_sys::VP8_ENC_OK),
            EncodingError::Unknown(0)
        );
        assert_eq!(EncodingError::from_code(11), EncodingError::Unknown(11));
    }
}
