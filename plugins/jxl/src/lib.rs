mod config;
mod dialog;

use aviutl2::{
    ConfigDialog, FileFilter, OutputInfo, OutputPlugin, PluginFlags, PluginInfo, register_logger,
    register_output_plugin, write_or_discard,
};
use config::{ColorFormat, Config};
use jxl_encoder::{ColorType, Config as EncoderConfig, Encoder};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use windows::Win32::Foundation::HWND;

/// 符号化器へ渡す、素材の枚数と時間の刻み
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sequence {
    num_frames: u32,
    /// 1秒あたりのtick数の分子
    tps_numerator: u32,
    /// 1秒あたりのtick数の分母
    tps_denominator: u32,
    /// 1フレームが占めるtick数
    duration: u32,
}

impl Sequence {
    /// 1フレームが `scale` / `rate` 秒の素材が `num_frames` 枚
    ///
    /// 1秒あたりのtick数を `rate` / `scale` に据えると、1フレームは1tickになる。
    fn new(num_frames: u32, scale: u32, rate: u32) -> Self {
        Self {
            num_frames,
            tps_numerator: rate,
            tps_denominator: scale,
            duration: 1,
        }
    }
}

/// プラグイン設定をエンコーダの設定へ対応付ける
fn encoder_config(config: &Config, sequence: Sequence) -> EncoderConfig {
    EncoderConfig {
        color_type: match config.color_format {
            ColorFormat::Rgb24 => ColorType::Rgb8,
            ColorFormat::Rgba32 => ColorType::Rgba8,
        },
        quality: f32::from(config.quality),
        effort: config.effort,
        num_plays: config.repeat,
        tps_numerator: sequence.tps_numerator,
        tps_denominator: sequence.tps_denominator,
        max_threads: config.threads.get(),
    }
}

/// `path` へ書き出し、`frames` が投入したフレームを閉じる
///
/// 途中で失敗したときは書きかけのファイルを残さない。
fn write_frames<F>(
    path: &Path,
    width: u32,
    height: u32,
    sequence: Sequence,
    config: &Config,
    frames: F,
) -> Result<(), String>
where
    F: FnOnce(&mut Encoder<BufWriter<File>>) -> Result<(), String>,
{
    write_or_discard(path, |output_file| {
        let mut encoder = Encoder::new(
            BufWriter::new(output_file),
            width,
            height,
            sequence.num_frames,
            encoder_config(config, sequence),
        )
        .map_err(|e| format!("エンコーダー初期化エラー: {}", e))?;

        frames(&mut encoder)?;

        encoder
            .finish()
            .map_err(|e| format!("エンコーダー終了エラー: {}", e))?
            .into_inner()
            .map_err(|e| format!("ファイル書き込みエラー: {}", e))?;

        Ok(())
    })
}

struct JxlOutputPlugin;

impl OutputPlugin for JxlOutputPlugin {
    type Config = Config;

    const FORMAT_NAME: &'static str = "JPEG XL";

    const HAS_CONFIG_DIALOG: bool = true;

    fn info() -> PluginInfo {
        PluginInfo {
            flags: PluginFlags::VIDEO,
            name: "JPEG XL出力プラグイン".into(),
            file_filter: FileFilter::new()
                .add("JPEG XL Files (*.jxl)", "*.jxl")
                .add("All Files (*)", "*"),
            information: format!(
                "JPEG XL出力プラグイン v{} by yu7400ki",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn encode(info: &OutputInfo, config: &Config) -> Result<(), String> {
        let video = info.video()?;
        let (width, height) = (video.width(), video.height());
        let sequence = Sequence::new(info.num_frames()?, info.scale()?, info.rate()?);

        write_frames(
            &info.savefile(),
            width,
            height,
            sequence,
            config,
            |encoder| {
                video
                    .encode_frames(config.color_format, |frame_data| {
                        encoder.add_frame(frame_data, sequence.duration)
                    })
                    .map_err(|e| e.to_string())
            },
        )
    }

    fn show_config_dialog(hwnd: HWND, config: Config) -> ConfigDialog<Config> {
        aviutl2::dialog::show_config_dialog(hwnd, Self::FORMAT_NAME, dialog::Inputs::new(&config))
    }
}

register_output_plugin!(JxlOutputPlugin);
register_logger!();

#[cfg(test)]
mod tests {
    use super::*;
    use aviutl2_host::{Host, Script, temp_path};
    use jxl::api::{self, states::Initialized};
    use std::num::NonZeroUsize;

    /// 書き出しを通すフレームの大きさ
    const FRAME_WIDTH: u32 = 32;
    const FRAME_HEIGHT: u32 = 16;

    /// 画素ごとに値の違う不透明なRGBA
    fn frame_of(seed: u32) -> Vec<u8> {
        (0..FRAME_HEIGHT)
            .flat_map(|y| {
                (0..FRAME_WIDTH)
                    .flat_map(move |x| [(x * 7) as u8, (y * 11) as u8, seed as u8, 0xFF])
            })
            .collect()
    }

    /// 書き出しを通す素材の刻み。分子と分母の取り違えが値に出るよう互いに離す
    const RATE: u32 = 30000;
    const SCALE: u32 = 1001;

    /// フレームを `declared` 枚宣言し、`frames` 枚だけ投入して閉じる
    fn write_animation(path: &Path, declared: u32, frames: u32) -> Result<(), String> {
        let config = Config {
            color_format: ColorFormat::Rgba32,
            ..Config::default()
        };
        let sequence = Sequence::new(declared, SCALE, RATE);

        write_frames(
            path,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            sequence,
            &config,
            |encoder| {
                for seed in 0..frames {
                    encoder
                        .add_frame(frame_of(seed), sequence.duration)
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )
    }

    /// 1フレームがscale / rate秒なので、rateが分子、scaleが分母
    #[test]
    fn the_rate_and_the_scale_become_the_ticks_per_second() {
        let sequence = Sequence::new(24, 1001, 30000);

        assert_eq!(sequence.num_frames, 24);
        assert_eq!(sequence.tps_numerator, 30000);
        assert_eq!(sequence.tps_denominator, 1001);
    }

    /// 1秒あたりのtick数がフレームレートそのものなので、1フレームは1tick
    #[test]
    fn every_frame_lasts_one_tick() {
        assert_eq!(Sequence::new(24, 1001, 30000).duration, 1);
        assert_eq!(Sequence::new(1, 1, 30).duration, 1);
    }

    /// 素材の刻みがそのままエンコーダの1秒あたりのtick数になる
    #[test]
    fn the_ticks_per_second_reach_the_encoder() {
        let sequence = Sequence::new(24, 1001, 30000);
        let config = encoder_config(&Config::default(), sequence);

        assert_eq!(config.tps_numerator, 30000);
        assert_eq!(config.tps_denominator, 1001);
    }

    #[test]
    fn color_format_maps_to_the_matching_color_type() {
        let sequence = Sequence::new(24, 1, 30);

        let rgb = encoder_config(
            &Config {
                color_format: ColorFormat::Rgb24,
                ..Config::default()
            },
            sequence,
        );
        assert_eq!(rgb.color_type, ColorType::Rgb8);

        let rgba = encoder_config(
            &Config {
                color_format: ColorFormat::Rgba32,
                ..Config::default()
            },
            sequence,
        );
        assert_eq!(rgba.color_type, ColorType::Rgba8);
    }

    #[test]
    fn the_compression_settings_are_passed_through() {
        let sequence = Sequence::new(24, 1, 30);

        let lossy = encoder_config(
            &Config {
                quality: 80,
                effort: 3,
                repeat: 5,
                threads: NonZeroUsize::new(4).unwrap(),
                ..Config::default()
            },
            sequence,
        );
        assert_eq!(lossy.quality, 80.0);
        assert_eq!(lossy.effort, 3);
        assert_eq!(lossy.num_plays, 5);
        assert_eq!(lossy.max_threads, 4);

        let top_quality = encoder_config(
            &Config {
                quality: 100,
                effort: 9,
                repeat: 0,
                threads: NonZeroUsize::MIN,
                ..Config::default()
            },
            sequence,
        );
        assert_eq!(top_quality.quality, 100.0);
        assert_eq!(top_quality.effort, 9);
        assert_eq!(top_quality.num_plays, 0);
        assert_eq!(top_quality.max_threads, 1);
    }

    /// 段を1つ進める。入力を使い切らずに止まったら書き出しが不完全
    fn complete<T, U>(result: api::ProcessingResult<T, U>) -> T {
        match result {
            api::ProcessingResult::Complete { result } => result,
            api::ProcessingResult::NeedsMoreInput { size_hint, .. } => {
                panic!("読み出しが入力不足で止まった (あと {size_hint} バイト)")
            }
        }
    }

    /// 書き出した `.jxl` から、寸法とアニメーションの設定、フレームの並びを読み出す
    fn decode(
        bytes: &[u8],
    ) -> (
        (usize, usize),
        api::JxlAnimation,
        Vec<api::VisibleFrameInfo>,
    ) {
        let mut input = bytes;
        let decoder = api::JxlDecoder::<Initialized>::new(api::JxlDecoderOptions::default());
        let mut decoder = complete(decoder.process(&mut input, None).unwrap());
        let size = decoder.basic_info().size;
        let animation = decoder
            .basic_info()
            .animation
            .clone()
            .expect("アニメーションになっていない");

        while decoder.has_more_frames() {
            let with_frame = complete(decoder.process(&mut input, None).unwrap());
            decoder = complete(with_frame.skip_frame(&mut input).unwrap());
        }

        (size, animation, decoder.scanned_frames().to_vec())
    }

    /// 書き出しはJPEG XLのcodestreamの印から始まり、素材の刻みと全フレームを持つ
    #[test]
    fn a_written_file_carries_every_frame_of_the_material() {
        let path = temp_path("jxl");

        write_animation(&path, 4, 4).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(&bytes[..2], &[0xFF, 0x0A]);

        let (_, animation, frames) = decode(&bytes);
        assert_eq!(animation.tps_numerator, RATE);
        assert_eq!(animation.tps_denominator, SCALE);
        assert_eq!(frames.len(), 4);
        let ticks: Vec<u32> = frames.iter().map(|frame| frame.duration_ticks).collect();
        assert_eq!(ticks, [1, 1, 1, 1]);
    }

    /// 失敗した書き出しは、書きかけのファイルを残さない
    #[test]
    fn a_failed_write_leaves_no_file() {
        let path = temp_path("jxl");

        write_animation(&path, 4, 3).expect_err("宣言より少ないので閉じられない");

        assert!(!path.exists(), "{}", path.display());
    }

    /// 書けない1秒あたりのtick数は、直し方を添えて利用者へ届く
    #[test]
    fn an_unwritable_ticks_per_second_reaches_the_user() {
        let path = temp_path("jxl");
        let sequence = Sequence::new(2, 1025, 1);

        let message = write_frames(
            &path,
            FRAME_WIDTH,
            FRAME_HEIGHT,
            sequence,
            &Config::default(),
            |_| Ok(()),
        )
        .expect_err("分母が書ける値域の外");

        assert!(message.contains("1〜1024"), "{message}");
        assert!(!path.exists(), "{}", path.display());
    }

    /// 偽ホストから通す素材の枚数
    const HOST_FRAMES: u32 = 6;

    /// 偽ホストの素材の寸法・表示時間・枚数がそのまま`.jxl`になる
    ///
    /// 1秒あたりのtick数が素材の刻みそのものなので、1フレームは1tickになる。
    #[test]
    fn an_encoded_animation_carries_the_size_and_the_timing_of_the_material() {
        let path = temp_path("jxl");
        let host = Host::open(
            Script::new()
                .size(FRAME_WIDTH, FRAME_HEIGHT)
                .frame_rate(RATE, SCALE)
                .frames(HOST_FRAMES)
                .savefile(&path),
        );

        host.encode::<JxlOutputPlugin>(&Config::default())
            .expect("書き出し");

        let bytes = std::fs::read(&path).expect("書き出した先");
        std::fs::remove_file(&path).expect("書き出した先の後始末");

        let (size, animation, frames) = decode(&bytes);
        assert_eq!(
            size,
            (FRAME_WIDTH as usize, FRAME_HEIGHT as usize),
            "画像の寸法"
        );
        assert_eq!(
            (animation.tps_numerator, animation.tps_denominator),
            (RATE, SCALE),
            "1秒あたりのtick数"
        );
        assert_eq!(frames.len(), HOST_FRAMES as usize, "フレーム数");
        let ticks: Vec<u32> = frames.iter().map(|frame| frame.duration_ticks).collect();
        assert_eq!(ticks, vec![1; HOST_FRAMES as usize], "フレームごとのtick数");
    }

    /// ホストが返さなかったフレームで書き出しを止め、その先を取りに行かない
    #[test]
    fn a_frame_the_host_refuses_stops_the_encoding() {
        const MISSING: i32 = 3;

        let path = temp_path("jxl");
        let host = Host::open(
            Script::new()
                .size(FRAME_WIDTH, FRAME_HEIGHT)
                .frames(HOST_FRAMES)
                .missing_at(MISSING)
                .savefile(&path),
        );

        let error = host
            .encode::<JxlOutputPlugin>(&Config::default())
            .expect_err("返らないフレームがある");

        assert_eq!(
            host.get_video().len(),
            MISSING as usize + 1,
            "取りに行ったフレーム数"
        );
        assert_eq!(error, format!("フレーム取得エラー: フレーム {MISSING}"));
        assert!(!path.exists(), "{}", path.display());
    }
}
