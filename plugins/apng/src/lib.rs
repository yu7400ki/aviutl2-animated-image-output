mod config;
mod dialog;

use apng_encoder::{ColorType, Config as EncoderConfig, Encoder, FrameDelay};
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
fn encoder_config(config: &Config) -> EncoderConfig {
    EncoderConfig {
        color_type: match config.color_format {
            ColorFormat::Rgb24 => ColorType::Rgb8,
            ColorFormat::Rgba32 => ColorType::Rgba8,
        },
        compression_level: config.compression_level,
        num_plays: config.repeat,
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
                .map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())?;

        encoder
            .finish()
            .map_err(|e| format!("エンコーダー終了エラー: {}", e))?
            .into_inner()
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

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
    use apng_encoder::{Error as EncoderError, delay_parts};
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// まだ存在しない一時ファイルの場所
    fn temp_path() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "png-output-{}-{}.png",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// 指定したバイト数までしか書き出せないファイル
    ///
    /// 受け付けた範囲は実ファイルへそのまま書き出し、超えた書き出しは
    /// `std::io::Error::other` で失敗する。
    struct FailingWriter {
        file: File,
        remaining: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf.len() > self.remaining {
                self.remaining = 0;
                return Err(std::io::Error::other("書き出し失敗"));
            }
            self.remaining -= buf.len();
            self.file.write_all(buf)?;
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }

    /// Ioエラーで失敗した書き出しは、書きかけのファイルを残さない
    ///
    /// 先頭フレームはdispose_opが決まる2フレーム目の投入まで書き出されないため、
    /// シグネチャ・IHDR・acTL・fcTLちょうどの長さで受け付けを止めると、2フレーム目を
    /// 投入した`add_frame`が先頭フレームのIDATの書き出しでIoエラーを返す。失敗する
    /// までに実ファイルへ書き出されたバイト数も確かめ、書きかけの状態を経ることを示す。
    #[test]
    fn an_io_failure_while_writing_a_frame_leaves_no_file() {
        let path = temp_path();
        let delay = frame_delay(1, 30).unwrap();
        let frame = vec![0u8; 8 * 8 * 4];
        // シグネチャ(8) + IHDR(25) + acTL(20) + fcTL(38)
        const BUDGET: usize = 8 + 25 + 20 + 38;

        let result = write_or_discard(&path, |file| {
            let probe = file.try_clone().map_err(|e| e.to_string())?;

            let mut encoder = Encoder::new(
                FailingWriter {
                    file,
                    remaining: BUDGET,
                },
                8,
                8,
                2,
                encoder_config(&Config {
                    color_format: ColorFormat::Rgba32,
                    ..Config::default()
                }),
            )
            .map_err(|e| e.to_string())?;

            encoder
                .add_frame(&frame, delay)
                .map_err(|e| e.to_string())?;

            let error = encoder
                .add_frame(&frame, delay)
                .expect_err("バッファを使い切るのでIoエラーになる");
            assert!(matches!(error, EncoderError::Io(_)), "{error}");

            let written = probe.metadata().map_err(|e| e.to_string())?.len();
            assert_eq!(written, BUDGET as u64, "失敗するまでに書き出したバイト数");

            Err(error.to_string())
        });

        result.expect_err("書き出しが失敗するので残らない");
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
}
