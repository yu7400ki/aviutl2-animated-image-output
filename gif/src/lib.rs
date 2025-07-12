mod config;
mod dialog;

use aviutl::{
    output2::{OutputInfo, OutputPluginTable},
    patch::{apply_rgba_patch, restore_rgba_patch},
    utils::{to_wide_string, wide_to_string},
};
use gif::{Encoder, Frame, Repeat};
use std::ffi::c_void;
use std::fs::File;
use std::sync::{LazyLock, Mutex};
use widestring::{Utf16Str, utf16str};
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*, core::*};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

static CONFIG: Mutex<Config> = Mutex::new(Config::default());

fn create_gif_from_video(info: &OutputInfo) -> std::result::Result<(), String> {
    let output_path = wide_to_string(info.savefile);

    let output_file =
        File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;
    let mut encoder = Encoder::new(output_file, info.w as u16, info.h as u16, &[])
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;
    // 設定を取得
    let (repeat_setting, use_rgba) = match CONFIG.lock() {
        Ok(guard) => {
            let repeat = if guard.repeat == 0 {
                Repeat::Infinite
            } else {
                Repeat::Finite(guard.repeat)
            };
            let rgba = matches!(guard.color_format, ColorFormat::Transparent);
            (repeat, rgba)
        }
        Err(_) => (Repeat::Infinite, true), // デフォルト値を使用
    };
    encoder
        .set_repeat(repeat_setting)
        .map_err(|e| format!("ループ設定エラー: {}", e))?;

    for frame in 0..info.n {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = if use_rgba {
            info.get_video_rgba(frame)
        } else {
            info.get_video_rgb(frame)
        };

        if let Some(mut image_data) = image_data {
            let mut gif_frame = if use_rgba {
                Frame::from_rgba(info.w as u16, info.h as u16, &mut image_data)
            } else {
                Frame::from_rgb(info.w as u16, info.h as u16, &image_data)
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
    let default_config = match CONFIG.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => Config::default(),
    };

    if let Ok(result) = show_config_dialog(hwnd, default_config)
        && let Ok(mut guard) = CONFIG.lock()
    {
        match result {
            Some(config) => {
                *guard = config;
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

        // 透明度ありモードの場合のみパッチを適用
        let use_rgba = match CONFIG.lock() {
            Ok(guard) => matches!(guard.color_format, ColorFormat::Transparent),
            Err(_) => false,
        };

        let old_protect = if use_rgba {
            match apply_rgba_patch(&info) {
                Ok(protect) => Some(protect),
                Err(e) => {
                    let error_msg = format!("メモリパッチ適用エラー: {}", e);
                    let error_wide = to_wide_string(&error_msg);
                    let title_wide = to_wide_string("エラー");

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

        let result = match create_gif_from_video(info) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("GIF出力エラー: {}", e);
                let error_wide = to_wide_string(&error_msg);
                let title_wide = to_wide_string("エラー");

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

static OUTPUT_PLUGIN_TABLE: LazyLock<OutputPluginTable> = LazyLock::new(|| init_plugin_table());

fn init_plugin_table() -> OutputPluginTable {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DllMain(_hinst: HINSTANCE, _reason: u32, _reserved: *mut c_void) -> BOOL {
    TRUE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetOutputPluginTable() -> *mut OutputPluginTable {
    &*OUTPUT_PLUGIN_TABLE as *const OutputPluginTable as *mut OutputPluginTable
}
