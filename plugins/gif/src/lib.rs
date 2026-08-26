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

/// 3桁ごとに区切った延べ画素数と、全画素に対する割合
///
/// レポートの画素数はフレームをまたいだ延べ数なので、分母を添えないと
/// 多いのか少ないのか読めない。
fn pixel_share(pixels: u64, total_pixels: u64) -> String {
    let digits = pixels.to_string();
    let mut grouped = String::with_capacity(digits.len() * 4 / 3);
    for (index, digit) in digits.char_indices() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }

    if total_pixels == 0 {
        return format!("{}画素", grouped);
    }

    let percent = pixels as f64 * 100.0 / total_pixels as f64;
    if percent < 0.1 {
        format!("{}画素、全体の0.1%未満", grouped)
    } else {
        format!("{}画素、全体の{:.1}%", grouped, percent)
    }
}

/// カラーテーブルの据え方の説明。全フレームの色が載ったなら `None`
fn palette_message(palette: PaletteKind) -> Option<String> {
    match palette {
        PaletteKind::Exact { .. } => None,
        PaletteKind::ExactFromPrefix { colors } => Some(format!(
            "パレット: 解析に使えるメモリを超えたため、先頭部分の色を載せました ({}色)",
            colors
        )),
        PaletteKind::Quantized { colors } => Some(format!(
            "パレット: 全フレームから減色しました ({}色)",
            colors
        )),
        PaletteKind::QuantizedFromPrefix { colors } => Some(format!(
            "パレット: 解析に使えるメモリを超えたため、先頭部分だけから減色しました ({}色)",
            colors
        )),
    }
}

/// 透過を2値へ寄せたときに動いた画素の説明
///
/// 向きごとに失うものが違う。薄い側は見えていたものが消え、濃い側は
/// 透けていたものが透けなくなる。
fn binarization_messages(report: &Report, total_pixels: u64) -> Vec<(Severity, String)> {
    let mut messages = Vec::new();

    if report.binarized_to_transparent > 0 {
        messages.push((
            Severity::Info,
            format!(
                "透過: 完全な透過にしました ({})",
                pixel_share(report.binarized_to_transparent, total_pixels)
            ),
        ));
    }

    if report.binarized_to_opaque > 0 {
        messages.push((
            Severity::Info,
            format!(
                "透過: 不透明にしました ({})",
                pixel_share(report.binarized_to_opaque, total_pixels)
            ),
        ));
    }

    messages
}

/// 据えたカラーテーブルが素材の色を覆えなかったところの説明
///
/// 近似と代替は失うものが違う。近似は色がずれるだけだが、代替は写す先が
/// 無かった画素で、素材の色が画面に残らない。
fn color_messages(report: &Report, total_pixels: u64) -> Vec<(Severity, String)> {
    let mut messages = Vec::new();

    if report.approximated_pixels > 0 {
        messages.push((
            Severity::Info,
            format!(
                "色の再現: 近い色へ置き換えました ({})",
                pixel_share(report.approximated_pixels, total_pixels)
            ),
        ));
    }

    if report.substituted_pixels > 0 {
        messages.push((
            Severity::Warn,
            format!(
                "色の再現: 表せない色を黒にしました ({})",
                pixel_share(report.substituted_pixels, total_pixels)
            ),
        ));
    }

    messages
}

/// 出力の見え方が入力と変わったところを並べる
///
/// 何も起きなければ1行も出さない。可逆で不透明でレートに収まる書き出しは
/// 報せるところが無く、無言になる。
///
/// 透過の2値化は色を決めるより前に起きるので、色の再現より先に出す。
fn report_messages(report: &Report, total_pixels: u64) -> Vec<(Severity, String)> {
    let mut messages: Vec<(Severity, String)> = palette_message(report.palette)
        .map(|message| (Severity::Info, message))
        .into_iter()
        .collect();
    messages.extend(binarization_messages(report, total_pixels));
    messages.extend(color_messages(report, total_pixels));

    if report.delay_clamped {
        messages.push((
            Severity::Warn,
            "表示時間: 素材より遅く再生されます (2/100秒へ引き上げ)".into(),
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

    let width = to_u32(info.width(), "幅")?;
    let height = to_u32(info.height(), "高さ")?;
    let num_frames = to_u32(info.num_frames(), "フレーム数")?;
    let total_pixels = u64::from(width) * u64::from(height) * u64::from(num_frames);

    let mut encoder = Encoder::new(
        BufWriter::new(output_file),
        width,
        height,
        num_frames,
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

    for (severity, message) in report_messages(&report, total_pixels) {
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

    /// テストで使う延べ画素数 (割合が読みやすい丸い数)
    const TOTAL_PIXELS: u64 = 1_000_000;

    /// 何も起きなかったときのレポート
    fn clean_report() -> Report {
        Report {
            palette: PaletteKind::Exact { colors: 128 },
            rebuilds: 0,
            local_tables: 0,
            approximated_pixels: 0,
            substituted_pixels: 0,
            black_fallback: false,
            binarized_to_transparent: 0,
            binarized_to_opaque: 0,
            delay_clamped: false,
            peak_spool_bytes: 0,
        }
    }

    fn messages(report: &Report) -> Vec<String> {
        report_messages(report, TOTAL_PIXELS)
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

    /// 延べ画素数には桁区切りと分母が付く
    #[test]
    fn a_pixel_count_carries_its_share_of_the_whole() {
        assert_eq!(
            pixel_share(1_234_567, 10_000_000),
            "1,234,567画素、全体の12.3%"
        );
        assert_eq!(pixel_share(123, 1000), "123画素、全体の12.3%");
        assert_eq!(pixel_share(1, 1_000_000), "1画素、全体の0.1%未満");
        assert_eq!(pixel_share(4096, 0), "4,096画素");
    }

    /// 全フレームの色がそのまま載ったなら、パレットの説明は出ない
    #[test]
    fn a_palette_that_holds_every_color_is_not_reported() {
        assert_eq!(palette_message(PaletteKind::Exact { colors: 198 }), None);
    }

    /// 据え方ごとに決まった説明が出て、色数は文面に載る
    #[test]
    fn every_reported_palette_kind_has_its_own_message() {
        let kinds = [
            PaletteKind::ExactFromPrefix { colors: 198 },
            PaletteKind::Quantized { colors: 198 },
            PaletteKind::QuantizedFromPrefix { colors: 198 },
        ];

        let messages: Vec<String> = kinds
            .iter()
            .map(|&palette| {
                let message = palette_message(palette).expect("説明が無い");
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

    /// 先頭部分から据えたことは、劣化が無くても出る
    #[test]
    fn a_palette_from_a_prefix_is_reported_even_when_lossless() {
        let report = Report {
            palette: PaletteKind::ExactFromPrefix { colors: 64 },
            ..clean_report()
        };

        assert_eq!(
            report_messages(&report, TOTAL_PIXELS),
            vec![(
                Severity::Info,
                "パレット: 解析に使えるメモリを超えたため、先頭部分の色を載せました (64色)".into()
            )]
        );
    }

    /// 近似した画素は、深刻さの無い1行になる
    #[test]
    fn approximated_pixels_are_reported_without_a_warning() {
        let report = Report {
            palette: PaletteKind::Quantized { colors: 256 },
            approximated_pixels: 4096,
            ..clean_report()
        };

        let colors = color_messages(&report, TOTAL_PIXELS);
        assert_eq!(
            colors,
            vec![(
                Severity::Info,
                "色の再現: 近い色へ置き換えました (4,096画素、全体の0.4%)".into()
            )]
        );
    }

    /// 代替した画素は、近似と別の行で警告になる
    ///
    /// 近い色へ寄せたのではなく、写す先が無くて色を失っている。
    #[test]
    fn substituted_pixels_are_warned_apart_from_the_approximated_ones() {
        let report = Report {
            approximated_pixels: 4096,
            substituted_pixels: 8192,
            ..clean_report()
        };

        let colors = color_messages(&report, TOTAL_PIXELS);
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[0].0, Severity::Info);
        assert_eq!(colors[1].0, Severity::Warn);
        assert!(colors[1].1.contains("8,192画素"), "{colors:?}");
        assert!(colors[1].1.contains("黒"), "{colors:?}");
        assert!(!colors[1].1.contains("近い色"), "{colors:?}");
    }

    /// 写す先の黒を足しただけで、そこへ写した画素が無ければ何も出ない
    #[test]
    fn a_black_fallback_without_substituted_pixels_says_nothing() {
        let report = Report {
            black_fallback: true,
            ..clean_report()
        };

        assert!(color_messages(&report, TOTAL_PIXELS).is_empty());
    }

    /// 2値化の向きごとに別の行が出る
    #[test]
    fn the_two_directions_of_binarization_are_reported_apart() {
        let report = Report {
            binarized_to_transparent: 4096,
            binarized_to_opaque: 8192,
            ..clean_report()
        };

        let lines = binarization_messages(&report, TOTAL_PIXELS);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].1.contains("4,096画素"), "{lines:?}");
        assert!(lines[0].1.contains("完全な透過"), "{lines:?}");
        assert!(lines[1].1.contains("8,192画素"), "{lines:?}");
        assert!(lines[1].1.contains("不透明"), "{lines:?}");
    }

    /// 2値化の説明は、それを前提にする色の再現より先に出る
    #[test]
    fn the_binarization_is_explained_before_the_colors() {
        let report = Report {
            binarized_to_transparent: 4096,
            approximated_pixels: 4096,
            ..clean_report()
        };

        let messages = messages(&report);
        assert_eq!(messages.len(), 2);
        assert!(messages[0].starts_with("透過:"), "{messages:?}");
        assert!(messages[1].starts_with("色の再現:"), "{messages:?}");
    }

    /// 何も起きなければ1行も出さない
    ///
    /// 可逆で不透明でレートに収まる書き出しは、報せるところが無い。
    #[test]
    fn a_clean_run_says_nothing() {
        assert!(report_messages(&clean_report(), TOTAL_PIXELS).is_empty());
    }

    /// 遅延の切り上げは、再生が遅くなることまで書いた警告になる
    #[test]
    fn a_clamped_delay_warns_that_playback_slows_down() {
        let report = Report {
            delay_clamped: true,
            ..clean_report()
        };

        let clamped = report_messages(&report, TOTAL_PIXELS);
        assert_eq!(clamped.len(), 1);
        assert_eq!(clamped[0].0, Severity::Warn);
        assert!(clamped[0].1.starts_with("表示時間:"), "{}", clamped[0].1);
        assert!(
            clamped[0].1.contains("遅く再生されます"),
            "{}",
            clamped[0].1
        );
    }

    /// 出さないと決めた列はログに現れない
    #[test]
    fn the_internal_counters_stay_out_of_the_log() {
        let report = Report {
            rebuilds: 37,
            local_tables: 41,
            peak_spool_bytes: 987_654_321,
            approximated_pixels: 4096,
            ..clean_report()
        };

        for message in messages(&report) {
            for hidden in ["37", "41", "987654321", "987,654,321"] {
                assert!(!message.contains(hidden), "{message}");
            }
        }
    }
}
