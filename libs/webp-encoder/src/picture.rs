//! `WebPPicture` と `WebPMemoryWriter` のRAII

use crate::error::{EncodingError, Error};
use crate::layout::ColorType;
use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;
use std::slice;
use webp_sys::{
    WebPConfig, WebPEncode, WebPMemoryWrite, WebPMemoryWriter, WebPMemoryWriterClear,
    WebPMemoryWriterInit, WebPPicture, WebPPictureFree, WebPPictureImportRGB,
    WebPPictureImportRGBA, WebPPictureInitARGB,
};

/// 画素を取り込んだ `WebPPicture`
///
/// 画素を持つ道は [`Picture::import`] だけで、[`Picture::encode`] が値を消費する。
/// 1つの picture が受け取る画素は、ARGB入力として取り込んだ1フレームに限られる。
pub(crate) struct Picture {
    raw: WebPPicture,
}

impl Picture {
    /// `width` x `height` の連続バッファをARGB入力として取り込む
    ///
    /// # Errors
    /// `data` の長さが `width * height * 1画素のバイト数` と違うとき
    /// [`Error::FrameSizeMismatch`]。取り込みに失敗したとき [`Error::Encode`]。
    pub(crate) fn import(
        data: &[u8],
        width: u32,
        height: u32,
        color_type: ColorType,
    ) -> Result<Self, Error> {
        let stride = width as usize * color_type.bytes_per_pixel();
        let expected = stride * height as usize;
        if data.len() != expected {
            return Err(Error::FrameSizeMismatch {
                expected,
                actual: data.len(),
            });
        }

        let (Ok(width), Ok(height), Ok(stride)) = (
            c_int::try_from(width),
            c_int::try_from(height),
            c_int::try_from(stride),
        ) else {
            return Err(Error::Encode(EncodingError::BadDimension));
        };

        let mut raw = MaybeUninit::<WebPPicture>::uninit();
        if unsafe { WebPPictureInitARGB(raw.as_mut_ptr()) } == 0 {
            return Err(Error::Encode(EncodingError::InvalidConfiguration));
        }
        let mut picture = Picture {
            raw: unsafe { raw.assume_init() },
        };
        picture.raw.width = width;
        picture.raw.height = height;

        let imported = unsafe {
            match color_type {
                ColorType::Rgb8 => {
                    WebPPictureImportRGB(&raw mut picture.raw, data.as_ptr(), stride)
                }
                ColorType::Rgba8 => {
                    WebPPictureImportRGBA(&raw mut picture.raw, data.as_ptr(), stride)
                }
            }
        };
        if imported == 0 {
            return Err(Error::Encode(EncodingError::from_code(
                picture.raw.error_code,
            )));
        }

        Ok(picture)
    }

    /// 単葉の .webp へ符号化する
    ///
    /// # Errors
    /// 符号化に失敗したとき [`Error::Encode`]。
    pub(crate) fn encode(self, config: &WebPConfig) -> Result<Vec<u8>, Error> {
        let mut picture = self;
        let mut writer = MemoryWriter::new();
        picture.raw.writer = Some(WebPMemoryWrite);
        picture.raw.custom_ptr = writer.as_mut_ptr().cast::<c_void>();

        if unsafe { WebPEncode(config, &raw mut picture.raw) } == 0 {
            return Err(Error::Encode(EncodingError::from_code(
                picture.raw.error_code,
            )));
        }

        Ok(writer.to_vec())
    }
}

impl Drop for Picture {
    fn drop(&mut self) {
        unsafe { WebPPictureFree(&raw mut self.raw) };
    }
}

/// libwebpが符号化した内容を書き足す領域
struct MemoryWriter {
    raw: WebPMemoryWriter,
}

impl MemoryWriter {
    fn new() -> Self {
        let mut raw = MaybeUninit::<WebPMemoryWriter>::uninit();
        unsafe { WebPMemoryWriterInit(raw.as_mut_ptr()) };
        MemoryWriter {
            raw: unsafe { raw.assume_init() },
        }
    }

    /// `WebPPicture::custom_ptr` へ渡す、この領域の位置
    fn as_mut_ptr(&mut self) -> *mut WebPMemoryWriter {
        &raw mut self.raw
    }

    /// 書き足されたバイト列を複製する
    fn to_vec(&self) -> Vec<u8> {
        if self.raw.size == 0 {
            return Vec::new();
        }
        unsafe { slice::from_raw_parts(self.raw.mem, self.raw.size) }.to_vec()
    }
}

impl Drop for MemoryWriter {
    fn drop(&mut self) {
        unsafe { WebPMemoryWriterClear(&raw mut self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_of_another_length_is_refused_before_the_import() {
        assert!(matches!(
            Picture::import(&[0; 15], 2, 2, ColorType::Rgba8),
            Err(Error::FrameSizeMismatch {
                expected: 16,
                actual: 15
            })
        ));
    }

    #[test]
    fn an_empty_canvas_is_refused_by_the_library() {
        assert!(matches!(
            Picture::import(&[], 0, 0, ColorType::Rgba8),
            Err(Error::Encode(EncodingError::BadDimension))
        ));
    }

    #[test]
    fn an_unwritten_memory_writer_yields_no_bytes() {
        assert!(MemoryWriter::new().to_vec().is_empty());
    }
}
