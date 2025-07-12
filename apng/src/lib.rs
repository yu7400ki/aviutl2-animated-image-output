mod config;
mod dialog;

use aviutl::{
    output2::{OutputInfo, OutputPluginTable},
    patch::{apply_rgba_patch, restore_rgba_patch},
    utils::{to_wide_string, wide_to_string},
};
use png::{BitDepth, ColorType, Encoder};
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex};
use widestring::{Utf16Str, utf16str};
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*, core::*};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

static CONFIG: Mutex<Config> = Mutex::new(Config::default());

fn create_apng_from_video(info: &OutputInfo) -> std::result::Result<(), String> {
    let output_path = wide_to_string(info.savefile);

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;
    let mut encoder = Encoder::new(output_file, info.w as u32, info.h as u32);
    // カラーフォーマット設定
    let (color_type, channels) = match CONFIG.lock() {
        Ok(guard) => match guard.color_format {
            ColorFormat::Rgb24 => (ColorType::Rgb, 3),
            ColorFormat::Rgba32 => (ColorType::Rgba, 4),
        },
        Err(_) => (ColorType::Rgb, 3), // デフォルト値を使用
    };
    encoder.set_color(color_type);
    encoder.set_depth(BitDepth::Eight);

    // APNG設定
    let repeat_count = match CONFIG.lock() {
        Ok(guard) => guard.repeat,
        Err(_) => 0, // デフォルト値を使用
    };
    encoder
        .set_animated(info.n as u32, repeat_count)
        .map_err(|e| format!("APNG設定エラー: {}", e))?;

    encoder
        .set_frame_delay(info.scale as u16, info.rate as u16)
        .map_err(|e| format!("フレームレート設定エラー: {}", e))?;

    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

    for frame in 0..info.n {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }
        // カラーフォーマットに応じてフレームデータを取得
        let image_data = if channels == 4 {
            // RGBAモード: パッチを適用してRGBA32データを取得
            info.get_video_rgba(frame)
        } else {
            // RGBモード: 通常のRGB24データを取得
            info.get_video_rgb(frame)
        };

        if let Some(image_data) = image_data {
            // フレームデータを書き込み
            writer
                .write_image_data(&image_data)
                .map_err(|e| format!("フレーム書き込みエラー: {}", e))?;
        }

        info.rest_time_disp(frame, info.n);
    }

    writer
        .finish()
        .map_err(|e| format!("エンコーダー終了エラー: {}", e))?;
    Ok(())
}

extern "C" fn output_func(oip: *mut OutputInfo) -> bool {
    unsafe {
        let info = match oip.as_ref() {
            Some(info) => info,
            None => return false,
        };

        // RGBAモードの場合のみパッチを適用
        let use_rgba = match CONFIG.lock() {
            Ok(guard) => matches!(guard.color_format, ColorFormat::Rgba32),
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

        let result = match create_apng_from_video(info) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("APNG出力エラー: {}", e);
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

static PLUGIN_NAME: &Utf16Str = utf16str!("APNG出力プラグイン\0");
static FILE_FILTER: &Utf16Str = utf16str!("PNG Files (*.png)\0*.png\0All Files (*)\0*\0");
static PLUGIN_INFO: &Utf16Str = utf16str!(concat!(
    "APNG出力プラグイン v",
    env!("CARGO_PKG_VERSION"),
    " by yu7400ki\0"
));

static OUTPUT_PLUGIN_TABLE: LazyLock<OutputPluginTable> = LazyLock::new(init_plugin_table);

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
