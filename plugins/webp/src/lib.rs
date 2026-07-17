mod config;
mod dialog;
mod encoder;

use crate::encoder::{AnimEncoder, AnimFrame, WebPConfig};
use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo,
    register_output_plugin,
};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use win32_dialog::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

fn create_webp_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = info.savefile();

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;

    let mut webp_config = WebPConfig::new().map_err(|_| "WebPConfig初期化エラー")?;

    webp_config.quality = config.quality;
    webp_config.method = config.method as i32;
    webp_config.lossless = if config.lossless { 1 } else { 0 };
    webp_config.alpha_compression = 1;
    webp_config.thread_level = 1;

    let mut encoder = AnimEncoder::new(
        info.width() as u32,
        info.height() as u32,
        &webp_config,
        output_file,
    )
    .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

    encoder.set_loop_count(config.repeat);

    let duration_ms = (1000.0 * info.scale() as f64 / info.rate() as f64).max(1.0) as i32;
    let mut timestamp = 0;

    for frame in 0..info.num_frames() {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let image_data = info.get_video_frame(frame, config.color_format);

        if let Some(pixel_data) = image_data {
            let anim_frame = match config.color_format {
                ColorFormat::Rgb24 => AnimFrame::from_rgb(
                    &pixel_data,
                    info.width() as u32,
                    info.height() as u32,
                    timestamp,
                ),
                ColorFormat::Rgba32 => AnimFrame::from_rgba(
                    &pixel_data,
                    info.width() as u32,
                    info.height() as u32,
                    timestamp,
                ),
            };

            encoder
                .add_frame(anim_frame)
                .map_err(|e| format!("フレーム追加エラー: {}", e))?;
        }

        timestamp += duration_ms;
        info.rest_time_disp(frame, info.num_frames());
    }

    encoder
        .finalize()
        .map_err(|e| format!("エンコード完了エラー: {}", e))?;

    Ok(())
}

struct WebpOutputPlugin;

impl OutputPlugin for WebpOutputPlugin {
    type Error = String;

    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "WebP出力プラグイン".into(),
            file_filter: FileFilter::new()
                .add("WebP Files (*.webp)", "*.webp")
                .add("All Files (*)", "*"),
            information: format!(
                "WebP出力プラグイン v{} by yu7400ki",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn output(info: &OutputInfo) -> std::result::Result<(), String> {
        let config = Config::load();
        create_webp_from_video(info, &config).map_err(|e| format!("WebP出力エラー: {}", e))
    }

    fn config(hwnd: HWND, _dll_hinst: HINSTANCE) -> bool {
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
}

register_output_plugin!(WebpOutputPlugin);
