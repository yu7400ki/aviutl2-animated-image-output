mod config;
mod dialog;

use apng_encoder::{ColorType, Config as EncoderConfig, Encoder, FrameDelay};
use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin,
};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use std::io::BufWriter;
use win32_dialog::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// 負の値をエンコーダへ渡さないためのi32からu32への変換
fn to_u32(value: i32, name: &str) -> std::result::Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{}が不正です: {}", name, value))
}

/// 1フレームの表示時間 (scale / rate 秒) を求める
fn frame_delay(scale: i32, rate: i32) -> std::result::Result<FrameDelay, String> {
    let scale = to_u32(scale, "フレームレートのスケール")?;
    let rate = to_u32(rate, "フレームレート")?;
    FrameDelay::new(scale, rate).map_err(|e| format!("フレームレート設定エラー: {}", e))
}

fn create_apng_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let output_path = info.savefile();

    let output_file =
        std::fs::File::create(&output_path).map_err(|e| format!("ファイル作成エラー: {}", e))?;

    let color_type = match config.color_format {
        ColorFormat::Rgb24 => ColorType::Rgb8,
        ColorFormat::Rgba32 => ColorType::Rgba8,
    };

    let delay = frame_delay(info.scale(), info.rate())?;

    let mut encoder = Encoder::new(
        BufWriter::new(output_file),
        to_u32(info.width(), "幅")?,
        to_u32(info.height(), "高さ")?,
        to_u32(info.num_frames(), "フレーム数")?,
        EncoderConfig {
            color_type,
            compression_level: config.compression_level,
            num_plays: config.repeat,
        },
    )
    .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

    for frame in 0..info.num_frames() {
        if info.is_abort() {
            return Err("処理が中断されました".into());
        }

        let frame_data = info
            .get_video_frame(frame, config.color_format)
            .ok_or_else(|| format!("フレーム取得エラー: フレーム {}", frame))?;

        encoder
            .add_frame(&frame_data, delay)
            .map_err(|e| format!("フレーム書き込みエラー: {}", e))?;

        info.rest_time_disp(frame, info.num_frames());
    }

    encoder
        .finish()
        .map_err(|e| format!("エンコーダー終了エラー: {}", e))?
        .into_inner()
        .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;
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
