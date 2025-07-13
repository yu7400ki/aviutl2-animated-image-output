mod config;
mod dialog;
mod encoder;

use crate::encoder::{AnimEncoder, AnimFrame, WebPConfig};
use aviutl::{
    output2::{OutputInfo, OutputPluginTable},
    patch::{apply_rgba_patch, restore_rgba_patch},
};
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex};
use widestring::{U16CStr, Utf16Str, utf16str};
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*, core::*};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

static CONFIG: Mutex<Config> = Mutex::new(Config::default());

fn create_webp_from_video(info: &OutputInfo) -> std::result::Result<(), String> {
    let output_path = unsafe { U16CStr::from_ptr_str(info.savefile).to_string_lossy() };

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;

    let mut config = WebPConfig::new().map_err(|_| "WebPConfig初期化エラー")?;
    config.lossless = 1; // ロスレス圧縮を有効化
    config.alpha_compression = 0; // アルファ圧縮を無効化
    config.quality = 75.0; // 画質を75に設定

    let mut encoder = AnimEncoder::new(info.w as u32, info.h as u32, &config, output_file)
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

    let (repeat_count, use_rgba) = match CONFIG.lock() {
        Ok(guard) => (guard.repeat, guard.color_format == ColorFormat::Rgba32),
        Err(_) => (0, false), // デフォルト値を使用
    };

    encoder.set_loop_count(repeat_count);

    let duration_ms = (1000.0 * info.scale as f64 / info.rate as f64).max(1.0) as i32;
    let mut timestamp = 0;

    for frame in 0..info.n {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        // カラーフォーマットに応じてフレームデータを取得
        let image_data = if use_rgba {
            // RGBAモード: パッチを適用してRGBA32データを取得
            info.get_video_rgba(frame)
        } else {
            // RGBモード: 通常のRGB24データを取得
            info.get_video_rgb(frame)
        };

        // データが取得できた場合のみ処理
        if let Some(data) = image_data {
            let anim_frame = if use_rgba {
                AnimFrame::from_rgba(&data, info.w as u32, info.h as u32, timestamp)
            } else {
                AnimFrame::from_rgb(&data, info.w as u32, info.h as u32, timestamp)
            };

            encoder
                .add_frame(anim_frame)
                .map_err(|e| format!("フレーム追加エラー: {}", e))?;
        }

        timestamp += duration_ms;
        info.rest_time_disp(frame, info.n);
    }

    encoder
        .finalize()
        .map_err(|e| format!("エンコード完了エラー: {}", e))?;

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

        let result = match create_webp_from_video(info) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("WebP出力エラー: {}", e);
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

static PLUGIN_NAME: &Utf16Str = utf16str!("WebP出力プラグイン\0");
static FILE_FILTER: &Utf16Str = utf16str!("WebP Files (*.webp)\0*.webp\0All Files (*)\0*\0");
static PLUGIN_INFO: &Utf16Str = utf16str!(concat!(
    "WebP出力プラグイン v",
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
