mod config;
mod dialog;

use apng_encoder::{ColorType, Config as EncoderConfig, Encoder};
use aviutl2::{
    ConfigDialog, FileFilter, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, register_logger,
    register_output_plugin, workers, write_or_discard,
};
use config::{ColorFormat, Config};
use std::io::BufWriter;
use windows::Win32::Foundation::HWND;

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

struct ApngOutputPlugin;

impl OutputPlugin for ApngOutputPlugin {
    type Config = Config;

    const FORMAT_NAME: &'static str = "APNG";

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

    fn encode(info: &OutputInfo, config: &Config) -> Result<(), String> {
        let delay = info.frame_delay()?;
        let video = info.video()?;
        let (width, height) = (video.width(), video.height());
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

            video
                .encode_frames(config.color_format, |frame_data| {
                    encoder
                        .add_frame(frame_data, delay)
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

    fn show_config_dialog(hwnd: HWND, config: Config) -> ConfigDialog<Config> {
        dialog::show_config_dialog(hwnd, config)
    }
}

register_output_plugin!(ApngOutputPlugin);
register_logger!();

#[cfg(test)]
mod tests {
    use super::*;
    use apng_encoder::{Error as EncoderError, FrameDelay};
    use std::fs::File;
    use std::io::Write;
    use std::num::NonZeroUsize;
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
    /// 先頭フレームは決定と書き出しの列を抜けるまで書き出されないため、
    /// シグネチャ・IHDR・acTL・fcTLちょうどの長さで受け付けを止めると、先頭フレームが
    /// 列を抜けたところで`add_frame`がそのIDATの書き出しでIoエラーを返す。失敗する
    /// までに実ファイルへ書き出されたバイト数も確かめ、書きかけの状態を経ることを示す。
    #[test]
    fn an_io_failure_while_writing_a_frame_leaves_no_file() {
        /// 先頭フレームの書き出しまで届くフレーム数
        ///
        /// 列を抜けるのに要る数より余裕を持たせている。ワーカー数は列の深さを決めるので
        /// 1つに固定する。
        const COUNT: u32 = 8;
        // シグネチャ(8) + IHDR(25) + acTL(20) + fcTL(38)
        const BUDGET: usize = 8 + 25 + 20 + 38;

        let path = temp_path();
        let delay = FrameDelay::new(1, 30).unwrap();
        let frame = vec![0u8; 8 * 8 * 4];

        let result = write_or_discard(&path, |file| {
            let probe = file.try_clone().map_err(|e| e.to_string())?;

            let mut encoder = Encoder::with_workers(
                FailingWriter {
                    file,
                    remaining: BUDGET,
                },
                8,
                8,
                COUNT,
                encoder_config(&Config {
                    color_format: ColorFormat::Rgba32,
                    ..Config::default()
                }),
                NonZeroUsize::MIN,
            )
            .map_err(|e| e.to_string())?;

            let error = (0..COUNT)
                .find_map(|_| encoder.add_frame(frame.clone(), delay).err())
                .expect("バッファを使い切るのでIoエラーになる");
            assert!(matches!(error, EncoderError::Io(_)), "{error}");

            let written = probe.metadata().map_err(|e| e.to_string())?.len();
            assert_eq!(written, BUDGET as u64, "失敗するまでに書き出したバイト数");

            Err(error.to_string())
        });

        result.expect_err("書き出しが失敗するので残らない");
        assert!(!path.exists(), "{}", path.display());
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
