mod config;
mod dialog;

use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin,
};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use png::{BitDepth, ColorType, Encoder};
use win32_dialog::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

fn create_apng_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = info.savefile();

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;
    let mut encoder = Encoder::new(output_file, info.width() as u32, info.height() as u32);

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
        .set_animated(info.num_frames() as u32, config.repeat)
        .map_err(|e| format!("APNG設定エラー: {}", e))?;

    encoder
        .set_frame_delay(info.scale() as u16, info.rate() as u16)
        .map_err(|e| format!("フレームレート設定エラー: {}", e))?;

    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

    for frame in 0..info.num_frames() {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }
        // カラーフォーマットに応じてフレームデータを取得
        let frame_data = info.get_video_frame(frame, config.color_format);

        if let Some(data) = frame_data {
            // フレームデータを書き込み
            writer
                .write_image_data(&data)
                .map_err(|e| format!("フレーム書き込みエラー: {}", e))?;
        }

        info.rest_time_disp(frame, info.num_frames());
    }

    writer
        .finish()
        .map_err(|e| format!("エンコーダー終了エラー: {}", e))?;
    Ok(())
}

struct ApngOutputPlugin;

impl OutputPlugin for ApngOutputPlugin {
    type Error = String;

    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "APNG出力プラグイン".into(),
            file_filter: FileFilter::new()
                .add("PNG Files (*.png)", "*.png")
                .add("All Files (*)", "*"),
            information: format!(
                "APNG出力プラグイン v{} by yu7400ki",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn output(info: &OutputInfo) -> std::result::Result<(), String> {
        // 設定を読み込み
        let config = Config::load();
        create_apng_from_video(info, &config).map_err(|e| format!("APNG出力エラー: {}", e))
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

register_output_plugin!(ApngOutputPlugin);
register_logger!();
