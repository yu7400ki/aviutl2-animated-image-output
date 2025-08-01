mod config;
mod dialog;
mod encoder;

use crate::encoder::{AnimEncoder, AnimFrame, WebPConfig};
use aviutl::output2::{OutputInfo, OutputPluginTable};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use std::ffi::c_void;
use widestring::{U16CStr, Utf16Str, utf16str};
use win32_dialog::MessageBox;
use windows::{Win32::Foundation::*, core::*};

fn create_webp_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = unsafe { U16CStr::from_ptr_str(info.savefile).to_string_lossy() };

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;

    let mut webp_config = WebPConfig::new().map_err(|_| "WebPConfig初期化エラー")?;

    webp_config.quality = config.quality;
    webp_config.method = config.method as i32;
    webp_config.lossless = if config.lossless { 1 } else { 0 };
    webp_config.alpha_compression = 1;
    webp_config.thread_level = 1;

    let mut encoder = AnimEncoder::new(info.w as u32, info.h as u32, &webp_config, output_file)
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

    encoder.set_loop_count(config.repeat);

    let duration_ms = (1000.0 * info.scale as f64 / info.rate as f64).max(1.0) as i32;
    let mut timestamp = 0;

    for frame in 0..info.n {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = match config.color_format {
            ColorFormat::Rgb24 => info.get_video_rgb(frame),
            ColorFormat::Rgba32 => info.get_video_rgba(frame),
        };

        if let Some(pixel_data) = image_data {
            let anim_frame = match config.color_format {
                ColorFormat::Rgb24 => {
                    AnimFrame::from_rgb(&pixel_data, info.w as u32, info.h as u32, timestamp)
                }
                ColorFormat::Rgba32 => {
                    AnimFrame::from_rgba(&pixel_data, info.w as u32, info.h as u32, timestamp)
                }
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

        let config = Config::load();

        let result = match create_webp_from_video(info, &config) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("WebP出力エラー: {}", e);
                MessageBox::error(None, &error_msg, "エラー");
                false
            }
        };

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
                    MessageBox::warning(Some(hwnd), &error_msg, "警告");
                }
                true
            }
            None => false,
        }
    } else {
        MessageBox::error(Some(hwnd), "設定の取得に失敗しました。", "エラー");
        false
    }
}

const PLUGIN_NAME: &Utf16Str = utf16str!("WebP出力プラグイン\0");
const FILE_FILTER: &Utf16Str = utf16str!("WebP Files (*.webp)\0*.webp\0All Files (*)\0*\0\0");
const PLUGIN_INFO: &Utf16Str = utf16str!(concat!(
    "WebP出力プラグイン v",
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

const OUTPUT_PLUGIN_TABLE: OutputPluginTable = init_plugin_table();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn DllMain(_hinst: HINSTANCE, _reason: u32, _reserved: *mut c_void) -> BOOL {
    TRUE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetOutputPluginTable() -> *mut OutputPluginTable {
    &OUTPUT_PLUGIN_TABLE as *const OutputPluginTable as *mut OutputPluginTable
}
