//! libwebp の符号化と復号への FFI
//!
//! 宣言は同梱した libwebp v1.6.0 の `src/webp/encode.h`・`src/webp/decode.h`・
//! `src/webp/types.h` に対応する。安全な抽象は置かず、`WebPPicture` の入力経路の
//! 選択だけを型で閉じる。

#![allow(non_snake_case)]

use std::ffi::{c_float, c_int, c_void};

/// `WebPConfig` と `WebPPicture` の並びを同定する版数
const WEBP_ENCODER_ABI_VERSION: c_int = 0x0210;

/// VP8/VP8L のフレームが取りうる幅・高さの上限
pub const WEBP_MAX_DIMENSION: c_int = 16383;

/// `WebPPicture::error_code`
pub const VP8_ENC_OK: c_int = 0;
pub const VP8_ENC_ERROR_OUT_OF_MEMORY: c_int = 1;
pub const VP8_ENC_ERROR_BITSTREAM_OUT_OF_MEMORY: c_int = 2;
pub const VP8_ENC_ERROR_NULL_PARAMETER: c_int = 3;
pub const VP8_ENC_ERROR_INVALID_CONFIGURATION: c_int = 4;
pub const VP8_ENC_ERROR_BAD_DIMENSION: c_int = 5;
pub const VP8_ENC_ERROR_PARTITION0_OVERFLOW: c_int = 6;
pub const VP8_ENC_ERROR_PARTITION_OVERFLOW: c_int = 7;
pub const VP8_ENC_ERROR_BAD_WRITE: c_int = 8;
pub const VP8_ENC_ERROR_FILE_TOO_BIG: c_int = 9;
pub const VP8_ENC_ERROR_USER_ABORT: c_int = 10;

/// `WebPConfigInit` が使う既定のプリセット
const WEBP_PRESET_DEFAULT: c_int = 0;

/// `WebPConfigInit` が使う既定の品質
const DEFAULT_QUALITY: c_float = 75.0;

/// 符号化のパラメータ
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WebPConfig {
    pub lossless: c_int,
    pub quality: c_float,
    pub method: c_int,
    pub image_hint: c_int,
    pub target_size: c_int,
    pub target_PSNR: c_float,
    pub segments: c_int,
    pub sns_strength: c_int,
    pub filter_strength: c_int,
    pub filter_sharpness: c_int,
    pub filter_type: c_int,
    pub autofilter: c_int,
    pub alpha_compression: c_int,
    pub alpha_filtering: c_int,
    pub alpha_quality: c_int,
    pub pass: c_int,
    pub show_compressed: c_int,
    pub preprocessing: c_int,
    pub partitions: c_int,
    pub partition_limit: c_int,
    pub emulate_jpeg_size: c_int,
    pub thread_level: c_int,
    pub low_memory: c_int,
    pub near_lossless: c_int,
    pub exact: c_int,
    pub use_delta_palette: c_int,
    pub use_sharp_yuv: c_int,
    pub qmin: c_int,
    pub qmax: c_int,
}

/// 符号化されたバイト列を受け取る関数
pub type WebPWriterFunction =
    unsafe extern "C" fn(data: *const u8, data_size: usize, picture: *const WebPPicture) -> c_int;

/// `WebPMemoryWrite` が書き足していく可変長のバッファ
#[repr(C)]
pub struct WebPMemoryWriter {
    pub mem: *mut u8,
    pub size: usize,
    pub max_size: usize,
    pad: [u32; 1],
}

/// 入力画素と符号化の結果を受け渡す構造体
///
/// 呼び側から `use_argb` は書けない。立てるのは [`WebPPictureInitARGB`] で、
/// 非可逆の [`WebPEncode`] は入力を YUV へ移したうえでこれを倒す。
#[repr(C)]
pub struct WebPPicture {
    use_argb: c_int,

    pub colorspace: c_int,
    pub width: c_int,
    pub height: c_int,
    pub y: *mut u8,
    pub u: *mut u8,
    pub v: *mut u8,
    pub y_stride: c_int,
    pub uv_stride: c_int,
    pub a: *mut u8,
    pub a_stride: c_int,
    pad1: [u32; 2],

    /// 1画素4バイトの ARGB。並びはリトルエンディアンの `0xAARRGGBB`
    pub argb: *mut u32,
    /// `argb` の行間。バイトではなく画素で数える
    pub argb_stride: c_int,
    pad2: [u32; 3],

    pub writer: Option<WebPWriterFunction>,
    pub custom_ptr: *mut c_void,

    pub extra_info_type: c_int,
    pub extra_info: *mut u8,

    pub stats: *mut c_void,
    pub error_code: c_int,
    pub progress_hook:
        Option<unsafe extern "C" fn(percent: c_int, picture: *const WebPPicture) -> c_int>,
    pub user_data: *mut c_void,
    pad3: [u32; 3],
    pad4: *mut u8,
    pad5: *mut u8,
    pad6: [u32; 8],

    memory: *mut c_void,
    memory_argb: *mut c_void,
    pad7: [*mut c_void; 2],
}

unsafe extern "C" {
    fn WebPConfigInitInternal(
        config: *mut WebPConfig,
        preset: c_int,
        quality: c_float,
        version: c_int,
    ) -> c_int;

    fn WebPPictureInitInternal(picture: *mut WebPPicture, version: c_int) -> c_int;

    /// 各項目が値域に収まっていれば非0
    pub fn WebPValidateConfig(config: *const WebPConfig) -> c_int;

    /// `rgba` を `picture` へ取り込む。`rgba_stride` はバイトで数える
    pub fn WebPPictureImportRGBA(
        picture: *mut WebPPicture,
        rgba: *const u8,
        rgba_stride: c_int,
    ) -> c_int;

    /// `rgb` を不透明な画素として取り込む。`rgb_stride` はバイトで数える
    pub fn WebPPictureImportRGB(
        picture: *mut WebPPicture,
        rgb: *const u8,
        rgb_stride: c_int,
    ) -> c_int;

    /// `picture` が抱える画素の領域を解放する
    pub fn WebPPictureFree(picture: *mut WebPPicture);

    /// `picture` を符号化し、`picture.writer` へ単葉の .webp を流す
    pub fn WebPEncode(config: *const WebPConfig, picture: *mut WebPPicture) -> c_int;

    pub fn WebPMemoryWriterInit(writer: *mut WebPMemoryWriter);

    /// `writer.mem` を解放する。`writer` 自体は解放しない
    pub fn WebPMemoryWriterClear(writer: *mut WebPMemoryWriter);

    /// `picture.custom_ptr` が指す [`WebPMemoryWriter`] へ書き足す
    pub fn WebPMemoryWrite(data: *const u8, data_size: usize, picture: *const WebPPicture)
    -> c_int;

    /// 主・副・改訂を各8bitに詰めた版数
    pub fn WebPGetEncoderVersion() -> c_int;

    /// 単葉の .webp を復号し、走査順に並べた RGBA と寸法を返す。失敗すれば NULL。
    /// 返った領域は [`WebPFree`] で解放する
    pub fn WebPDecodeRGBA(
        data: *const u8,
        data_size: usize,
        width: *mut c_int,
        height: *mut c_int,
    ) -> *mut u8;

    /// [`WebPDecodeRGBA`] が返した領域を解放する
    pub fn WebPFree(ptr: *mut c_void);
}

/// `config` を既定値で初期化する。版数が合わなければ 0
///
/// # Safety
///
/// `config` は `WebPConfig` を置ける領域を指していること。
pub unsafe fn WebPConfigInit(config: *mut WebPConfig) -> c_int {
    unsafe {
        WebPConfigInitInternal(
            config,
            WEBP_PRESET_DEFAULT,
            DEFAULT_QUALITY,
            WEBP_ENCODER_ABI_VERSION,
        )
    }
}

/// `picture` を初期化し、入力経路を ARGB にする。版数が合わなければ 0
///
/// ARGB 入力は `WebPPictureImportRGBA` / `WebPPictureImportRGB` が置く並びを
/// そのまま符号化器へ渡す。非可逆の [`WebPEncode`] は符号化の途中で入力を
/// YUV へ移し `use_argb` を倒すため、`picture` を次のフレームへ使い回すには
/// この関数で初期化し直すこと。零で埋めた領域も ARGB 入力を指さない。
///
/// # Safety
///
/// `picture` は `WebPPicture` を置ける領域を指していること。
pub unsafe fn WebPPictureInitARGB(picture: *mut WebPPicture) -> c_int {
    unsafe {
        let ok = WebPPictureInitInternal(picture, WEBP_ENCODER_ABI_VERSION);
        if ok != 0 {
            (*picture).use_argb = 1;
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{MaybeUninit, offset_of, size_of};

    unsafe extern "C" {
        fn webp_sys_sizeof_config() -> usize;
        fn webp_sys_sizeof_picture() -> usize;
        fn webp_sys_sizeof_memory_writer() -> usize;
        fn webp_sys_config_offsets(out: *mut usize);
        fn webp_sys_picture_offsets(out: *mut usize);
        fn webp_sys_memory_writer_offsets(out: *mut usize);
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
    fn the_linked_library_reports_v1_6_0() {
        assert_eq!(unsafe { WebPGetEncoderVersion() }, 0x01_06_00);
    }

    #[test]
    fn struct_sizes_match_the_vendored_header() {
        assert_eq!(size_of::<WebPConfig>(), unsafe { webp_sys_sizeof_config() });
        assert_eq!(size_of::<WebPPicture>(), unsafe {
            webp_sys_sizeof_picture()
        });
        assert_eq!(size_of::<WebPMemoryWriter>(), unsafe {
            webp_sys_sizeof_memory_writer()
        });
    }

    #[test]
    fn config_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(webp_sys_config_offsets),
            offsets!(
                WebPConfig,
                lossless,
                quality,
                method,
                image_hint,
                target_size,
                target_PSNR,
                segments,
                sns_strength,
                filter_strength,
                filter_sharpness,
                filter_type,
                autofilter,
                alpha_compression,
                alpha_filtering,
                alpha_quality,
                pass,
                show_compressed,
                preprocessing,
                partitions,
                partition_limit,
                emulate_jpeg_size,
                thread_level,
                low_memory,
                near_lossless,
                exact,
                use_delta_palette,
                use_sharp_yuv,
                qmin,
                qmax,
            )
        );
    }

    #[test]
    fn picture_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(webp_sys_picture_offsets),
            offsets!(
                WebPPicture,
                use_argb,
                colorspace,
                width,
                height,
                y,
                u,
                v,
                y_stride,
                uv_stride,
                a,
                a_stride,
                argb,
                argb_stride,
                writer,
                custom_ptr,
                extra_info_type,
                extra_info,
                stats,
                error_code,
                progress_hook,
                user_data,
            )
        );
    }

    #[test]
    fn memory_writer_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(webp_sys_memory_writer_offsets),
            offsets!(WebPMemoryWriter, mem, size, max_size)
        );
    }

    #[test]
    fn an_initialized_picture_selects_argb_input() {
        let mut picture = MaybeUninit::<WebPPicture>::uninit();
        assert_ne!(unsafe { WebPPictureInitARGB(picture.as_mut_ptr()) }, 0);
        assert_eq!(unsafe { picture.assume_init() }.use_argb, 1);
    }

    #[test]
    fn an_initialized_config_is_valid() {
        let mut config = MaybeUninit::<WebPConfig>::uninit();
        assert_ne!(unsafe { WebPConfigInit(config.as_mut_ptr()) }, 0);
        assert_ne!(unsafe { WebPValidateConfig(config.as_ptr()) }, 0);
    }

    /// `WebPPictureInitARGB` の説明が言う倒れ方を押さえる。使い回しの前に
    /// 初期化し直す要求はここから来る
    #[test]
    fn a_lossy_encode_leaves_the_picture_off_the_argb_input() {
        let (width, height) = (32, 32);
        let rgba = vec![0x40u8; (width * height * 4) as usize];

        let mut config = MaybeUninit::<WebPConfig>::uninit();
        assert_ne!(unsafe { WebPConfigInit(config.as_mut_ptr()) }, 0);
        let config = unsafe { config.assume_init() };

        let mut picture = MaybeUninit::<WebPPicture>::uninit();
        assert_ne!(unsafe { WebPPictureInitARGB(picture.as_mut_ptr()) }, 0);
        let mut picture = unsafe { picture.assume_init() };
        picture.width = width;
        picture.height = height;
        assert_ne!(
            unsafe { WebPPictureImportRGBA(&mut picture, rgba.as_ptr(), width * 4) },
            0
        );
        assert_eq!(picture.use_argb, 1);

        let mut memory = MaybeUninit::<WebPMemoryWriter>::uninit();
        unsafe { WebPMemoryWriterInit(memory.as_mut_ptr()) };
        let mut memory = unsafe { memory.assume_init() };
        picture.writer = Some(WebPMemoryWrite);
        picture.custom_ptr = (&raw mut memory).cast::<c_void>();

        assert_ne!(unsafe { WebPEncode(&config, &mut picture) }, 0);
        assert_eq!(picture.use_argb, 0);

        unsafe {
            WebPMemoryWriterClear(&mut memory);
            WebPPictureFree(&mut picture);
        }
    }

    /// 可逆なら復号が寸法・行の並び・チャネルの並びまで戻す
    #[test]
    fn a_lossless_encode_comes_back_through_the_decoder() {
        let (width, height) = (5, 3);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 0xFF]);
            }
        }

        let mut config = MaybeUninit::<WebPConfig>::uninit();
        assert_ne!(unsafe { WebPConfigInit(config.as_mut_ptr()) }, 0);
        let mut config = unsafe { config.assume_init() };
        config.lossless = 1;

        let mut picture = MaybeUninit::<WebPPicture>::uninit();
        assert_ne!(unsafe { WebPPictureInitARGB(picture.as_mut_ptr()) }, 0);
        let mut picture = unsafe { picture.assume_init() };
        picture.width = width;
        picture.height = height;
        assert_ne!(
            unsafe { WebPPictureImportRGBA(&mut picture, rgba.as_ptr(), width * 4) },
            0
        );

        let mut memory = MaybeUninit::<WebPMemoryWriter>::uninit();
        unsafe { WebPMemoryWriterInit(memory.as_mut_ptr()) };
        let mut memory = unsafe { memory.assume_init() };
        picture.writer = Some(WebPMemoryWrite);
        picture.custom_ptr = (&raw mut memory).cast::<c_void>();

        assert_ne!(unsafe { WebPEncode(&config, &mut picture) }, 0);

        let (mut decoded_width, mut decoded_height) = (0, 0);
        let decoded = unsafe {
            WebPDecodeRGBA(
                memory.mem,
                memory.size,
                &mut decoded_width,
                &mut decoded_height,
            )
        };
        assert!(!decoded.is_null());
        assert_eq!((decoded_width, decoded_height), (width, height));
        assert_eq!(
            unsafe { std::slice::from_raw_parts(decoded, rgba.len()) },
            rgba
        );

        unsafe {
            WebPFree(decoded.cast::<c_void>());
            WebPMemoryWriterClear(&mut memory);
            WebPPictureFree(&mut picture);
        }
    }
}
