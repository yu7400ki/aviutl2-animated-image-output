mod config;
mod dialog;

use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin,
};
use rustavif::{BitDepth, Encoder, RgbFormat, RgbImage};
use win32_dialog::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

use config::{ColorFormat, Config};
use dialog::show_config_dialog;

fn rgb_format_for(color_format: ColorFormat) -> RgbFormat {
    match color_format {
        ColorFormat::Rgb24 => RgbFormat::Rgb,
        ColorFormat::Rgba32 => RgbFormat::Rgba,
    }
}

fn create_avif_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = info.savefile();

    let mut encoder = Encoder::new().map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;
    encoder.set_repetition_count(config.repeat);
    encoder.set_timescale(info.rate() as u64);
    encoder.set_quality(config.quality);
    encoder.set_speed(config.speed);
    encoder.set_max_threads(config.threads as u32);

    let width = info.width() as u32;
    let height = info.height() as u32;
    let num_frames = info.num_frames() as u32;

    for frame in 0..num_frames {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = info.get_video_frame(frame as i32, config.color_format);

        if let Some(mut pixel_data) = image_data {
            let rgb_pixels = RgbImage::from_pixels(
                width,
                height,
                BitDepth::Eight,
                rgb_format_for(config.color_format),
                &mut pixel_data,
            )
            .map_err(|e| format!("RGBピクセル作成エラー: {}", e))?;

            let image = rgb_pixels
                .to_yuv_image(config.yuv_format.into())
                .map_err(|e| format!("YUV画像変換エラー: {}", e))?;

            encoder
                .add_image(&image, info.scale() as u64, Default::default())
                .map_err(|e| format!("フレーム追加エラー: {}", e))?;
        }

        info.rest_time_disp(frame as i32, num_frames as i32);
    }

    let data = encoder
        .finish()
        .map_err(|e| format!("エンコード完了エラー: {}", e))?;

    std::fs::write(&output_path, data.as_slice())
        .map_err(|e| format!("ファイル保存エラー: {}", e))?;

    Ok(())
}

struct AvifOutputPlugin;

impl OutputPlugin for AvifOutputPlugin {
    type Error = String;

    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "AVIF出力プラグイン".into(),
            file_filter: FileFilter::new()
                .add("AVIF Files (*.avif)", "*.avif")
                .add("All Files (*)", "*"),
            information: format!(
                "AVIF出力プラグイン v{} by yu7400ki",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn output(info: &OutputInfo) -> std::result::Result<(), String> {
        let config = Config::load();
        create_avif_from_video(info, &config).map_err(|e| format!("AVIF出力エラー: {}", e))
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

register_output_plugin!(AvifOutputPlugin);
register_logger!();
