//! libavif のエンコード経路への FFI
//!
//! 宣言は同梱した libavif v1.3.0 の `include/avif/avif.h` に対応する。安全な抽象は置かず、
//! 符号化に要る関数と、その受け渡しに現れる構造体だけを写す。
//!
//! `avifEncoder` と `avifImage` は libavif が確保するため、読み書きする項目だけを名前で持ち、
//! 残りは間隔を合わせる詰め物にしてある。どれも Rust から参照を作るので、項目の位置に加えて
//! 大きさも同梱ヘッダと一致させ、テストで検める。

#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, c_void};

/// `avifDiagnostics::error` の長さ
pub const AVIF_DIAGNOSTICS_ERROR_BUFFER_SIZE: usize = 256;

/// `avifEncoder::repetitionCount` に置く無限ループ
pub const AVIF_REPETITION_COUNT_INFINITE: c_int = -1;

/// `avifResult`
pub const AVIF_RESULT_OK: c_int = 0;
pub const AVIF_RESULT_UNKNOWN_ERROR: c_int = 1;
pub const AVIF_RESULT_INVALID_FTYP: c_int = 2;
pub const AVIF_RESULT_NO_CONTENT: c_int = 3;
pub const AVIF_RESULT_NO_YUV_FORMAT_SELECTED: c_int = 4;
pub const AVIF_RESULT_REFORMAT_FAILED: c_int = 5;
pub const AVIF_RESULT_UNSUPPORTED_DEPTH: c_int = 6;
pub const AVIF_RESULT_ENCODE_COLOR_FAILED: c_int = 7;
pub const AVIF_RESULT_ENCODE_ALPHA_FAILED: c_int = 8;
pub const AVIF_RESULT_BMFF_PARSE_FAILED: c_int = 9;
pub const AVIF_RESULT_MISSING_IMAGE_ITEM: c_int = 10;
pub const AVIF_RESULT_DECODE_COLOR_FAILED: c_int = 11;
pub const AVIF_RESULT_DECODE_ALPHA_FAILED: c_int = 12;
pub const AVIF_RESULT_COLOR_ALPHA_SIZE_MISMATCH: c_int = 13;
pub const AVIF_RESULT_ISPE_SIZE_MISMATCH: c_int = 14;
pub const AVIF_RESULT_NO_CODEC_AVAILABLE: c_int = 15;
pub const AVIF_RESULT_NO_IMAGES_REMAINING: c_int = 16;
pub const AVIF_RESULT_INVALID_EXIF_PAYLOAD: c_int = 17;
pub const AVIF_RESULT_INVALID_IMAGE_GRID: c_int = 18;
pub const AVIF_RESULT_INVALID_CODEC_SPECIFIC_OPTION: c_int = 19;
pub const AVIF_RESULT_TRUNCATED_DATA: c_int = 20;
pub const AVIF_RESULT_IO_NOT_SET: c_int = 21;
pub const AVIF_RESULT_IO_ERROR: c_int = 22;
pub const AVIF_RESULT_WAITING_ON_IO: c_int = 23;
pub const AVIF_RESULT_INVALID_ARGUMENT: c_int = 24;
pub const AVIF_RESULT_NOT_IMPLEMENTED: c_int = 25;
pub const AVIF_RESULT_OUT_OF_MEMORY: c_int = 26;
pub const AVIF_RESULT_CANNOT_CHANGE_SETTING: c_int = 27;
pub const AVIF_RESULT_INCOMPATIBLE_IMAGE: c_int = 28;
pub const AVIF_RESULT_INTERNAL_ERROR: c_int = 29;
pub const AVIF_RESULT_ENCODE_GAIN_MAP_FAILED: c_int = 30;
pub const AVIF_RESULT_DECODE_GAIN_MAP_FAILED: c_int = 31;
pub const AVIF_RESULT_INVALID_TONE_MAPPED_IMAGE: c_int = 32;

/// `avifPixelFormat` — `avifImage::yuvFormat`
pub const AVIF_PIXEL_FORMAT_NONE: c_int = 0;
pub const AVIF_PIXEL_FORMAT_YUV444: c_int = 1;
pub const AVIF_PIXEL_FORMAT_YUV422: c_int = 2;
pub const AVIF_PIXEL_FORMAT_YUV420: c_int = 3;
pub const AVIF_PIXEL_FORMAT_YUV400: c_int = 4;

/// `avifRGBFormat` — `avifRGBImage::format`
pub const AVIF_RGB_FORMAT_RGB: c_int = 0;
pub const AVIF_RGB_FORMAT_RGBA: c_int = 1;

/// `avifRange` — `avifImage::yuvRange`
pub const AVIF_RANGE_LIMITED: c_int = 0;
pub const AVIF_RANGE_FULL: c_int = 1;

/// `avifColorPrimaries` — sRGB / BT.709 の原色
pub const AVIF_COLOR_PRIMARIES_BT709: u16 = 1;

/// `avifTransferCharacteristics` — sRGB の伝達特性
pub const AVIF_TRANSFER_CHARACTERISTICS_SRGB: u16 = 13;

/// `avifMatrixCoefficients` — BT.601 の色行列
pub const AVIF_MATRIX_COEFFICIENTS_BT601: u16 = 6;

/// `avifAddImageFlags`
pub const AVIF_ADD_IMAGE_FLAG_NONE: u32 = 0;
/// 単葉として符号化する。表示時間は無視される
pub const AVIF_ADD_IMAGE_FLAG_SINGLE: u32 = 1 << 1;

/// libavif が確保して返すバイト列。解放は [`avifRWDataFree`]
#[repr(C)]
pub struct avifRWData {
    pub data: *mut u8,
    pub size: usize,
}

/// 直前の書き出しが積んだ AV1 ストリームの大きさ
#[repr(C)]
#[derive(Clone, Copy)]
pub struct avifIOStats {
    pub colorOBUSize: usize,
    pub alphaOBUSize: usize,
}

/// 直前の失敗の詳細。[`avifResultToString`] が返す総称名より具体的な文言が入る
#[repr(C)]
pub struct avifDiagnostics {
    pub error: [c_char; AVIF_DIAGNOSTICS_ERROR_BUFFER_SIZE],
}

/// 符号化へ渡す画像。確保と解放は [`avifImageCreate`] と [`avifImageDestroy`]
#[repr(C)]
pub struct avifImage {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub yuvFormat: c_int,
    pub yuvRange: c_int,
    pad1: [c_int; 1],
    pad2: [*mut u8; 3],
    pad3: [u32; 4],

    /// [`avifImageRGBToYUV`] は RGB 形式が α を持つときだけここを確保する
    pub alphaPlane: *mut u8,
    pad4: [u32; 3],
    pad5: [usize; 2],

    pub colorPrimaries: u16,
    pub transferCharacteristics: u16,
    pub matrixCoefficients: u16,
    pad6: [u16; 2],
    pad7: [u32; 11],
    pad8: [u8; 2],
    pad9: [usize; 4],
    pad10: [usize; 3],
}

/// 取り込む RGB 画素。初期化は [`avifRGBImageSetDefaults`] で行う
#[repr(C)]
pub struct avifRGBImage {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: c_int,
    pub chromaUpsampling: c_int,
    pub chromaDownsampling: c_int,
    pub avoidLibYUV: c_int,
    pub ignoreAlpha: c_int,
    pub alphaPremultiplied: c_int,
    pub isFloat: c_int,
    /// YUV→RGB でのみ使われる
    pub maxThreads: c_int,

    pub pixels: *mut u8,
    /// `pixels` の行間。バイトで数える
    pub rowBytes: u32,
}

/// 符号化器。確保と解放は [`avifEncoderCreate`] と [`avifEncoderDestroy`]
#[repr(C)]
pub struct avifEncoder {
    pad1: [c_int; 1],
    pub maxThreads: c_int,
    pub speed: c_int,
    pad2: [c_int; 1],
    /// 1秒あたりの時間刻み数
    pub timescale: u64,
    /// 追加の繰り返し回数。n で n+1 回再生、[`AVIF_REPETITION_COUNT_INFINITE`] で無限
    pub repetitionCount: c_int,
    pad3: [u32; 1],
    pub quality: c_int,
    pub qualityAlpha: c_int,
    pad4: [c_int; 11],

    pub ioStats: avifIOStats,
    pub diag: avifDiagnostics,

    pad5: [*mut c_void; 2],
    pad6: [c_int; 2],
}

unsafe extern "C" {
    /// 主・副・改訂を点で繋いだ版数
    pub fn avifVersion() -> *const c_char;

    /// `result` に対応する総称名
    pub fn avifResultToString(result: c_int) -> *const c_char;

    /// `raw.data` を解放する。`raw` 自体は解放しない
    pub fn avifRWDataFree(raw: *mut avifRWData);

    /// 失敗すれば NULL
    pub fn avifImageCreate(width: u32, height: u32, depth: u32, yuvFormat: c_int)
    -> *mut avifImage;

    pub fn avifImageDestroy(image: *mut avifImage);

    /// `rgb` を `image` の寸法・深度と既定値で埋める
    pub fn avifRGBImageSetDefaults(rgb: *mut avifRGBImage, image: *const avifImage);

    /// `rgb` を `image` へ変換する。要るプレーンはこの関数が確保する
    pub fn avifImageRGBToYUV(image: *mut avifImage, rgb: *const avifRGBImage) -> c_int;

    /// 失敗すれば NULL
    pub fn avifEncoderCreate() -> *mut avifEncoder;

    /// `image` を符号化して積む。`durationInTimescales` は `encoder.timescale` 刻み
    pub fn avifEncoderAddImage(
        encoder: *mut avifEncoder,
        image: *const avifImage,
        durationInTimescales: u64,
        addImageFlags: u32,
    ) -> c_int;

    /// 積んだ画像をファイル全体へ組み立てる。成功したら `output` を [`avifRWDataFree`] で解放する
    pub fn avifEncoderFinish(encoder: *mut avifEncoder, output: *mut avifRWData) -> c_int;

    pub fn avifEncoderDestroy(encoder: *mut avifEncoder);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::mem::{MaybeUninit, offset_of, size_of};

    unsafe extern "C" {
        fn avif_sys_sizeof_encoder() -> usize;
        fn avif_sys_sizeof_image() -> usize;
        fn avif_sys_sizeof_rgb_image() -> usize;
        fn avif_sys_sizeof_rw_data() -> usize;
        fn avif_sys_sizeof_diagnostics() -> usize;
        fn avif_sys_encoder_offsets(out: *mut usize);
        fn avif_sys_image_offsets(out: *mut usize);
        fn avif_sys_rgb_image_offsets(out: *mut usize);
        fn avif_sys_rw_data_offsets(out: *mut usize);
        fn avif_sys_io_stats_offsets(out: *mut usize);
    }

    /// 並べた順に項目の位置を取る
    macro_rules! offsets {
        ($ty:ty, $($field:ident),+ $(,)?) => {
            [$(offset_of!($ty, $field)),+]
        };
    }

    /// C 側の出口が同じ順で並べた位置
    fn from_header<const N: usize>(fill: unsafe extern "C" fn(*mut usize)) -> [usize; N] {
        let mut offsets = [0usize; N];
        unsafe { fill(offsets.as_mut_ptr()) };
        offsets
    }

    #[test]
    fn the_linked_library_reports_v1_3_0() {
        let version = unsafe { CStr::from_ptr(avifVersion()) };
        assert_eq!(version.to_str(), Ok("1.3.0"));
    }

    /// 写した構造体は同梱ヘッダと同じ大きさになる
    ///
    /// 位置の突き合わせは末尾の詰め物を見ないため、大きさを別に検める。libavif が
    /// 確保する構造体でも `&mut` を作る以上、確保された領域はこの大きさだけ要る。
    #[test]
    fn struct_sizes_match_the_vendored_header() {
        assert_eq!(size_of::<avifEncoder>(), unsafe {
            avif_sys_sizeof_encoder()
        });
        assert_eq!(size_of::<avifImage>(), unsafe { avif_sys_sizeof_image() });
        assert_eq!(size_of::<avifRGBImage>(), unsafe {
            avif_sys_sizeof_rgb_image()
        });
        assert_eq!(size_of::<avifRWData>(), unsafe {
            avif_sys_sizeof_rw_data()
        });
        assert_eq!(size_of::<avifDiagnostics>(), unsafe {
            avif_sys_sizeof_diagnostics()
        });
    }

    #[test]
    fn encoder_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(avif_sys_encoder_offsets),
            offsets!(
                avifEncoder,
                maxThreads,
                speed,
                timescale,
                repetitionCount,
                quality,
                qualityAlpha,
                ioStats,
                diag,
            )
        );
    }

    #[test]
    fn image_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(avif_sys_image_offsets),
            offsets!(
                avifImage,
                width,
                height,
                depth,
                yuvFormat,
                yuvRange,
                alphaPlane,
                colorPrimaries,
                transferCharacteristics,
                matrixCoefficients,
            )
        );
    }

    #[test]
    fn rgb_image_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(avif_sys_rgb_image_offsets),
            offsets!(
                avifRGBImage,
                width,
                height,
                depth,
                format,
                chromaUpsampling,
                chromaDownsampling,
                avoidLibYUV,
                ignoreAlpha,
                alphaPremultiplied,
                isFloat,
                maxThreads,
                pixels,
                rowBytes,
            )
        );
    }

    #[test]
    fn rw_data_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(avif_sys_rw_data_offsets),
            offsets!(avifRWData, data, size)
        );
    }

    #[test]
    fn io_stats_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(avif_sys_io_stats_offsets),
            offsets!(avifIOStats, colorOBUSize, alphaOBUSize)
        );
    }

    /// 手詰めを置き換える初期化。項目の追加にも既定値で追従する
    #[test]
    fn set_defaults_fills_an_rgb_image_from_the_image() {
        let image = unsafe { avifImageCreate(48, 32, 8, AVIF_PIXEL_FORMAT_YUV420) };
        assert!(!image.is_null());

        let mut rgb = MaybeUninit::<avifRGBImage>::zeroed();
        unsafe { avifRGBImageSetDefaults(rgb.as_mut_ptr(), image) };
        let rgb = unsafe { rgb.assume_init() };

        assert_eq!((rgb.width, rgb.height, rgb.depth), (48, 32, 8));
        assert_eq!(rgb.format, AVIF_RGB_FORMAT_RGBA);
        assert_eq!(rgb.maxThreads, 1);
        assert_eq!(rgb.isFloat, 0);
        assert_eq!(rgb.alphaPremultiplied, 0);
        assert!(rgb.pixels.is_null());
        assert_eq!(rgb.rowBytes, 0);

        unsafe { avifImageDestroy(image) };
    }

    #[test]
    fn a_created_image_carries_the_requested_format() {
        let image = unsafe { avifImageCreate(16, 16, 8, AVIF_PIXEL_FORMAT_YUV444) };
        assert!(!image.is_null());
        let read = unsafe { &*image };
        assert_eq!((read.width, read.height, read.depth), (16, 16, 8));
        assert_eq!(read.yuvFormat, AVIF_PIXEL_FORMAT_YUV444);
        assert!(read.alphaPlane.is_null());
        unsafe { avifImageDestroy(image) };
    }

    #[test]
    fn a_created_encoder_starts_without_a_diagnostic() {
        let encoder = unsafe { avifEncoderCreate() };
        assert!(!encoder.is_null());
        let read = unsafe { &*encoder };
        assert_eq!(read.repetitionCount, AVIF_REPETITION_COUNT_INFINITE);
        assert_eq!(read.diag.error[0], 0);
        assert_eq!(read.ioStats.colorOBUSize, 0);
        unsafe { avifEncoderDestroy(encoder) };
    }

    #[test]
    fn result_names_come_from_the_linked_library() {
        let ok = unsafe { CStr::from_ptr(avifResultToString(AVIF_RESULT_OK)) };
        assert_eq!(ok.to_str(), Ok("OK"));
        let invalid = unsafe { CStr::from_ptr(avifResultToString(AVIF_RESULT_INVALID_ARGUMENT)) };
        assert_eq!(invalid.to_str(), Ok("Invalid argument"));
    }
}
