mod config;
mod dialog;

use aviutl::output2::{OutputInfo, OutputPluginTable};
#[cfg(feature = "rgba")]
use aviutl::patch::{apply_rgba_patch, restore_rgba_patch};
use libavif::{Encoder, RgbPixels, YuvFormat};
use std::ffi::c_void;
use widestring::{U16CStr, Utf16Str, utf16str};
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*, core::*};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

fn create_avif_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = unsafe { U16CStr::from_ptr_str(info.savefile).to_string_lossy() };

    let mut encoder = Encoder::new();
    encoder.set_timescale(info.rate as u64);
    encoder.set_quality(config.quality);
    encoder.set_speed(config.speed);
    encoder.set_max_threads(config.threads);

    let width = info.w as u32;
    let height = info.h as u32;
    let num_frames = info.n as u32;

    for frame in 0..num_frames {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = match config.color_format {
            ColorFormat::Rgb24 => info.get_video_rgb(frame as i32),
            #[cfg(feature = "rgba")]
            ColorFormat::Rgba32 => info.get_video_rgba(frame as i32),
            #[cfg(not(feature = "rgba"))]
            ColorFormat::Rgba32 => info.get_video_rgb(frame as i32),
        };

        if let Some(pixel_data) = image_data {
            let rgb_pixels = RgbPixels::new(width, height, &pixel_data)
                .map_err(|e| format!("RGBピクセル作成エラー: {}", e))?;

            let image = rgb_pixels.to_image(YuvFormat::Yuv420);

            encoder
                .add_image(&image, info.scale as u64, Default::default())
                .map_err(|e| format!("フレーム追加エラー: {}", e))?;
        }

        info.rest_time_disp(frame as i32, num_frames as i32);
    }

    let data = encoder
        .finish()
        .map_err(|e| format!("エンコード完了エラー: {}", e))?;

    std::fs::write(&output_path, &*data).map_err(|e| format!("ファイル保存エラー: {}", e))?;

    Ok(())
}

extern "C" fn output_func(oip: *mut OutputInfo) -> bool {
    unsafe {
        let info = match oip.as_ref() {
            Some(info) => info,
            None => return false,
        };

        let config = Config::load();

        // RGBAモードの場合のみパッチを適用
        #[cfg(feature = "rgba")]
        let use_rgba = matches!(config.color_format, ColorFormat::Rgba32);

        #[cfg(feature = "rgba")]
        let old_protect: Option<u32> = if use_rgba {
            match apply_rgba_patch(&info) {
                Ok(protect) => Some(protect),
                Err(e) => {
                    let error_msg = format!("メモリパッチ適用エラー: {}", e);
                    let error_wide =
                        widestring::U16CString::from_str(&error_msg).unwrap_or_default();

                    MessageBoxW(
                        Some(HWND::default()),
                        PCWSTR(error_wide.as_ptr()),
                        w!("エラー"),
                        MB_OK | MB_ICONERROR,
                    );
                    return false;
                }
            }
        } else {
            None
        };

        let result = match create_avif_from_video(info, &config) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("AVIF出力エラー: {}", e);
                let error_wide = widestring::U16CString::from_str(&error_msg).unwrap_or_default();

                MessageBoxW(
                    Some(HWND::default()),
                    PCWSTR(error_wide.as_ptr()),
                    w!("エラー"),
                    MB_OK | MB_ICONERROR,
                );
                false
            }
        };

        // パッチを復元
        #[cfg(feature = "rgba")]
        if let Some(protect) = old_protect {
            restore_rgba_patch(&info, protect);
        }

        result
    }
}

extern "C" fn config_func(hwnd: HWND, _dll_hinst: HINSTANCE) -> bool {
    let default_config = Config::load();

    if let Ok(result) = show_config_dialog(hwnd, default_config) {
        match result {
            Some(config) => {
                // 設定を保存
                if let Err(e) = config.save() {
                    let error_msg = format!("設定保存エラー: {}", e);
                    let error_wide =
                        widestring::U16CString::from_str(&error_msg).unwrap_or_default();

                    unsafe {
                        MessageBoxW(
                            Some(hwnd),
                            PCWSTR(error_wide.as_ptr()),
                            w!("警告"),
                            MB_OK | MB_ICONWARNING,
                        );
                    }
                }
                true
            }
            None => false,
        }
    } else {
        unsafe {
            MessageBoxW(
                Some(hwnd),
                w!("設定の取得に失敗しました。"),
                w!("エラー"),
                MB_OK | MB_ICONERROR,
            );
        }
        false
    }
}

static PLUGIN_NAME: &Utf16Str = utf16str!("AVIF出力プラグイン\0");
static FILE_FILTER: &Utf16Str = utf16str!("AVIF Files (*.avif)\0*.avif\0All Files (*)\0*\0\0");
static PLUGIN_INFO: &Utf16Str = utf16str!(concat!(
    "AVIF出力プラグイン v",
    env!("CARGO_PKG_VERSION"),
    " by yu7400ki\0"
));

const fn init_plugin_table() -> OutputPluginTable {
    OutputPluginTable {
        flag: OutputPluginTable::FLAG_VIDEO,
        name: PLUGIN_NAME.as_ptr(),
        filefilter: FILE_FILTER.as_ptr(),
        information: PLUGIN_INFO.as_ptr(),
        func_output: Some(output_func),
        func_config: Some(config_func),
        func_get_config_text: None,
    }
}

static OUTPUT_PLUGIN_TABLE: OutputPluginTable = init_plugin_table();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DllMain(_hinst: HINSTANCE, _reason: u32, _reserved: *mut c_void) -> BOOL {
    TRUE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetOutputPluginTable() -> *mut OutputPluginTable {
    &OUTPUT_PLUGIN_TABLE as *const OutputPluginTable as *mut OutputPluginTable
}
