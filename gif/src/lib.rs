mod config;
mod dialog;

use aviutl::{
    output2::{OutputInfo, OutputPluginTable, video_format},
    patch::{apply_rgba_patch, restore_rgba_patch},
    utils::{to_wide_string, wide_to_string},
};
use gif::{Encoder, Frame, Repeat};
use std::ffi::c_void;
use std::fs::File;
use std::sync::{LazyLock, Mutex};
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

        let frame_data = info.get_video(frame, video_format::BI_RGB);
        if let Some(data_ptr) = frame_data {
            let gif_frame = unsafe {
                if use_rgba {
                    // RGBAモード: RGBA32データを処理
                    let input_stride = info.w * 4; // RGBA32のストライド
                    let data_slice = std::slice::from_raw_parts(
                        data_ptr as *const u8,
                        (input_stride * info.h) as usize,
                    );

                    let mut rgba_buffer = Vec::with_capacity((info.w * info.h * 4) as usize);
                    // BMPは下から上に格納されているので反転
                    for y in (0..info.h).rev() {
                        for x in 0..info.w {
                            let offset = (y * input_stride + x * 4) as usize;
                            // BGRA -> RGBA変換
                            rgba_buffer.push(data_slice[offset + 2]); // R
                            rgba_buffer.push(data_slice[offset + 1]); // G
                            rgba_buffer.push(data_slice[offset]); // B
                            rgba_buffer.push(data_slice[offset + 3]); // A
                        }
                    }

                    let mut frame =
                        Frame::from_rgba(info.w as u16, info.h as u16, &mut rgba_buffer);
                    frame.dispose = gif::DisposalMethod::Background;
                    let delay = (100.0 * info.scale as f64 / info.rate as f64).round() as u16;
                    frame.delay = delay.max(1);
                    frame
                } else {
                    // RGBモード: RGB24データを直接使用
                    let input_stride = ((info.w * 3 + 3) / 4) * 4; // RGB24のストライド
                    let data_slice = std::slice::from_raw_parts(
                        data_ptr as *const u8,
                        (input_stride * info.h) as usize,
                    );

                    let mut rgb_buffer = Vec::with_capacity((info.w * info.h * 3) as usize);
                    // BMPは下から上に格納されているので反転
                    for y in (0..info.h).rev() {
                        for x in 0..info.w {
                            let offset = (y * input_stride + x * 3) as usize;
                            // BGR -> RGB変換
                            rgb_buffer.push(data_slice[offset + 2]); // R
                            rgb_buffer.push(data_slice[offset + 1]); // G
                            rgb_buffer.push(data_slice[offset]); // B
                        }
                    }

                    let mut frame = Frame::from_rgb(info.w as u16, info.h as u16, &rgb_buffer);
                    frame.dispose = gif::DisposalMethod::Background;
                    let delay = (100.0 * info.scale as f64 / info.rate as f64).round() as u16;
                    frame.delay = delay.max(1);
                    frame
                }
            };

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

const VERSION: &str = env!("CARGO_PKG_VERSION");
static PLUGIN_NAME: LazyLock<Vec<u16>> = LazyLock::new(|| to_wide_string("GIF出力プラグイン"));
static FILE_FILTER: LazyLock<Vec<u16>> =
    LazyLock::new(|| to_wide_string("GIF Files (*.gif)\0*.gif\0All Files (*)\0*\0"));
static PLUGIN_INFO: LazyLock<Vec<u16>> =
    LazyLock::new(|| to_wide_string(&format!("GIF出力プラグイン v{} by yu7400ki", VERSION)));

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
