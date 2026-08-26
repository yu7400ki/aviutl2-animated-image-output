mod config;
mod dialog;

use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin,
};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use gif_encoder::{ColorType, Config as EncoderConfig, Encoder, FrameDelay, PaletteKind, Report};
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
        num_plays: config.repeat as u32,
        ..EncoderConfig::default()
    }
}

/// ログの深刻さ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warn,
}

/// カラーテーブルの据え方の説明
fn palette_message(palette: PaletteKind) -> String {
    match palette {
        PaletteKind::Exact { colors } => {
            format!(
                "パレット: 全フレームの色をそのまま載せました ({}色)",
                colors
            )
        }
        PaletteKind::ExactFromPrefix { colors } => format!(
            "パレット: 解析に使えるメモリを超えたため、先頭部分の色を載せました ({}色)",
            colors
        ),
        PaletteKind::Quantized { colors } => {
            format!("パレット: 全フレームから減色しました ({}色)", colors)
        }
        PaletteKind::QuantizedFromPrefix { colors } => format!(
            "パレット: 解析に使えるメモリを超えたため、先頭部分だけから減色しました ({}色)",
            colors
        ),
    }
}

/// 据えたカラーテーブルが素材の色を覆えたかの説明
///
/// 黒への退避はテーブルの形でしかなく、深刻さは写した画素数との組で決まる。
fn approximation_message(report: &Report) -> (Severity, String) {
    match (report.black_fallback, report.approximated_pixels) {
        (true, 0) => (
            Severity::Info,
            "色の再現: 全フレームに不透明な画素がありませんでした".into(),
        ),
        (true, pixels) => (
            Severity::Warn,
            format!(
                "色の再現: 解析した範囲が完全な透過だったため、不透明な{}画素をすべて黒にしました",
                pixels
            ),
        ),
        (false, 0) if report.binarized_pixels > 0 => (
            Severity::Info,
            "色の再現: 透過を2値にした後の全画素を元の色のまま出力しました".into(),
        ),
        (false, 0) => (
            Severity::Info,
            "色の再現: 全画素を元の色のまま出力しました".into(),
        ),
        (false, pixels) => (
            Severity::Info,
            format!("色の再現: {}画素をパレットの近い色へ置き換えました", pixels),
        ),
    }
}

/// 出力の見え方が入力と変わったところを並べる
fn report_messages(report: &Report) -> Vec<(Severity, String)> {
    let mut messages = vec![
        (Severity::Info, palette_message(report.palette)),
        approximation_message(report),
    ];

    if report.binarized_pixels > 0 {
        messages.push((
            Severity::Info,
            format!(
                "透過: GIFの透過は2値のため、{}画素を完全な透過として扱いました",
                report.binarized_pixels
            ),
        ));
    }

    if report.delay_clamped {
        messages.push((
            Severity::Warn,
            "表示時間: GIFで表現できるのは50fpsまでのため、1フレームの表示時間を2/100秒へ切り上げました".into(),
        ));
    }

    messages
}

/// 1フレームの表示時間 (scale / rate 秒) を求める
fn frame_delay(scale: i32, rate: i32) -> std::result::Result<FrameDelay, String> {
    let scale = to_u32(scale, "フレームレートのスケール")?;
    let rate = to_u32(rate, "フレームレート")?;
    FrameDelay::new(scale, rate).map_err(|e| format!("フレームレート設定エラー: {}", e))
}

fn create_gif_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
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

    let (writer, report) = encoder
        .finish()
        .map_err(|e| format!("エンコーダー終了エラー: {}", e))?;

    writer
        .into_inner()
        .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

    for (severity, message) in report_messages(&report) {
        match severity {
            Severity::Info => logger::info(&message),
            Severity::Warn => logger::warn(&message),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 何も起きなかったときのレポート
    fn clean_report() -> Report {
        Report {
            palette: PaletteKind::Exact { colors: 128 },
            rebuilds: 0,
            local_tables: 0,
            approximated_pixels: 0,
            black_fallback: false,
            binarized_pixels: 0,
            delay_clamped: false,
            peak_spool_bytes: 0,
        }
    }

    fn messages(report: &Report) -> Vec<String> {
        report_messages(report)
            .into_iter()
            .map(|(_, message)| message)
            .collect()
    }

    #[test]
    fn invalid_frame_rates_are_rejected() {
        assert!(frame_delay(1, 0).is_err());
        assert!(frame_delay(1, -30).is_err());
        assert!(frame_delay(-1, 30).is_err());
        assert!(frame_delay(1001, 30000).is_ok());
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
    fn repeat_is_passed_through_as_the_number_of_plays() {
        assert_eq!(
            encoder_config(&Config {
                repeat: 5,
                ..Config::default()
            })
            .num_plays,
            5
        );
        assert_eq!(
            encoder_config(&Config {
                repeat: 0,
                ..Config::default()
            })
            .num_plays,
            0
        );
    }

    /// 据え方ごとに決まった説明が出て、色数は文面に載る
    #[test]
    fn every_palette_kind_has_its_own_message() {
        let kinds = [
            PaletteKind::Exact { colors: 198 },
            PaletteKind::ExactFromPrefix { colors: 198 },
            PaletteKind::Quantized { colors: 198 },
            PaletteKind::QuantizedFromPrefix { colors: 198 },
        ];

        let messages: Vec<String> = kinds
            .iter()
            .map(|&palette| {
                let message = palette_message(palette);
                assert!(
                    message.contains("198"),
                    "{palette:?} に色数が無い: {message}"
                );
                message
            })
            .collect();

        for (index, message) in messages.iter().enumerate() {
            assert!(
                !messages[index + 1..].contains(message),
                "重複した説明: {message}"
            );
        }
    }

    /// 先頭部分から据えたことと、据えたものが覆えたことは別に出る
    #[test]
    fn a_palette_from_a_prefix_can_still_be_lossless() {
        let report = Report {
            palette: PaletteKind::ExactFromPrefix { colors: 64 },
            ..clean_report()
        };

        assert_eq!(
            approximation_message(&report),
            (
                Severity::Info,
                "色の再現: 全画素を元の色のまま出力しました".into()
            )
        );
    }

    /// 写した画素があれば、可逆の文面は出ずに画素数が出る
    #[test]
    fn approximated_pixels_replace_the_lossless_message() {
        let report = Report {
            palette: PaletteKind::Quantized { colors: 256 },
            approximated_pixels: 4096,
            ..clean_report()
        };

        let (severity, message) = approximation_message(&report);
        assert_eq!(severity, Severity::Info);
        assert!(message.contains("4096"), "{message}");
        assert!(!message.contains("元の色のまま"), "{message}");
    }

    /// 黒への退避は、写した画素があるときだけ警告になる
    #[test]
    fn a_black_fallback_is_read_with_the_approximated_pixels() {
        let harmless = Report {
            black_fallback: true,
            ..clean_report()
        };
        let (severity, message) = approximation_message(&harmless);
        assert_eq!(severity, Severity::Info);
        assert!(message.contains("不透明な画素がありません"), "{message}");

        let damaging = Report {
            black_fallback: true,
            approximated_pixels: 4096,
            ..clean_report()
        };
        let (severity, message) = approximation_message(&damaging);
        assert_eq!(severity, Severity::Warn);
        assert!(message.contains("4096"), "{message}");
        assert!(message.contains("黒"), "{message}");
    }

    /// 透過を潰した素材では、可逆の文面が正規化の後だと分かる形になる
    #[test]
    fn binarized_pixels_qualify_the_lossless_message() {
        let report = Report {
            binarized_pixels: 4096,
            ..clean_report()
        };

        let messages = messages(&report);
        assert!(messages[1].contains("透過を2値にした後"), "{:?}", messages);
        assert!(messages[2].contains("4096"), "{:?}", messages);
    }

    /// 何も起きなければ、パレットと色の再現だけが出る
    #[test]
    fn a_clean_run_reports_only_the_palette_and_the_colors() {
        let messages = report_messages(&clean_report());

        assert_eq!(messages.len(), 2);
        assert!(
            messages
                .iter()
                .all(|(severity, _)| *severity == Severity::Info)
        );
    }

    /// 遅延の切り上げは警告として出る
    #[test]
    fn a_clamped_delay_is_a_warning() {
        let report = Report {
            delay_clamped: true,
            ..clean_report()
        };

        let clamped: Vec<(Severity, String)> = report_messages(&report)
            .into_iter()
            .filter(|(_, message)| message.starts_with("表示時間:"))
            .collect();

        assert_eq!(clamped.len(), 1);
        assert_eq!(clamped[0].0, Severity::Warn);
        assert!(clamped[0].1.contains("50fps"), "{}", clamped[0].1);
    }

    /// 出さないと決めた列はログに現れない
    #[test]
    fn the_internal_counters_stay_out_of_the_log() {
        let report = Report {
            rebuilds: 7,
            local_tables: 13,
            peak_spool_bytes: 987_654_321,
            ..clean_report()
        };

        for message in messages(&report) {
            for hidden in ["7", "13", "987654321"] {
                assert!(!message.contains(hidden), "{message}");
            }
        }
    }
}
