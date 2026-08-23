mod config;
mod dialog;

use apng_encoder::{ColorReduction, ColorType, Config as EncoderConfig, Encoder, FrameDelay};
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

/// プラグイン設定をエンコーダの設定へ対応付ける
fn encoder_config(config: &Config) -> EncoderConfig {
    EncoderConfig {
        color_type: match config.color_format {
            ColorFormat::Rgb24 => ColorType::Rgb8,
            ColorFormat::Rgba32 => ColorType::Rgba8,
        },
        compression_level: config.compression_level,
        num_plays: config.repeat,
        reduce_color: config.reduce_color,
        ..EncoderConfig::default()
    }
}

/// 色数の最適化が出力の色種別に及ぼした結果の説明
fn color_reduction_message(reduction: ColorReduction) -> String {
    match reduction {
        ColorReduction::Palette { colors } => {
            format!("色数の最適化: パレットに置き換えました ({}色)", colors)
        }
        ColorReduction::AlphaDropped => "色数の最適化: アルファを削除しました".into(),
        ColorReduction::AlphaRequired => "色数の最適化: 透過があるためアルファを残しました".into(),
        ColorReduction::AlphaKept => {
            "色数の最適化: アルファを削除すると大きくなるため残しました".into()
        }
        ColorReduction::Kept => {
            "色数の最適化: 色数が多いため、カラーフォーマットのまま出力しました".into()
        }
        ColorReduction::Abandoned => {
            "色数の最適化: 解析に使えるメモリを超えたため、カラーフォーマットのまま出力しました"
                .into()
        }
    }
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

    let delay = frame_delay(info.scale(), info.rate())?;

    let mut encoder = Encoder::new(
        BufWriter::new(output_file),
        to_u32(info.width(), "幅")?,
        to_u32(info.height(), "高さ")?,
        to_u32(info.num_frames(), "フレーム数")?,
        encoder_config(config),
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

    // 色種別は最後のフレームまでに決まるが、書き出しの成否は終端まで分からない
    let reduction = encoder.color_reduction();

    encoder
        .finish()
        .map_err(|e| format!("エンコーダー終了エラー: {}", e))?
        .into_inner()
        .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

    if let Some(reduction) = reduction {
        logger::info(&color_reduction_message(reduction));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rate_becomes_a_delay_in_seconds() {
        // 29.97fps
        assert_eq!(
            frame_delay(1001, 30000).unwrap().to_parts(),
            FrameDelay::new(1001, 30000).unwrap().to_parts()
        );
        // レートがu16を超えても切り詰めない
        assert_eq!(frame_delay(1001, 120000).unwrap().to_parts(), (342, 40999));
        assert_eq!(frame_delay(1, 60).unwrap().to_parts(), (1, 60));
    }

    #[test]
    fn invalid_frame_rates_are_rejected() {
        assert!(frame_delay(1, 0).is_err());
        assert!(frame_delay(1, -30).is_err());
        assert!(frame_delay(-1, 30).is_err());
    }

    #[test]
    fn negative_dimensions_are_rejected() {
        assert_eq!(to_u32(1920, "幅").unwrap(), 1920);
        assert!(to_u32(-1, "幅").is_err());
    }

    #[test]
    fn color_format_maps_to_the_matching_color_type() {
        let rgb = encoder_config(&Config {
            color_format: ColorFormat::Rgb24,
            ..Config::default()
        });
        assert_eq!(rgb.color_type, ColorType::Rgb8);

        let rgba = encoder_config(&Config {
            color_format: ColorFormat::Rgba32,
            ..Config::default()
        });
        assert_eq!(rgba.color_type, ColorType::Rgba8);
    }

    #[test]
    fn repeat_and_compression_level_are_passed_through() {
        let encoder_config = encoder_config(&Config {
            repeat: 5,
            compression_level: 3,
            ..Config::default()
        });
        assert_eq!(encoder_config.num_plays, 5);
        assert_eq!(encoder_config.compression_level, 3);
    }

    #[test]
    fn the_color_reduction_setting_is_passed_through() {
        assert!(
            !encoder_config(&Config {
                reduce_color: false,
                ..Config::default()
            })
            .reduce_color
        );
        assert!(
            encoder_config(&Config {
                reduce_color: true,
                ..Config::default()
            })
            .reduce_color
        );
    }

    /// 結果ごとに決まった説明が出て、パレットの色数は文面に載る
    #[test]
    fn every_color_reduction_has_its_own_message() {
        let cases = [
            (
                ColorReduction::Palette { colors: 198 },
                "パレットに置き換え",
            ),
            (ColorReduction::AlphaDropped, "アルファを削除しました"),
            (ColorReduction::AlphaRequired, "透過があるため"),
            (ColorReduction::AlphaKept, "削除すると大きくなるため"),
            (ColorReduction::Kept, "色数が多いため"),
            (ColorReduction::Abandoned, "メモリを超えたため"),
        ];

        let messages: Vec<String> = cases
            .iter()
            .map(|&(reduction, expected)| {
                let message = color_reduction_message(reduction);
                assert!(
                    message.contains(expected),
                    "{reduction:?} の説明に {expected} が無い: {message}"
                );
                message
            })
            .collect();

        assert!(messages[0].contains("198"));
        for (index, message) in messages.iter().enumerate() {
            assert!(
                !messages[index + 1..].contains(message),
                "重複した説明: {message}"
            );
        }
    }
}
