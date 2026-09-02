//! libjxl のエンコード経路への FFI
//!
//! 宣言は同梱した libjxl v0.12.0 の `lib/include/jxl/` に対応する。安全な抽象は置かず、
//! 符号化に要る関数と、その受け渡しに現れる構造体だけを写す。
//!
//! 構造体はすべて呼び側が確保して値で渡すため、項目を名前で持ち、大きさも一致する。

#![allow(non_snake_case)]

use std::ffi::{c_int, c_void};

/// `JXL_BOOL` の真
pub const JXL_TRUE: c_int = 1;
/// `JXL_BOOL` の偽
pub const JXL_FALSE: c_int = 0;

/// `JxlDataType` — [`JxlPixelFormat::data_type`]
pub const JXL_TYPE_FLOAT: c_int = 0;
pub const JXL_TYPE_UINT8: c_int = 2;
pub const JXL_TYPE_UINT16: c_int = 3;
pub const JXL_TYPE_FLOAT16: c_int = 5;

/// `JxlEndianness` — [`JxlPixelFormat::endianness`]
pub const JXL_NATIVE_ENDIAN: c_int = 0;
pub const JXL_LITTLE_ENDIAN: c_int = 1;
pub const JXL_BIG_ENDIAN: c_int = 2;

/// `JxlEncoderStatus`
pub const JXL_ENC_SUCCESS: c_int = 0;
pub const JXL_ENC_ERROR: c_int = 1;
pub const JXL_ENC_NEED_MORE_OUTPUT: c_int = 2;

/// `JxlEncoderError` — [`JxlEncoderGetError`] が返す内訳
pub const JXL_ENC_ERR_OK: c_int = 0;
pub const JXL_ENC_ERR_GENERIC: c_int = 1;
pub const JXL_ENC_ERR_OOM: c_int = 2;
pub const JXL_ENC_ERR_JBRD: c_int = 3;
pub const JXL_ENC_ERR_BAD_INPUT: c_int = 4;
pub const JXL_ENC_ERR_NOT_SUPPORTED: c_int = 0x80;
pub const JXL_ENC_ERR_API_USAGE: c_int = 0x81;

/// `JxlEncoderFrameSettingId` — 速度と圧縮率の均衡 1..=10
pub const JXL_ENC_FRAME_SETTING_EFFORT: c_int = 0;

/// [`JxlParallelRunner`] の戻り値。0 が成功
pub type JxlParallelRetCode = c_int;

/// 並列実行の開始を告げる呼び戻し。使うスレッド数を渡す
pub type JxlParallelRunInit =
    unsafe extern "C" fn(jpegxl_opaque: *mut c_void, num_threads: usize) -> JxlParallelRetCode;

/// `[start_range, end_range)` の各値について一度ずつ呼ぶ呼び戻し
pub type JxlParallelRunFunction =
    unsafe extern "C" fn(jpegxl_opaque: *mut c_void, value: u32, thread_id: usize);

/// 並列実行の実体。`init` を自分のスレッドで一度呼び、`func` を範囲の各値について呼ぶ
pub type JxlParallelRunner = unsafe extern "C" fn(
    runner_opaque: *mut c_void,
    jpegxl_opaque: *mut c_void,
    init: JxlParallelRunInit,
    func: JxlParallelRunFunction,
    start_range: u32,
    end_range: u32,
) -> JxlParallelRetCode;

/// 確保を差し替える器。NULL で libjxl の既定になる
#[repr(C)]
pub struct JxlMemoryManager {
    _private: [u8; 0],
}

/// 符号化器。確保と解放は [`JxlEncoderCreate`] と [`JxlEncoderDestroy`]
#[repr(C)]
pub struct JxlEncoder {
    _private: [u8; 0],
}

/// フレーム単位の設定。所有は [`JxlEncoder`] にあり、個別の解放は無い
#[repr(C)]
pub struct JxlEncoderFrameSettings {
    _private: [u8; 0],
}

/// 取り込む画素の並び
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlPixelFormat {
    /// 1画素あたりのチャネル数。RGB で 3、RGBA で 4
    pub num_channels: u32,
    pub data_type: c_int,
    pub endianness: c_int,
    /// 行の先頭を揃えるバイト数。0 は詰めなし
    pub align: usize,
}

/// プレビュー画像の寸法
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlPreviewHeader {
    pub xsize: u32,
    pub ysize: u32,
}

/// 全フレームに効くアニメーションの設定
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlAnimationHeader {
    /// 1秒あたりの tick 数の分子
    pub tps_numerator: u32,
    /// 1秒あたりの tick 数の分母
    pub tps_denominator: u32,
    /// 再生回数。0 で無限
    pub num_loops: u32,
    pub have_timecodes: c_int,
}

/// 画像全体のメタデータ。初期化は [`JxlEncoderInitBasicInfo`]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlBasicInfo {
    pub have_container: c_int,
    pub xsize: u32,
    pub ysize: u32,
    pub bits_per_sample: u32,
    pub exponent_bits_per_sample: u32,
    pub intensity_target: f32,
    pub min_nits: f32,
    pub relative_to_max_display: c_int,
    pub linear_below: f32,
    /// 原色空間のまま符号化するか。可逆には真が要る
    pub uses_original_profile: c_int,
    pub have_preview: c_int,
    pub have_animation: c_int,
    pub orientation: c_int,
    /// 色チャネル数。グレースケールで 1、それ以外は 3
    pub num_color_channels: u32,
    /// α を含む追加チャネル数
    pub num_extra_channels: u32,
    /// α のビット深度。0 で α 無し
    pub alpha_bits: u32,
    pub alpha_exponent_bits: u32,
    pub alpha_premultiplied: c_int,
    pub preview: JxlPreviewHeader,
    pub animation: JxlAnimationHeader,
    pub intrinsic_xsize: u32,
    pub intrinsic_ysize: u32,
    pub padding: [u8; 100],
}

/// 重ね方。[`JxlEncoderInitFrameHeader`] が [`JxlLayerInfo`] ごと既定値で埋める
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlBlendInfo {
    pub blendmode: c_int,
    /// 下地にする参照フレームの番号 0..=3
    pub source: u32,
    /// α として使う追加チャネルの番号
    pub alpha: u32,
    pub clamp: c_int,
}

/// フレームがキャンバスのどこを占めるか
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlLayerInfo {
    /// 真のとき `crop_x0` / `crop_y0` / `xsize` / `ysize` が効く
    pub have_crop: c_int,
    pub crop_x0: i32,
    pub crop_y0: i32,
    pub xsize: u32,
    pub ysize: u32,
    pub blend_info: JxlBlendInfo,
    /// 重ねた結果を参照フレームとして残す番号 0..=3
    pub save_as_reference: u32,
}

/// フレーム単位のメタデータ。初期化は [`JxlEncoderInitFrameHeader`]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlFrameHeader {
    /// 表示し終えてから待つ tick 数
    pub duration: u32,
    pub timecode: u32,
    pub name_length: u32,
    pub is_last: c_int,
    pub layer_info: JxlLayerInfo,
}

/// 色の解釈。sRGB で埋めるには [`JxlColorEncodingSetToSRGB`]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct JxlColorEncoding {
    pub color_space: c_int,
    pub white_point: c_int,
    pub white_point_xy: [f64; 2],
    pub primaries: c_int,
    pub primaries_red_xy: [f64; 2],
    pub primaries_green_xy: [f64; 2],
    pub primaries_blue_xy: [f64; 2],
    pub transfer_function: c_int,
    pub gamma: f64,
    pub rendering_intent: c_int,
}

unsafe extern "C" {
    /// `主 * 1000000 + 副 * 1000 + 改訂`
    pub fn JxlEncoderVersion() -> u32;

    /// 失敗すれば NULL
    pub fn JxlEncoderCreate(memory_manager: *const JxlMemoryManager) -> *mut JxlEncoder;

    pub fn JxlEncoderDestroy(enc: *mut JxlEncoder);

    /// フレームを追加する前に呼ぶ
    pub fn JxlEncoderSetParallelRunner(
        enc: *mut JxlEncoder,
        parallel_runner: JxlParallelRunner,
        parallel_runner_opaque: *mut c_void,
    ) -> c_int;

    /// 直前の [`JXL_ENC_ERROR`] の内訳
    pub fn JxlEncoderGetError(enc: *mut JxlEncoder) -> c_int;

    /// `info` を 8bit RGB・追加チャネル無しの既定値で埋める
    pub fn JxlEncoderInitBasicInfo(info: *mut JxlBasicInfo);

    /// 画像全体のメタデータを確定する。フレームの追加より前に呼ぶ
    pub fn JxlEncoderSetBasicInfo(enc: *mut JxlEncoder, info: *const JxlBasicInfo) -> c_int;

    /// `color_encoding` を sRGB で埋める
    pub fn JxlColorEncodingSetToSRGB(color_encoding: *mut JxlColorEncoding, is_gray: c_int);

    /// 原色空間を確定する。[`JxlEncoderSetBasicInfo`] より後に呼ぶ
    pub fn JxlEncoderSetColorEncoding(
        enc: *mut JxlEncoder,
        color: *const JxlColorEncoding,
    ) -> c_int;

    /// `source` から値を写した設定を作る。NULL の `source` で既定値になる。
    /// 解放は [`JxlEncoderDestroy`] がまとめて行う
    pub fn JxlEncoderFrameSettingsCreate(
        enc: *mut JxlEncoder,
        source: *const JxlEncoderFrameSettings,
    ) -> *mut JxlEncoderFrameSettings;

    pub fn JxlEncoderFrameSettingsSetOption(
        frame_settings: *mut JxlEncoderFrameSettings,
        option: c_int,
        value: i64,
    ) -> c_int;

    /// 目標とする butteraugli 距離 0.0..=25.0。小さいほど高品質
    pub fn JxlEncoderSetFrameDistance(
        frame_settings: *mut JxlEncoderFrameSettings,
        distance: f32,
    ) -> c_int;

    /// 可逆に必要な設定をまとめて上書きする
    pub fn JxlEncoderSetFrameLossless(
        frame_settings: *mut JxlEncoderFrameSettings,
        lossless: c_int,
    ) -> c_int;

    /// libjpeg-turbo の quality 0.0..=100.0 に揃う distance
    pub fn JxlEncoderDistanceFromQuality(quality: f32) -> f32;

    /// `frame_header` を表示時間 0・重ね方 replace の既定値で埋める
    pub fn JxlEncoderInitFrameHeader(frame_header: *mut JxlFrameHeader);

    /// 次に追加するフレームのメタデータを与える
    pub fn JxlEncoderSetFrameHeader(
        frame_settings: *mut JxlEncoderFrameSettings,
        frame_header: *const JxlFrameHeader,
    ) -> c_int;

    /// `buffer` をフレームとして積む。内容は内部へ複製される
    pub fn JxlEncoderAddImageFrame(
        frame_settings: *const JxlEncoderFrameSettings,
        pixel_format: *const JxlPixelFormat,
        buffer: *const c_void,
        size: usize,
    ) -> c_int;

    /// これ以上フレームを追加しないことを告げる。最後のフレームを排水する前に呼ぶ
    pub fn JxlEncoderCloseInput(enc: *mut JxlEncoder);

    /// 積んだフレームを符号化して `next_out` へ書き、書いた分だけ両者を進める。
    /// [`JXL_ENC_NEED_MORE_OUTPUT`] なら空きを足して呼び直す
    pub fn JxlEncoderProcessOutput(
        enc: *mut JxlEncoder,
        next_out: *mut *mut u8,
        avail_out: *mut usize,
    ) -> c_int;

    /// [`JxlEncoderSetParallelRunner`] へ渡す並列実行の実体
    pub fn JxlThreadParallelRunner(
        runner_opaque: *mut c_void,
        jpegxl_opaque: *mut c_void,
        init: JxlParallelRunInit,
        func: JxlParallelRunFunction,
        start_range: u32,
        end_range: u32,
    ) -> JxlParallelRetCode;

    /// [`JxlThreadParallelRunner`] へ渡す不透明な実行状態。失敗すれば NULL
    pub fn JxlThreadParallelRunnerCreate(
        memory_manager: *const JxlMemoryManager,
        num_worker_threads: usize,
    ) -> *mut c_void;

    pub fn JxlThreadParallelRunnerDestroy(runner_opaque: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{MaybeUninit, offset_of, size_of};

    unsafe extern "C" {
        fn jxl_sys_sizes(out: *mut usize);
        fn jxl_sys_pixel_format_offsets(out: *mut usize);
        fn jxl_sys_preview_header_offsets(out: *mut usize);
        fn jxl_sys_animation_header_offsets(out: *mut usize);
        fn jxl_sys_basic_info_offsets(out: *mut usize);
        fn jxl_sys_blend_info_offsets(out: *mut usize);
        fn jxl_sys_layer_info_offsets(out: *mut usize);
        fn jxl_sys_frame_header_offsets(out: *mut usize);
        fn jxl_sys_color_encoding_offsets(out: *mut usize);
    }

    /// 並べた順に項目の位置を取る
    macro_rules! offsets {
        ($ty:ty, $($field:ident),+ $(,)?) => {
            [$(offset_of!($ty, $field)),+]
        };
    }

    /// C 側の出口が同じ順で並べた位置
    fn from_header<const N: usize>(fill: unsafe extern "C" fn(*mut usize)) -> [usize; N] {
        let mut values = [0usize; N];
        unsafe { fill(values.as_mut_ptr()) };
        values
    }

    #[test]
    fn the_linked_library_reports_v0_12_0() {
        assert_eq!(unsafe { JxlEncoderVersion() }, 12000);
    }

    #[test]
    fn struct_sizes_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_sizes),
            [
                size_of::<JxlPixelFormat>(),
                size_of::<JxlPreviewHeader>(),
                size_of::<JxlAnimationHeader>(),
                size_of::<JxlBasicInfo>(),
                size_of::<JxlBlendInfo>(),
                size_of::<JxlLayerInfo>(),
                size_of::<JxlFrameHeader>(),
                size_of::<JxlColorEncoding>(),
            ]
        );
    }

    #[test]
    fn pixel_format_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_pixel_format_offsets),
            offsets!(JxlPixelFormat, num_channels, data_type, endianness, align)
        );
    }

    #[test]
    fn preview_header_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_preview_header_offsets),
            offsets!(JxlPreviewHeader, xsize, ysize)
        );
    }

    #[test]
    fn animation_header_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_animation_header_offsets),
            offsets!(
                JxlAnimationHeader,
                tps_numerator,
                tps_denominator,
                num_loops,
                have_timecodes,
            )
        );
    }

    #[test]
    fn basic_info_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_basic_info_offsets),
            offsets!(
                JxlBasicInfo,
                have_container,
                xsize,
                ysize,
                bits_per_sample,
                exponent_bits_per_sample,
                intensity_target,
                min_nits,
                relative_to_max_display,
                linear_below,
                uses_original_profile,
                have_preview,
                have_animation,
                orientation,
                num_color_channels,
                num_extra_channels,
                alpha_bits,
                alpha_exponent_bits,
                alpha_premultiplied,
                preview,
                animation,
                intrinsic_xsize,
                intrinsic_ysize,
                padding,
            )
        );
    }

    #[test]
    fn blend_info_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_blend_info_offsets),
            offsets!(JxlBlendInfo, blendmode, source, alpha, clamp)
        );
    }

    #[test]
    fn layer_info_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_layer_info_offsets),
            offsets!(
                JxlLayerInfo,
                have_crop,
                crop_x0,
                crop_y0,
                xsize,
                ysize,
                blend_info,
                save_as_reference,
            )
        );
    }

    #[test]
    fn frame_header_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_frame_header_offsets),
            offsets!(
                JxlFrameHeader,
                duration,
                timecode,
                name_length,
                is_last,
                layer_info,
            )
        );
    }

    #[test]
    fn color_encoding_field_offsets_match_the_vendored_header() {
        assert_eq!(
            from_header(jxl_sys_color_encoding_offsets),
            offsets!(
                JxlColorEncoding,
                color_space,
                white_point,
                white_point_xy,
                primaries,
                primaries_red_xy,
                primaries_green_xy,
                primaries_blue_xy,
                transfer_function,
                gamma,
                rendering_intent,
            )
        );
    }

    /// 手詰めを置き換える初期化。項目の追加にも既定値で追従する
    #[test]
    fn init_basic_info_fills_an_8bit_rgb_image() {
        let mut info = MaybeUninit::<JxlBasicInfo>::zeroed();
        unsafe { JxlEncoderInitBasicInfo(info.as_mut_ptr()) };
        let info = unsafe { info.assume_init() };

        assert_eq!(info.bits_per_sample, 8);
        assert_eq!(info.exponent_bits_per_sample, 0);
        assert_eq!(info.num_color_channels, 3);
        assert_eq!(info.num_extra_channels, 0);
        assert_eq!(info.alpha_bits, 0);
        assert_eq!(info.have_animation, JXL_FALSE);
        assert_eq!(info.uses_original_profile, JXL_FALSE);
        assert_eq!(info.animation.num_loops, 0);
    }

    #[test]
    fn init_frame_header_fills_a_still_frame() {
        let mut header = MaybeUninit::<JxlFrameHeader>::zeroed();
        unsafe { JxlEncoderInitFrameHeader(header.as_mut_ptr()) };
        let header = unsafe { header.assume_init() };

        assert_eq!(header.duration, 0);
        assert_eq!(header.timecode, 0);
        assert_eq!(header.layer_info.have_crop, JXL_FALSE);
        assert_eq!(header.layer_info.blend_info.source, 0);
    }

    #[test]
    fn srgb_color_encoding_comes_from_the_linked_library() {
        let mut color = MaybeUninit::<JxlColorEncoding>::zeroed();
        unsafe { JxlColorEncodingSetToSRGB(color.as_mut_ptr(), JXL_FALSE) };
        let color = unsafe { color.assume_init() };

        assert_eq!(color.white_point_xy, [0.3127, 0.3290]);
        assert_eq!(color.primaries_red_xy, [0.639998686, 0.330010138]);
        assert_eq!(color.primaries_green_xy, [0.300003784, 0.600003357]);
        assert_eq!(color.primaries_blue_xy, [0.150002046, 0.059997204]);
        assert_eq!(color.gamma, 0.0);
    }

    /// quality 100 は distance 0 に写り、libjxl が可逆へ倒す起点になる
    #[test]
    fn quality_maps_to_a_butteraugli_distance() {
        assert_eq!(unsafe { JxlEncoderDistanceFromQuality(100.0) }, 0.0);
        assert!(unsafe { JxlEncoderDistanceFromQuality(90.0) } > 0.0);
        assert!(
            unsafe { JxlEncoderDistanceFromQuality(50.0) }
                > unsafe { JxlEncoderDistanceFromQuality(90.0) }
        );
    }

    #[test]
    fn a_created_encoder_starts_without_an_error() {
        let enc = unsafe { JxlEncoderCreate(std::ptr::null()) };
        assert!(!enc.is_null());
        assert_eq!(unsafe { JxlEncoderGetError(enc) }, JXL_ENC_ERR_OK);
        unsafe { JxlEncoderDestroy(enc) };
    }
}
