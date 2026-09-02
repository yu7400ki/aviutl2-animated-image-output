mod config;
mod dialog;

use avif_encoder::{ColorType, Config as EncoderConfig, Encoder, Usage};
use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin, write_or_discard,
};
use config::{ColorFormat, Config};
use dialog::show_config_dialog;
use std::io::BufWriter;
use win32_ui::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

/// 負の値をエンコーダへ渡さないためのi32からu32への変換
fn to_u32(value: i32, name: &str) -> std::result::Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{}が不正です: {}", name, value))
}

/// プラグイン設定をエンコーダの設定へ対応付ける
fn encoder_config(config: &Config, timescale: u32) -> EncoderConfig {
    EncoderConfig {
        color_type: match config.color_format {
            ColorFormat::Rgb24 => ColorType::Rgb8,
            ColorFormat::Rgba32 => ColorType::Rgba8,
        },
        quality: config.quality,
        speed: config.speed,
        yuv_format: config.yuv_format.into(),
        num_plays: config.repeat,
        timescale,
        max_threads: u32::try_from(config.threads).unwrap_or(u32::MAX),
    }
}

/// 符号化器へ渡す、素材の枚数と時間の刻み
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sequence {
    num_frames: u32,
    /// 1秒あたりの刻み数
    timescale: u32,
    /// 1フレームが占める刻み数
    duration: u32,
}

impl Sequence {
    /// 1フレームが `scale` / `rate` 秒の素材が `num_frames` 枚
    ///
    /// 刻みを `rate` に据えると、1フレームは `scale` 刻みになる。
    fn new(num_frames: i32, rate: i32, scale: i32) -> std::result::Result<Self, String> {
        Ok(Self {
            num_frames: to_u32(num_frames, "フレーム数")?,
            timescale: to_u32(rate, "フレームレート")?,
            duration: to_u32(scale, "フレームレートのスケール")?,
        })
    }

    /// 動きを持たない1枚の素材か
    fn single(&self) -> bool {
        self.num_frames == 1
    }
}

/// aomの動作の用途の説明
fn usage_label(usage: Usage) -> &'static str {
    match usage {
        Usage::AllIntra => "all-intra",
        Usage::GoodQuality => "good-quality",
        Usage::Realtime => "realtime",
    }
}

/// speedから解決された動作点を、利用者が読める形にする
fn operating_point_message(config: &EncoderConfig, sequence: &Sequence) -> String {
    let point = config.operating_point(sequence.single());
    format!(
        "speed {} → {} (cpu_used={})",
        config.speed,
        usage_label(point.usage),
        point.cpu_used
    )
}

fn create_avif_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let width = to_u32(info.width(), "幅")?;
    let height = to_u32(info.height(), "高さ")?;
    let sequence = Sequence::new(info.num_frames(), info.rate(), info.scale())?;

    let encoder_config = encoder_config(config, sequence.timescale);

    write_or_discard(&info.savefile(), |output_file| {
        let mut encoder = Encoder::new(
            BufWriter::new(output_file),
            width,
            height,
            sequence.num_frames,
            encoder_config,
        )
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

        logger::info(&operating_point_message(&encoder_config, &sequence));

        info.encode_frames(config.color_format, |frame_data| {
            encoder.add_frame(&frame_data, sequence.duration)
        })
        .map_err(|e| e.to_string())?;

        let writer = encoder
            .finish()
            .map_err(|e| format!("エンコーダー終了エラー: {}", e))?;

        writer
            .into_inner()
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

        Ok(())
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use avif_encoder::YuvFormat;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn negative_dimensions_are_rejected() {
        assert_eq!(to_u32(1920, "幅").unwrap(), 1920);
        assert!(to_u32(-1, "幅").is_err());
    }

    #[test]
    fn color_format_maps_to_the_matching_color_type() {
        let rgb = encoder_config(
            &Config {
                color_format: ColorFormat::Rgb24,
                ..Config::default()
            },
            30,
        );
        assert_eq!(rgb.color_type, ColorType::Rgb8);

        let rgba = encoder_config(
            &Config {
                color_format: ColorFormat::Rgba32,
                ..Config::default()
            },
            30,
        );
        assert_eq!(rgba.color_type, ColorType::Rgba8);
    }

    #[test]
    fn yuv_format_maps_to_the_matching_encoder_format() {
        let config = |yuv_format| Config {
            yuv_format,
            ..Config::default()
        };

        assert_eq!(
            encoder_config(&config(config::YuvFormat::Yuv420), 30).yuv_format,
            YuvFormat::Yuv420
        );
        assert_eq!(
            encoder_config(&config(config::YuvFormat::Yuv422), 30).yuv_format,
            YuvFormat::Yuv422
        );
        assert_eq!(
            encoder_config(&config(config::YuvFormat::Yuv444), 30).yuv_format,
            YuvFormat::Yuv444
        );
    }

    #[test]
    fn repeat_is_passed_through_as_the_number_of_plays() {
        assert_eq!(
            encoder_config(
                &Config {
                    repeat: 5,
                    ..Config::default()
                },
                30
            )
            .num_plays,
            5
        );
        assert_eq!(
            encoder_config(
                &Config {
                    repeat: 0,
                    ..Config::default()
                },
                30
            )
            .num_plays,
            0
        );
    }

    /// 1フレームがscale / rate秒なので、rateが刻み数、scaleが1フレームの長さ
    #[test]
    fn the_rate_becomes_the_timescale_and_the_scale_becomes_the_duration() {
        let sequence = Sequence::new(24, 30000, 1001).unwrap();
        assert_eq!(sequence.timescale, 30000);
        assert_eq!(sequence.duration, 1001);
    }

    #[test]
    fn a_negative_frame_count_or_frame_rate_is_rejected() {
        assert!(Sequence::new(-1, 30000, 1001).is_err());
        assert!(Sequence::new(24, -1, 1001).is_err());
        assert!(Sequence::new(24, 30000, -1).is_err());
    }

    #[test]
    fn threads_are_passed_through_up_to_the_field_width() {
        assert_eq!(
            encoder_config(
                &Config {
                    threads: 4,
                    ..Config::default()
                },
                30
            )
            .max_threads,
            4
        );
        assert_eq!(
            encoder_config(
                &Config {
                    threads: usize::MAX,
                    ..Config::default()
                },
                30
            )
            .max_threads,
            u32::MAX
        );
    }

    /// speedから解決された動作点は、speedの値と用途の両方を含む
    #[test]
    fn the_operating_point_message_names_the_speed_and_the_usage() {
        let config = encoder_config(
            &Config {
                speed: 8,
                ..Config::default()
            },
            30,
        );

        let message = operating_point_message(&config, &Sequence::new(24, 30, 1).unwrap());
        assert!(message.contains("speed 8"), "{message}");
        assert!(message.contains("realtime"), "{message}");
    }

    /// 1枚だけの素材は、動きを持つ素材とは別の動作点へ解決される
    #[test]
    fn a_lone_frame_resolves_to_all_intra() {
        let config = encoder_config(
            &Config {
                speed: 8,
                ..Config::default()
            },
            30,
        );

        let single = operating_point_message(&config, &Sequence::new(1, 30, 1).unwrap());
        assert!(single.contains("all-intra"), "{single}");

        let pair = operating_point_message(&config, &Sequence::new(2, 30, 1).unwrap());
        assert!(!pair.contains("all-intra"), "{pair}");
    }

    /// まだ存在しない一時ファイルの場所
    fn temp_path() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "avif-output-{}-{}.avif",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// 画素ごとに値の違う不透明なRGBA
    fn frame_of(seed: u8, width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| [(x * 7) as u8, (y * 11) as u8, seed, 0xFF]))
            .collect()
    }

    /// フレームを`declared`枚宣言し、`frames`枚だけ投入して閉じる
    fn write_animation(
        path: &std::path::Path,
        declared: u32,
        frames: u32,
    ) -> std::result::Result<(), String> {
        let width = 16;
        let height = 16;
        let config = encoder_config(
            &Config {
                color_format: ColorFormat::Rgba32,
                ..Config::default()
            },
            30,
        );

        write_or_discard(path, |output_file| {
            let mut encoder =
                Encoder::new(BufWriter::new(output_file), width, height, declared, config)
                    .map_err(|e| e.to_string())?;

            for seed in 0..frames {
                encoder
                    .add_frame(&frame_of(seed as u8, width, height), 1)
                    .map_err(|e| e.to_string())?;
            }

            encoder
                .finish()
                .map_err(|e| e.to_string())?
                .into_inner()
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    /// 書き出しはISOBMFFの `ftyp` から始まる
    #[test]
    fn a_written_file_starts_with_the_ftyp_box() {
        let path = temp_path();

        write_animation(&path, 4, 4).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(&bytes[4..8], b"ftyp");
    }

    /// 失敗した書き出しは、書きかけのファイルを残さない
    #[test]
    fn a_failed_write_leaves_no_file() {
        let path = temp_path();

        write_animation(&path, 4, 3).expect_err("宣言より少ないので閉じられない");

        assert!(!path.exists(), "{}", path.display());
    }
}
