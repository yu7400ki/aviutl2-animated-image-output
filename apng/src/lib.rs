mod config;
mod dialog;

use aviutl::{
    output2::{OutputInfo, OutputPluginTable, video_format},
    patch::{apply_rgba_patch, restore_rgba_patch},
    utils::{to_wide_string, wide_to_string},
};
use png::{BitDepth, ColorType, Encoder};
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex};
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
        let frame_data = if channels == 4 {
            // RGBAモード: パッチを適用してRGBA32データを取得
            info.get_video(frame, video_format::BI_RGB)
        } else {
            // RGBモード: 通常のRGB24データを取得
            info.get_video(frame, video_format::BI_RGB)
        };
        if let Some(data_ptr) = frame_data {
            let image_data = unsafe {
                if channels == 4 {
                    // RGBAモード: RGBA32データを処理
                    let input_stride = info.w * 4; // RGBA32のストライド
                    let data_slice = std::slice::from_raw_parts(
                        data_ptr as *const u8,
                        (input_stride * info.h) as usize,
                    );

                    let mut image_buffer = Vec::with_capacity((info.w * info.h * 4) as usize);
                    // BMPは下から上に格納されているので反転
                    for y in (0..info.h).rev() {
                        for x in 0..info.w {
                            let offset = (y * input_stride + x * 4) as usize;
                            // BGRA -> RGBA変換
                            image_buffer.push(data_slice[offset + 2]); // R
                            image_buffer.push(data_slice[offset + 1]); // G
                            image_buffer.push(data_slice[offset]); // B
                            image_buffer.push(data_slice[offset + 3]); // A
                        }
                    }
                    image_buffer
                } else {
                    // RGBモード: RGB24データを処理
                    let input_stride = ((info.w * 3 + 3) / 4) * 4; // RGB24のストライド
                    let data_slice = std::slice::from_raw_parts(
                        data_ptr as *const u8,
                        (input_stride * info.h) as usize,
                    );

                    let mut image_buffer = Vec::with_capacity((info.w * info.h * 3) as usize);
                    // BMPは下から上に格納されているので反転
                    for y in (0..info.h).rev() {
                        for x in 0..info.w {
                            let offset = (y * input_stride + x * 3) as usize;
                            // BGR -> RGB変換
                            image_buffer.push(data_slice[offset + 2]); // R
                            image_buffer.push(data_slice[offset + 1]); // G
                            image_buffer.push(data_slice[offset]); // B
                        }
                    }
                    image_buffer
                }
            };

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

const VERSION: &str = env!("CARGO_PKG_VERSION");
static PLUGIN_NAME: LazyLock<Vec<u16>> = LazyLock::new(|| to_wide_string("APNG出力プラグイン"));
static FILE_FILTER: LazyLock<Vec<u16>> =
    LazyLock::new(|| to_wide_string("PNG Files (*.png)\0*.png\0All Files (*)\0*\0"));
static PLUGIN_INFO: LazyLock<Vec<u16>> =
    LazyLock::new(|| to_wide_string(&format!("APNG出力プラグイン v{} by yu7400ki", VERSION)));

static mut OUTPUT_PLUGIN_TABLE: Option<OutputPluginTable> = None;

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
    unsafe {
        let table_ptr = std::ptr::addr_of_mut!(OUTPUT_PLUGIN_TABLE);
        if (*table_ptr).is_none() {
            *table_ptr = Some(init_plugin_table());
        }
        (*table_ptr).as_mut().unwrap() as *mut OutputPluginTable
    }
}
