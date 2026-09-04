mod config;
mod dialog;

use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin, write_or_discard,
};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use std::io::BufWriter;
use std::num::NonZeroUsize;
use webp_encoder::{ColorType, Config as EncoderConfig, Encoder, FrameDelay, Report};
use win32_ui::MessageBox;
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
        lossless: config.lossless,
        quality: config.quality,
        method: config.method,
        // 再生回数は0が無限ループなので、負の値もそこへ寄せる
        num_plays: config.repeat.max(0) as u32,
    }
}

/// 設定のスレッド数をエンコーダのワーカー数へ渡す形にする
fn encoder_workers(config: &Config) -> NonZeroUsize {
    NonZeroUsize::new(config.threads).unwrap_or(NonZeroUsize::MIN)
}

/// ログの深刻さ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warn,
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

/// 1フレームの表示時間 (scale / rate 秒) を求める
fn frame_delay(scale: i32, rate: i32) -> std::result::Result<FrameDelay, String> {
    let scale = to_u32(scale, "フレームレートのスケール")?;
    let rate = to_u32(rate, "フレームレート")?;
    FrameDelay::new(scale, rate).map_err(|e| format!("フレームレート設定エラー: {}", e))
}

fn create_webp_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let delay = frame_delay(info.scale(), info.rate())?;

    let width = to_u32(info.width(), "幅")?;
    let height = to_u32(info.height(), "高さ")?;
    let num_frames = to_u32(info.num_frames(), "フレーム数")?;

    write_or_discard(&info.savefile(), |output_file| {
        let mut encoder = Encoder::with_workers(
            BufWriter::new(output_file),
            width,
            height,
            num_frames,
            encoder_config(config),
            encoder_workers(config),
        )
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

        info.encode_frames(config.color_format, |frame_data| {
            encoder.add_frame(&frame_data, delay)
        })
        .map_err(|e| e.to_string())?;

        let (writer, report) = encoder
            .finish()
            .map_err(|e| format!("エンコーダー終了エラー: {}", e))?;

        writer
            .into_inner()
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

        for (severity, message) in report_messages(&report, num_frames) {
            match severity {
                Severity::Info => logger::info(&message),
                Severity::Warn => logger::warn(&message),
            }
        }
        Ok(())
    })
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

register_output_plugin!(WebpOutputPlugin);
register_logger!();

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

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

    /// 隣り合う `seed` の隔たり
    ///
    /// 非可逆が矩形を採る許容量は品質から決まるので、そこを動かしても素材の
    /// 意味が変わらないよう、隔たりを値域の幅で置く。
    const SEED_STEP: u32 = 64;

    /// 画素ごとに値の違う不透明なRGBA
    fn frame_of(seed: u32) -> Vec<u8> {
        (0..FRAME_HEIGHT)
            .flat_map(|y| {
                (0..FRAME_WIDTH).flat_map(move |x| {
                    [
                        (x * 7) as u8,
                        (y * 11) as u8,
                        (seed * SEED_STEP) as u8,
                        0xFF,
                    ]
                })
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
        let delay = frame_delay(1, 30).unwrap();

        write_or_discard(path, |output_file| {
            let mut encoder = Encoder::with_workers(
                BufWriter::new(output_file),
                FRAME_WIDTH,
                FRAME_HEIGHT,
                declared,
                encoder_config(&config),
                encoder_workers(&config),
            )
            .map_err(|e| e.to_string())?;

            for seed in 0..frames {
                encoder
                    .add_frame(&frame_of(seed), delay)
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
        for repeat in [0, 1, 5, 65535, i32::MAX] {
            assert_eq!(
                encoder_config(&Config {
                    repeat,
                    ..Config::default()
                })
                .num_plays,
                repeat as u32
            );
        }
    }

    /// 負のループ回数は無限ループとして渡る
    #[test]
    fn a_negative_repeat_becomes_an_infinite_loop() {
        assert_eq!(
            encoder_config(&Config {
                repeat: -1,
                ..Config::default()
            })
            .num_plays,
            0
        );
    }

    #[test]
    fn threads_are_passed_through_as_the_number_to_wake() {
        for threads in [1, 2, 7] {
            assert_eq!(
                encoder_workers(&Config {
                    threads,
                    ..Config::default()
                })
                .get(),
                threads
            );
        }
    }

    /// 0 は起こせないので1へ寄る
    #[test]
    fn a_zero_worker_count_becomes_one() {
        assert_eq!(
            encoder_workers(&Config {
                threads: 0,
                ..Config::default()
            })
            .get(),
            1
        );
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
