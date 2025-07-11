use aviutl::{
    output2::{OutputInfo, OutputPluginTable, video_format},
    utils::{to_wide_string, wide_to_string},
};
use gif::{Encoder, Frame, Repeat};
use std::ffi::c_void;
use std::fs::File;
use std::sync::LazyLock;
use windows::{Win32::Foundation::*, Win32::UI::WindowsAndMessaging::*, core::*};

fn create_gif_from_video(info: &OutputInfo) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let output_path = wide_to_string(info.savefile);

    let output_file = File::create(&output_path)?;
    let mut encoder = Encoder::new(output_file, info.w as u16, info.h as u16, &[])?;
    encoder.set_repeat(Repeat::Infinite)?;

    for frame in 0..info.n {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let frame_data = info.get_video(frame, video_format::BI_RGB);
        if let Some(data_ptr) = frame_data {
            let rgb_data = unsafe {
                let stride = ((info.w * 3 + 3) / 4) * 4; // 4バイト境界にアライン
                let data_slice =
                    std::slice::from_raw_parts(data_ptr as *const u8, (stride * info.h) as usize);

                // BMPは下から上に格納されているので反転
                let mut rgb_buffer = Vec::with_capacity((info.w * info.h * 3) as usize);
                for y in (0..info.h).rev() {
                    for x in 0..info.w {
                        let offset = (y * stride + x * 3) as usize;
                        // BGR -> RGB変換
                        rgb_buffer.push(data_slice[offset + 2]); // R
                        rgb_buffer.push(data_slice[offset + 1]); // G
                        rgb_buffer.push(data_slice[offset]); // B
                    }
                }
                rgb_buffer
            };

            // フレームデータを書き込み
            let mut frame = Frame::from_rgb(info.w as u16, info.h as u16, &rgb_data);
            // フレーム遅延を設定 (1/100秒単位)
            frame.delay = 10; // 0.1秒 = 10fps
            encoder.write_frame(&frame)?;
        }

        info.rest_time_disp(frame, info.n);
    }

    Ok(())
}

extern "C" fn output_func(oip: *mut OutputInfo) -> bool {
    unsafe {
        let info = match oip.as_ref() {
            Some(info) => info,
            None => return false,
        };

        match create_gif_from_video(info) {
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
        }
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
        func_config: None,
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
