//! `avifImage` / `avifRGBImage` / `avifRWData` のRAII

use crate::error::{EncodingError, Error};
use crate::layout::Layout;
use anim_core::ColorType;
use avif_sys::{
    AVIF_COLOR_PRIMARIES_BT709, AVIF_MATRIX_COEFFICIENTS_BT601, AVIF_PIXEL_FORMAT_YUV420,
    AVIF_PIXEL_FORMAT_YUV422, AVIF_PIXEL_FORMAT_YUV444, AVIF_RANGE_FULL, AVIF_RESULT_OK,
    AVIF_RESULT_OUT_OF_MEMORY, AVIF_RGB_FORMAT_RGB, AVIF_RGB_FORMAT_RGBA,
    AVIF_TRANSFER_CHARACTERISTICS_SRGB, avifImage, avifImageCreate, avifImageDestroy,
    avifImageRGBToYUV, avifRGBImage, avifRGBImageSetDefaults, avifRWData, avifRWDataFree,
};
use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::ptr;
use std::slice;

/// クロマサブサンプリング
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvFormat {
    Yuv420,
    Yuv422,
    Yuv444,
}

impl YuvFormat {
    /// `avifPixelFormat` へ写す
    fn to_raw(self) -> c_int {
        match self {
            YuvFormat::Yuv420 => AVIF_PIXEL_FORMAT_YUV420,
            YuvFormat::Yuv422 => AVIF_PIXEL_FORMAT_YUV422,
            YuvFormat::Yuv444 => AVIF_PIXEL_FORMAT_YUV444,
        }
    }
}

/// 符号化へ渡す1フレーム
///
/// 画素を持つ道は [`Image::import`] だけで、YUVの各面もα面も
/// `avifImageRGBToYUV` が確保する。α面が作られるのは入力がαを持つときに限られる。
pub(crate) struct Image {
    raw: *mut avifImage,
}

impl Image {
    /// `layout` の1フレームを取り込む
    ///
    /// 色はBT.709の原色・sRGBの伝達特性・BT.601の色行列・full rangeとして書く。
    ///
    /// # Errors
    /// `data` の長さが1フレームぶんと違うとき [`Error::Input`]。
    /// 画像を確保できないか変換に失敗したとき [`Error::Encode`]。
    pub(crate) fn import(
        data: &[u8],
        layout: &Layout,
        yuv_format: YuvFormat,
    ) -> Result<Self, Error> {
        layout.check_frame(data)?;

        let raw = unsafe { avifImageCreate(layout.width, layout.height, 8, yuv_format.to_raw()) };
        if raw.is_null() {
            return Err(Error::Encode(EncodingError::new(
                AVIF_RESULT_OUT_OF_MEMORY,
                String::new(),
            )));
        }
        let image = Image { raw };

        unsafe {
            (*raw).yuvRange = AVIF_RANGE_FULL;
            (*raw).colorPrimaries = AVIF_COLOR_PRIMARIES_BT709;
            (*raw).transferCharacteristics = AVIF_TRANSFER_CHARACTERISTICS_SRGB;
            (*raw).matrixCoefficients = AVIF_MATRIX_COEFFICIENTS_BT601;
        }

        let mut rgb = MaybeUninit::<avifRGBImage>::zeroed();
        unsafe { avifRGBImageSetDefaults(rgb.as_mut_ptr(), raw) };
        let mut rgb = unsafe { rgb.assume_init() };
        rgb.format = match layout.color_type {
            ColorType::Rgb8 => AVIF_RGB_FORMAT_RGB,
            ColorType::Rgba8 => AVIF_RGB_FORMAT_RGBA,
        };
        rgb.pixels = data.as_ptr().cast_mut();
        rgb.rowBytes = layout.stride;

        let result = unsafe { avifImageRGBToYUV(raw, &rgb) };
        if result != AVIF_RESULT_OK {
            return Err(Error::Encode(EncodingError::new(result, String::new())));
        }

        Ok(image)
    }

    /// `avifEncoderAddImage` へ渡す、この画像の位置
    pub(crate) fn as_ptr(&self) -> *const avifImage {
        self.raw
    }

    /// α面を持つか
    #[cfg(test)]
    pub(crate) fn has_alpha(&self) -> bool {
        !unsafe { (*self.raw).alphaPlane }.is_null()
    }

    /// 変換先の `avifPixelFormat`
    #[cfg(test)]
    pub(crate) fn raw_yuv_format(&self) -> c_int {
        unsafe { (*self.raw).yuvFormat }
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        unsafe { avifImageDestroy(self.raw) };
    }
}

/// libavifが組み立てたバイト列
pub(crate) struct RwData {
    raw: avifRWData,
}

impl RwData {
    pub(crate) fn new() -> Self {
        RwData {
            raw: avifRWData {
                data: ptr::null_mut(),
                size: 0,
            },
        }
    }

    /// `avifEncoderFinish` へ渡す、この領域の位置
    pub(crate) fn as_mut_ptr(&mut self) -> *mut avifRWData {
        &raw mut self.raw
    }

    /// 組み立てられたバイト列
    pub(crate) fn as_slice(&self) -> &[u8] {
        if self.raw.data.is_null() {
            return &[];
        }
        unsafe { slice::from_raw_parts(self.raw.data, self.raw.size) }
    }
}

impl Drop for RwData {
    fn drop(&mut self) {
        unsafe { avifRWDataFree(&raw mut self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_core::InputError;

    #[test]
    fn a_buffer_of_another_length_is_refused_before_the_conversion() {
        let layout = Layout::new(2, 2, ColorType::Rgba8).unwrap();
        assert!(matches!(
            Image::import(&[0; 15], &layout, YuvFormat::Yuv420),
            Err(Error::Input(InputError::FrameSizeMismatch {
                expected: 16,
                actual: 15
            }))
        ));
    }

    /// α面の有無は取り込んだ画素の色種別だけで決まる
    #[test]
    fn the_alpha_plane_follows_the_input_color_type() {
        let rgb = Layout::new(16, 16, ColorType::Rgb8).unwrap();
        assert!(
            !Image::import(&vec![0; rgb.frame_len], &rgb, YuvFormat::Yuv420)
                .unwrap()
                .has_alpha()
        );

        let rgba = Layout::new(16, 16, ColorType::Rgba8).unwrap();
        assert!(
            Image::import(&vec![0; rgba.frame_len], &rgba, YuvFormat::Yuv420)
                .unwrap()
                .has_alpha()
        );
    }

    /// クロマサブサンプリングの選択が変換先の画素形式に届く
    #[test]
    fn the_chroma_subsampling_reaches_the_converted_image() {
        let layout = Layout::new(16, 16, ColorType::Rgb8).unwrap();
        let data = vec![0; layout.frame_len];
        for (yuv_format, expected) in [
            (YuvFormat::Yuv420, AVIF_PIXEL_FORMAT_YUV420),
            (YuvFormat::Yuv422, AVIF_PIXEL_FORMAT_YUV422),
            (YuvFormat::Yuv444, AVIF_PIXEL_FORMAT_YUV444),
        ] {
            let image = Image::import(&data, &layout, yuv_format).unwrap();
            assert_eq!(image.raw_yuv_format(), expected, "{yuv_format:?}");
        }
    }

    #[test]
    fn an_unwritten_area_yields_no_bytes() {
        assert!(RwData::new().as_slice().is_empty());
    }
}
