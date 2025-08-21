mod config;
mod dialog;

use aviutl::output2::{OutputInfo, OutputPluginTable};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use png::{BitDepth, ColorType, Encoder};
use std::ffi::c_void;
use widestring::{U16CStr, Utf16Str, utf16str};
use win32_dialog::MessageBox;
use windows::{Win32::Foundation::*, core::*};

fn create_apng_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = unsafe { U16CStr::from_ptr_str(info.savefile).to_string_lossy() };

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;
    let mut encoder = Encoder::new(output_file, info.w as u32, info.h as u32);

    let color_type = if config.color_format == ColorFormat::Rgba32 {
        ColorType::Rgba
    } else {
        ColorType::Rgb
    };

    encoder.set_color(color_type);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(config.compression_type.into());

    if config.adaptive_filter {
        encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
    } else {
        encoder.set_filter(config.filter_type.into());
        encoder.set_adaptive_filter(png::AdaptiveFilterType::NonAdaptive);
    }

    // APNG設定
    encoder
        .set_animated(info.n as u32, config.repeat)
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
        let frame_data = if config.color_format == ColorFormat::Rgba32 {
            info.get_video_rgba(frame)
        } else {
            info.get_video_rgb(frame)
        };

        if let Some(data) = frame_data {
            // フレームデータを書き込み
            writer
                .write_image_data(&data)
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

        // 設定を読み込み
        let config = Config::load();

        let result = match create_apng_from_video(info, &config) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("APNG出力エラー: {}", e);
                MessageBox::error(Some(HWND::default()), &error_msg, "エラー");
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

const PLUGIN_NAME: &Utf16Str = utf16str!("APNG出力プラグイン\0");
const FILE_FILTER: &Utf16Str = utf16str!("PNG Files (*.png)\0*.png\0All Files (*)\0*\0\0");
const PLUGIN_INFO: &Utf16Str = utf16str!(concat!(
    "APNG出力プラグイン v",
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
