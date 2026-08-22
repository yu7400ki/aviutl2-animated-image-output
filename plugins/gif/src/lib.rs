mod config;
mod dialog;

use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin,
};
use gif::{Encoder, Frame, Repeat};
use std::fs::File;
use win32_dialog::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

fn create_gif_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = info.savefile();

    let output_file =
        File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;
    let mut encoder = Encoder::new(output_file, info.width() as u16, info.height() as u16, &[])
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;
    // 設定を取得
    let repeat_setting = if config.repeat == 0 {
        Repeat::Infinite
    } else {
        Repeat::Finite(config.repeat - 1)
    };

    encoder
        .set_repeat(repeat_setting)
        .map_err(|e| format!("ループ設定エラー: {}", e))?;

    let delay = (100.0 * info.scale() as f64 / info.rate() as f64).round() as u16;
    let delay = delay.max(1);

    for frame in 0..info.num_frames() {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = info.get_video_frame(frame, config.color_format);

        if let Some(mut image_data) = image_data {
            let mut gif_frame = match config.color_format {
                ColorFormat::Rgb24 => Frame::from_rgb_speed(
                    info.width() as u16,
                    info.height() as u16,
                    &image_data,
                    config.speed,
                ),
                ColorFormat::Rgba32 => Frame::from_rgba_speed(
                    info.width() as u16,
                    info.height() as u16,
                    &mut image_data,
                    config.speed,
                ),
            };

            gif_frame.dispose = gif::DisposalMethod::Background;
            gif_frame.delay = delay;

            encoder
                .write_frame(&gif_frame)
                .map_err(|e| format!("フレーム書き込みエラー: {}", e))?;
        }

        info.rest_time_disp(frame, info.num_frames());
    }
    Ok(())
}

struct GifOutputPlugin;

impl OutputPlugin for GifOutputPlugin {
    type Error = String;

    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "GIF出力プラグイン".into(),
            file_filter: FileFilter::new()
                .add("GIF Files (*.gif)", "*.gif")
                .add("All Files (*)", "*"),
            information: format!(
                "GIF出力プラグイン v{} by yu7400ki",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn output(info: &OutputInfo) -> std::result::Result<(), String> {
        let config = Config::load();
        create_gif_from_video(info, &config).map_err(|e| format!("GIF出力エラー: {}", e))
    }

    fn config(hwnd: HWND, _dll_hinst: HINSTANCE) -> bool {
        let default_config = Config::load();

        if let Ok(result) = show_config_dialog(hwnd, default_config) {
            match result {
                Some(config) => {
                    // 設定を保存
                    if let Err(e) = config.save() {
                        let error_msg = format!("設定保存エラー: {}", e);
                        logger::warn(&error_msg);
                        MessageBox::warning(Some(hwnd), &error_msg, "警告");
                    }
                    true
                }
                None => false,
            }
        } else {
            logger::error("設定の取得に失敗しました。");
            MessageBox::error(Some(hwnd), "設定の取得に失敗しました。", "エラー");
            false
        }
    }
}

register_output_plugin!(GifOutputPlugin);
register_logger!();
