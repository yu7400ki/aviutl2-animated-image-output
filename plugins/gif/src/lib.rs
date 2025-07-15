mod config;
mod dialog;

use aviutl::output2::{OutputInfo, OutputPluginTable};
#[cfg(feature = "rgba")]
use aviutl::patch::{apply_rgba_patch, restore_rgba_patch};
use gif::{Encoder, Frame, Repeat};
use std::ffi::c_void;
use std::fs::File;
use widestring::{U16CStr, Utf16Str, utf16str};
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*, core::*};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

fn create_gif_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = unsafe { U16CStr::from_ptr_str(info.savefile).to_string_lossy() };

    let output_file =
        File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;
    let mut encoder = Encoder::new(output_file, info.w as u16, info.h as u16, &[])
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;
    // 設定を取得
    let repeat_setting = if config.repeat == 0 {
        Repeat::Infinite
    } else {
        Repeat::Finite(config.repeat)
    };

    encoder
        .set_repeat(repeat_setting)
        .map_err(|e| format!("ループ設定エラー: {}", e))?;

    for frame in 0..info.n {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = match config.color_format {
            ColorFormat::Palette => info.get_video_rgb(frame),
            #[cfg(feature = "rgba")]
            ColorFormat::Transparent => info.get_video_rgba(frame),
            #[cfg(not(feature = "rgba"))]
            ColorFormat::Transparent => info.get_video_rgb(frame),
        };

        #[allow(unused_mut)]
        if let Some(mut image_data) = image_data {
            let mut gif_frame = match config.color_format {
                ColorFormat::Palette => {
                    Frame::from_rgb_speed(info.w as u16, info.h as u16, &image_data, config.speed)
                }
                #[cfg(feature = "rgba")]
                ColorFormat::Transparent => Frame::from_rgba_speed(
                    info.w as u16,
                    info.h as u16,
                    &mut image_data,
                    config.speed,
                ),
                #[cfg(not(feature = "rgba"))]
                ColorFormat::Transparent => {
                    Frame::from_rgb_speed(info.w as u16, info.h as u16, &image_data, config.speed)
                }
            };

            gif_frame.dispose = gif::DisposalMethod::Background;
            let delay = (100.0 * info.scale as f64 / info.rate as f64).round() as u16;
            gif_frame.delay = delay.max(1);

            encoder
                .write_frame(&gif_frame)
                .map_err(|e| format!("フレーム書き込みエラー: {}", e))?;
        }

        info.rest_time_disp(frame, info.n);
    }
    Ok(())
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

extern "C" fn output_func(oip: *mut OutputInfo) -> bool {
    unsafe {
        let info = match oip.as_ref() {
            Some(info) => info,
            None => return false,
        };

        let config = Config::load();

        // 透明度ありモードの場合のみパッチを適用
        #[cfg(feature = "rgba")]
        let use_rgba = matches!(config.color_format, ColorFormat::Transparent);

        #[cfg(feature = "rgba")]
        let old_protect: Option<u32> = if use_rgba {
            match apply_rgba_patch(&info) {
                Ok(protect) => Some(protect),
                Err(e) => {
                    let error_msg = format!("メモリパッチ適用エラー: {}", e);
                    let error_wide =
                        widestring::U16CString::from_str(&error_msg).unwrap_or_default();
                    let title_wide = widestring::U16CString::from_str("エラー").unwrap_or_default();

                    MessageBoxW(
                        Some(HWND::default()),
                        PCWSTR(error_wide.as_ptr()),
                        PCWSTR(title_wide.as_ptr()),
                        MB_OK | MB_ICONERROR,
                    );
                    return false;
                }
            }
        } else {
            None
        };

        let result = match create_gif_from_video(info, &config) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("GIF出力エラー: {}", e);
                let error_wide = widestring::U16CString::from_str(&error_msg).unwrap_or_default();
                let title_wide = widestring::U16CString::from_str("エラー").unwrap_or_default();

                MessageBoxW(
                    Some(HWND::default()),
                    PCWSTR(error_wide.as_ptr()),
                    PCWSTR(title_wide.as_ptr()),
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

static PLUGIN_NAME: &Utf16Str = utf16str!("GIF出力プラグイン\0");
static FILE_FILTER: &Utf16Str = utf16str!("GIF Files (*.gif)\0*.gif\0All Files (*)\0*\0\0");
static PLUGIN_INFO: &Utf16Str = utf16str!(concat!(
    "GIF出力プラグイン v",
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
