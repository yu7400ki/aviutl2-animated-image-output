mod config;
mod dialog;

use aviutl2::{
    ConfigDialog, FileFilter, OutputInfo, OutputPlugin, PluginFlags, PluginInfo,
    logger::{self, Severity},
    register_logger, register_output_plugin, workers, write_or_discard,
};
use config::{ColorFormat, Config};
use std::io::BufWriter;
use webp_encoder::{ColorType, Config as EncoderConfig, Encoder, Report};
use windows::Win32::Foundation::HWND;

/// プラグイン設定をエンコーダの設定へ対応付ける
fn encoder_config(config: &Config) -> EncoderConfig {
    EncoderConfig {
        color_type: match config.color_format {
            ColorFormat::Rgb24 => ColorType::Rgb8,
            ColorFormat::Rgba32 => ColorType::Rgba8,
        },
        lossless: config.lossless,
        quality: config.quality,
        method: config.method,
        num_plays: config.repeat,
    }
}

/// 出力が素材の見え方や並びと変わったところを並べる
///
/// 何も起きなければ1行も出さない。素材の透過の有無は変化ではないので載せない。
fn report_messages(report: &Report, num_frames: u32) -> Vec<(Severity, String)> {
    let mut messages = Vec::new();

    if report.merged_frames > 0 {
        messages.push((
            Severity::Info,
            format!(
                "フレームの併合: 前フレームと同じ{}フレームを表示時間の延長にまとめました (全{}フレーム)",
                report.merged_frames, num_frames
            ),
        ));
    }

    if report.delay_clamped {
        messages.push((
            Severity::Warn,
            "表示時間: 素材より遅く再生されます (1/1000秒へ引き上げ)".into(),
        ));
    }

    messages
}

struct WebpOutputPlugin;

impl OutputPlugin for WebpOutputPlugin {
    type Config = Config;

    const FORMAT_NAME: &'static str = "WebP";

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

    fn encode(info: &OutputInfo, config: &Config) -> Result<(), String> {
        let delay = info.frame_delay()?;

        let width = info.width()?;
        let height = info.height()?;
        let num_frames = info.num_frames()?;

        write_or_discard(&info.savefile(), |output_file| {
            let mut encoder = Encoder::with_workers(
                BufWriter::new(output_file),
                width,
                height,
                num_frames,
                encoder_config(config),
                workers(config.threads),
            )
            .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

            info.encode_frames(config.color_format, |frame_data| {
                encoder.add_frame(frame_data, delay)
            })
            .map_err(|e| e.to_string())?;

            let (writer, report) = encoder
                .finish()
                .map_err(|e| format!("エンコーダー終了エラー: {}", e))?;

            writer
                .into_inner()
                .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

            logger::report(report_messages(&report, num_frames));
            Ok(())
        })
    }

    fn show_config_dialog(hwnd: HWND, config: Config) -> ConfigDialog<Config> {
        dialog::show_config_dialog(hwnd, config)
    }
}

register_output_plugin!(WebpOutputPlugin);
register_logger!();

#[cfg(test)]
mod tests {
    use super::*;
    use aviutl2::MAX_REPEAT;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use webp_encoder::FrameDelay;

    /// テストで使うフレーム数
    const NUM_FRAMES: u32 = 120;

    /// 書き出しを通すフレームの大きさ
    const FRAME_WIDTH: u32 = 32;
    const FRAME_HEIGHT: u32 = 16;

    /// まだ存在しない一時ファイルの場所
    fn temp_path() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "webp-output-{}-{}.webp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// 画素ごとに値の違う不透明なRGBA
    fn frame_of(seed: u32) -> Vec<u8> {
        (0..FRAME_HEIGHT)
            .flat_map(|y| {
                (0..FRAME_WIDTH)
                    .flat_map(move |x| [(x * 7) as u8, (y * 11) as u8, seed as u8, 0xFF])
            })
            .collect()
    }

    /// フレームを `declared` 枚宣言し、`frames` 枚だけ投入して閉じる
    fn write_animation(
        path: &std::path::Path,
        declared: u32,
        frames: u32,
    ) -> std::result::Result<(), String> {
        let config = Config {
            color_format: ColorFormat::Rgba32,
            ..Config::default()
        };
        let delay = FrameDelay::new(1, 30).unwrap();

        write_or_discard(path, |output_file| {
            let mut encoder = Encoder::with_workers(
                BufWriter::new(output_file),
                FRAME_WIDTH,
                FRAME_HEIGHT,
                declared,
                encoder_config(&config),
                workers(config.threads),
            )
            .map_err(|e| e.to_string())?;

            for seed in 0..frames {
                encoder
                    .add_frame(frame_of(seed), delay)
                    .map_err(|e| e.to_string())?;
            }

            encoder
                .finish()
                .map_err(|e| e.to_string())?
                .0
                .into_inner()
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    /// 書き出しの経路がRIFFのサイズを書き戻す
    ///
    /// 後埋めは末尾まで書いた後のシークで起きるので、`BufWriter` を挟んだ
    /// ファイルでも効くことをここで確かめる。
    #[test]
    fn a_written_file_carries_the_riff_size() {
        let path = temp_path();

        write_animation(&path, 4, 4).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(size as usize, bytes.len() - 8);
    }

    /// 失敗した書き出しは、書きかけのファイルを残さない
    #[test]
    fn a_failed_write_leaves_no_file() {
        let path = temp_path();

        write_animation(&path, 4, 3).expect_err("宣言より少ないので閉じられない");

        assert!(!path.exists(), "{}", path.display());
    }

    /// 何も起きなかったときのレポート
    fn clean_report() -> Report {
        Report {
            merged_frames: 0,
            delay_clamped: false,
        }
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
        for repeat in [0, 1, 5, 65535, MAX_REPEAT] {
            assert_eq!(
                encoder_config(&Config {
                    repeat,
                    ..Config::default()
                })
                .num_plays,
                repeat
            );
        }
    }

    #[test]
    fn the_compression_settings_are_passed_through() {
        let lossy = encoder_config(&Config {
            lossless: false,
            quality: 80.0,
            method: 2,
            ..Config::default()
        });
        assert!(!lossy.lossless);
        assert_eq!(lossy.quality, 80.0);
        assert_eq!(lossy.method, 2);

        let lossless = encoder_config(&Config {
            lossless: true,
            quality: 100.0,
            method: 6,
            ..Config::default()
        });
        assert!(lossless.lossless);
        assert_eq!(lossless.quality, 100.0);
        assert_eq!(lossless.method, 6);
    }

    /// 素材のとおりに書けた出力は何も報せない
    #[test]
    fn a_faithful_output_says_nothing() {
        assert!(report_messages(&clean_report(), NUM_FRAMES).is_empty());
    }

    /// 併合したフレーム数は、全体のフレーム数を添えた1行になる
    #[test]
    fn merged_frames_are_reported_with_the_whole_count() {
        let report = Report {
            merged_frames: 42,
            ..clean_report()
        };

        let messages = report_messages(&report, NUM_FRAMES);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, Severity::Info);
        assert!(messages[0].1.contains("42"), "{messages:?}");
        assert!(messages[0].1.contains("120"), "{messages:?}");
    }

    /// 表示時間を切り上げたことは警告になる
    ///
    /// 素材より遅く再生されるので、併合と違って見え方が変わる。
    #[test]
    fn a_raised_duration_is_warned() {
        let report = Report {
            merged_frames: 3,
            delay_clamped: true,
        };

        let messages = report_messages(&report, NUM_FRAMES);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].0, Severity::Warn);
        assert!(messages[1].1.contains("1/1000秒"), "{messages:?}");
    }
}
