mod config;
mod dialog;

use aviutl::output2::{OutputInfo, OutputPluginTable};
#[cfg(feature = "rgba")]
use aviutl::patch::{apply_rgba_patch, restore_rgba_patch};
use chroma_key::apply_chroma_key;
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

    let color_type = if config.color_format == ColorFormat::Rgba32 || config.chroma_key_enabled {
        ColorType::Rgba
    } else {
        ColorType::Rgb
    };

    encoder.set_color(color_type);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(config.compression_type.into());
    encoder.set_filter(config.filter_type.into());

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
            // RGBAモード: パッチを適用してRGBA32データを取得
            #[cfg(feature = "rgba")]
            {
                info.get_video_rgba(frame)
            }
            #[cfg(not(feature = "rgba"))]
            {
                if config.chroma_key_enabled {
                    info.get_video_rgb_4ch(frame)
                } else {
                    info.get_video_rgb(frame)
                }
            }
        } else {
            if config.chroma_key_enabled {
                info.get_video_rgb_4ch(frame)
            } else {
                info.get_video_rgb(frame)
            }
        };

        if let Some(mut data) = frame_data {
            // クロマキー処理を適用
            if config.chroma_key_enabled {
                apply_chroma_key(
                    &mut data,
                    config.chroma_key_color.to_array(),
                    config.chroma_key_hue_range as f32, // 0-360度
                    config.chroma_key_saturation_range as f32 / 100.0, // 0-100 → 0.0-1.0
                );
            }

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

        // RGBAモードの場合のみパッチを適用
        #[cfg(feature = "rgba")]
        let use_rgba = matches!(config.color_format, ColorFormat::Rgba32);

        #[cfg(feature = "rgba")]
        let old_protect: Option<u32> = if use_rgba {
            match apply_rgba_patch(&info) {
                Ok(protect) => Some(protect),
                Err(e) => {
                    let error_msg = format!("メモリパッチ適用エラー: {}", e);
                    MessageBox::error(Some(HWND::default()), &error_msg, "エラー");
                    return false;
                }
            }
        } else {
            None
        };

        let result = match create_apng_from_video(info, &config) {
            Ok(_) => true,
            Err(e) => {
                let error_msg = format!("APNG出力エラー: {}", e);
                MessageBox::error(Some(HWND::default()), &error_msg, "エラー");
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
