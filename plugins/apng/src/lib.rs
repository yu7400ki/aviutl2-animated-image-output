mod config;
mod dialog;

use apng_encoder::{
    ColorType, Config as EncoderConfig, Encoder, Error as EncoderError, FrameDelay,
};
use aviutl2::{
    FileFilter, IniConfig, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, logger,
    register_logger, register_output_plugin, write_or_discard,
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
    }
}

/// 色数の削減が出力に及ぼした結果の説明
fn color_reduction_message(colors: u16) -> String {
    format!("色数の削減: パレットに置き換えました ({}色)", colors)
}

/// エンコーダのエラーの説明
///
/// 設定を変えれば避けられるものには、その道を添える。エンコーダは色数が溢れた
/// 事実だけを言い、どの設定を切れば書き出せるかはここが知っている。
fn encoder_error_message(error: &EncoderError) -> String {
    match error {
        EncoderError::ColorLimitExceeded { .. } => {
            format!(
                "{} (「{}」を無効にすると出力できます)",
                error,
                dialog::REDUCE_COLOR_LABEL
            )
        }
        error => error.to_string(),
    }
}

/// 1フレームの表示時間 (scale / rate 秒) を求める
fn frame_delay(scale: i32, rate: i32) -> std::result::Result<FrameDelay, String> {
    let scale = to_u32(scale, "フレームレートのスケール")?;
    let rate = to_u32(rate, "フレームレート")?;
    FrameDelay::new(scale, rate).map_err(|e| format!("フレームレート設定エラー: {}", e))
}

fn create_apng_from_video(info: &OutputInfo, config: &Config) -> std::result::Result<(), String> {
    let delay = frame_delay(info.scale(), info.rate())?;
    let width = to_u32(info.width(), "幅")?;
    let height = to_u32(info.height(), "高さ")?;
    let num_frames = to_u32(info.num_frames(), "フレーム数")?;

    write_or_discard(&info.savefile(), |output_file| {
        let mut encoder = Encoder::new(
            BufWriter::new(output_file),
            width,
            height,
            num_frames,
            encoder_config(config),
        )
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

        info.encode_frames(config.color_format, |frame_data| {
            encoder
                .add_frame(&frame_data, delay)
                .map_err(|e| encoder_error_message(&e))
        })
        .map_err(|e| e.to_string())?;

        // 終端はエンコーダを消費するため、報告する色数は先に読む
        let colors = encoder.palette_colors();

        encoder
            .finish()
            .map_err(|e| format!("エンコーダー終了エラー: {}", encoder_error_message(&e)))?
            .into_inner()
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

        if let Some(colors) = colors {
            logger::info(&color_reduction_message(colors));
        }
        Ok(())
    })
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
    use apng_encoder::delay_parts;
    use std::fs::File;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 賭けに出る先頭フレームと、パレットから溢れるフレームの大きさ
    const FRAME_WIDTH: u32 = 32;
    const FRAME_HEIGHT: u32 = 16;
    const FRAME_PIXELS: usize = FRAME_WIDTH as usize * FRAME_HEIGHT as usize;

    /// まだ存在しない一時ファイルの場所
    fn temp_path() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "png-output-{}-{}.png",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// 色数の削減を有効にした設定でエンコーダを作る
    fn reduce_color_encoder<W: Write + Seek>(writer: W, num_frames: u32) -> Encoder<W> {
        Encoder::new(
            writer,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            num_frames,
            encoder_config(&Config {
                color_format: ColorFormat::Rgba32,
                reduce_color: true,
                ..Config::default()
            }),
        )
        .unwrap()
    }

    /// `colors` 種類の色を敷き詰めた不透明なRGBA8のフレーム
    fn frame_of(colors: usize) -> Vec<u8> {
        (0..FRAME_PIXELS)
            .flat_map(|pixel| {
                let color = pixel % colors;
                [color as u8, (color >> 8) as u8, (color >> 16) as u8, 0xFF]
            })
            .collect()
    }

    /// 先頭からの絶対位置へのシークだけが失敗するファイル
    ///
    /// その向きのシークはPLTEとtRNSの書き戻しでしか起きないため、全フレームを
    /// 書き終えた後の書き戻しで失敗する。
    struct RewriteFails(File);

    impl Write for RewriteFails {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }

    impl Seek for RewriteFails {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            match pos {
                SeekFrom::Start(_) => Err(std::io::Error::other("書き戻し失敗")),
                pos => self.0.seek(pos),
            }
        }
    }

    /// 色数が溢れたエラーは、溢れたフレームと、切れば出力できる設定を指す
    ///
    /// 設定の名前はダイアログと同じものを引くため、項目名を変えても離れない。
    #[test]
    fn the_color_limit_error_points_at_the_setting_to_turn_off() {
        let message = encoder_error_message(&EncoderError::ColorLimitExceeded { frame: 42 });

        assert!(message.contains("フレーム 42"), "{message}");
        assert!(message.contains(dialog::REDUCE_COLOR_LABEL), "{message}");
    }

    /// 設定で避けられないエラーは、エンコーダの説明のまま届く
    #[test]
    fn an_unavoidable_error_keeps_the_encoder_wording() {
        let error = EncoderError::InvalidFrameDelay;

        assert_eq!(encoder_error_message(&error), error.to_string());
    }

    /// 色数がパレットから溢れた書き出しは、書きかけのファイルを残さない
    ///
    /// 先頭フレームは256色に収まるので賭けに出るが、次のフレームで溢れる。
    #[test]
    fn a_write_that_runs_out_of_palette_leaves_no_file() {
        let path = temp_path();
        let delay = frame_delay(1, 30).unwrap();

        let result = write_or_discard(&path, |file| {
            let mut encoder = reduce_color_encoder(BufWriter::new(file), 2);
            encoder
                .add_frame(&frame_of(2), delay)
                .map_err(|e| e.to_string())?;
            encoder
                .add_frame(&frame_of(FRAME_PIXELS), delay)
                .map_err(|e| e.to_string())?;
            encoder.finish().map_err(|e| e.to_string())?;
            Ok(())
        });

        let message = result.expect_err("色数が溢れるので失敗する");
        assert!(
            message.contains("色数がパレットに収まりません"),
            "{message}"
        );
        assert!(!path.exists(), "{}", path.display());
    }

    /// パレットの書き戻しに失敗した書き出しも、書きかけのファイルを残さない
    ///
    /// 全フレームを書き終えた後の失敗なので、残せばIENDだけが無く、詰め物の
    /// パレットを引くファイルになる。
    #[test]
    fn a_write_that_fails_while_settling_leaves_no_file() {
        let path = temp_path();
        let delay = frame_delay(1, 30).unwrap();

        let result = write_or_discard(&path, |file| {
            let mut encoder = reduce_color_encoder(BufWriter::new(RewriteFails(file)), 2);
            for _ in 0..2 {
                encoder
                    .add_frame(&frame_of(2), delay)
                    .map_err(|e| e.to_string())?;
            }
            encoder.finish().map_err(|e| e.to_string())?;
            Ok(())
        });

        result.expect_err("パレットを書き戻せないので失敗する");
        assert!(!path.exists(), "{}", path.display());
    }

    #[test]
    fn frame_rate_becomes_a_delay_in_seconds() {
        // 29.97fps
        assert_eq!(
            delay_parts(frame_delay(1001, 30000).unwrap()),
            delay_parts(FrameDelay::new(1001, 30000).unwrap())
        );
        // レートがu16を超えても切り詰めない
        assert_eq!(
            delay_parts(frame_delay(1001, 120000).unwrap()),
            (342, 40999)
        );
        assert_eq!(delay_parts(frame_delay(1, 60).unwrap()), (1, 60));
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

    /// パレットに置き換えたことと、載せた色数が文面に出る
    #[test]
    fn the_color_reduction_message_carries_the_color_count() {
        for colors in [1u16, 198, 256] {
            let message = color_reduction_message(colors);
            assert!(
                message.contains("パレットに置き換え"),
                "{colors} 色: {message}"
            );
            assert!(
                message.contains(&colors.to_string()),
                "{colors} 色: {message}"
            );
        }
    }
}
