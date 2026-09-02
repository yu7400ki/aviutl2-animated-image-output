//! 符号化した .jxl を jxl-rs で読み直し、入力と設定に照らす

use jxl::api::{self, states::Initialized};
use jxl_encoder::{ColorType, Config, Encoder};

const WIDTH: u32 = 48;
const HEIGHT: u32 = 32;

/// 1秒あたりのtick数。分子と分母の取り違えが値に出るよう互いに離す
const TPS_NUMERATOR: u32 = 30000;
const TPS_DENOMINATOR: u32 = 1001;

/// アニメーションの再生回数。±1の混入が値に出るよう1から離す
const NUM_PLAYS: u32 = 5;

/// 各フレームの表示時間。並びの取り違えが総和と個別の両方に出るよう互いに違える
const DURATIONS: [u32; 3] = [3, 5, 7];

/// 横方向と縦方向で滑らかに変わる不透明なRGB。`phase` は絵をずらす
fn gradient_rgb(phase: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((WIDTH * HEIGHT * 3) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let x = (x + phase) % WIDTH;
            rgb.push((x * 255 / WIDTH) as u8);
            rgb.push((y * 255 / HEIGHT) as u8);
            rgb.push(((x + y) * 255 / (WIDTH + HEIGHT)) as u8);
        }
    }
    rgb
}

/// `gradient_rgb` に、左から右へ薄れるαを足したもの
fn gradient_rgba(phase: u32) -> Vec<u8> {
    gradient_rgb(phase)
        .chunks_exact(3)
        .enumerate()
        .flat_map(|(index, pixel)| {
            let x = (index as u32) % WIDTH;
            [pixel[0], pixel[1], pixel[2], (x * 255 / WIDTH) as u8]
        })
        .collect()
}

/// `color_type` の画素を `phase` ぶんずらした1フレーム
fn frame(color_type: ColorType, phase: u32) -> Vec<u8> {
    match color_type {
        ColorType::Rgb8 => gradient_rgb(phase),
        ColorType::Rgba8 => gradient_rgba(phase),
    }
}

fn config(color_type: ColorType) -> Config {
    Config {
        color_type,
        lossless: true,
        quality: 90.0,
        effort: 3,
        num_plays: NUM_PLAYS,
        tps_numerator: TPS_NUMERATOR,
        tps_denominator: TPS_DENOMINATOR,
        max_threads: 2,
    }
}

/// `durations` と同じ数のフレームを符号化する
fn encode(config: Config, durations: &[u32]) -> Vec<u8> {
    let frames: Vec<Vec<u8>> = (0..durations.len())
        .map(|index| frame(config.color_type, index as u32 * 5))
        .collect();

    let mut encoder =
        Encoder::new(Vec::new(), WIDTH, HEIGHT, durations.len() as u32, config).unwrap();
    for (frame, duration) in frames.iter().zip(durations) {
        encoder.add_frame(frame, *duration).unwrap();
    }
    encoder.finish().unwrap()
}

/// 読み戻した画像の全体
struct Decoded {
    info: api::JxlBasicInfo,
    frames: Vec<api::VisibleFrameInfo>,
    pixels: Vec<Vec<u8>>,
}

/// 段を1つ進める。入力を使い切らずに止まったら符号化が不完全
fn complete<T, U>(result: api::ProcessingResult<T, U>, what: &str) -> T {
    match result {
        api::ProcessingResult::Complete { result } => result,
        api::ProcessingResult::NeedsMoreInput { size_hint, .. } => {
            panic!("{what} が入力不足で止まった (あと {size_hint} バイト)")
        }
    }
}

/// 全フレームの画素とメタデータを読み出す
fn decode(encoded: &[u8], color_type: ColorType) -> Decoded {
    let mut input: &[u8] = encoded;
    let decoder = api::JxlDecoder::<Initialized>::new(api::JxlDecoderOptions::default());
    let mut decoder = complete(
        decoder.process(&mut input, None).unwrap(),
        "画像情報の読み出し",
    );

    let info = decoder.basic_info().clone();
    let extra_channels = info.extra_channels.len();
    decoder.set_pixel_format(match color_type {
        ColorType::Rgb8 => api::JxlPixelFormat::rgb8(extra_channels),
        ColorType::Rgba8 => api::JxlPixelFormat::rgba8(extra_channels),
    });

    let bytes_per_row = WIDTH as usize * color_type.bytes_per_pixel();
    let mut pixels = Vec::new();
    while decoder.has_more_frames() {
        let with_frame = complete(
            decoder.process(&mut input, None).unwrap(),
            "フレーム情報の読み出し",
        );
        let mut frame = vec![0u8; bytes_per_row * HEIGHT as usize];
        decoder = {
            let mut buffers = [api::JxlOutputBuffer::new(
                &mut frame,
                HEIGHT as usize,
                bytes_per_row,
            )];
            complete(
                with_frame.process(&mut input, &mut buffers, None).unwrap(),
                "画素の読み出し",
            )
        };
        pixels.push(frame);
    }

    Decoded {
        frames: decoder.scanned_frames().to_vec(),
        info,
        pixels,
    }
}

/// 表示時間は個別と総和の両方で読み戻す
#[test]
fn an_animation_carries_its_timing_and_loop_count() {
    let decoded = decode(
        &encode(config(ColorType::Rgba8), &DURATIONS),
        ColorType::Rgba8,
    );

    let animation = decoded
        .info
        .animation
        .expect("アニメーションになっていない");
    assert_eq!(animation.tps_numerator, TPS_NUMERATOR);
    assert_eq!(animation.tps_denominator, TPS_DENOMINATOR);
    assert_eq!(animation.num_loops, NUM_PLAYS);

    assert_eq!(decoded.frames.len(), DURATIONS.len());
    let ticks: Vec<u32> = decoded.frames.iter().map(|f| f.duration_ticks).collect();
    assert_eq!(ticks, DURATIONS);
    assert_eq!(
        ticks.iter().sum::<u32>(),
        DURATIONS.iter().sum::<u32>(),
        "1周の総表示時間が一致しない"
    );
}

/// 最後のフレームで閉じた完全なストリームの主張
#[test]
fn the_last_frame_closes_the_stream() {
    let decoded = decode(
        &encode(config(ColorType::Rgba8), &DURATIONS),
        ColorType::Rgba8,
    );

    assert_eq!(decoded.frames.len(), DURATIONS.len());
    for frame in &decoded.frames[..DURATIONS.len() - 1] {
        assert!(!frame.is_last, "{} 枚目で閉じている", frame.index + 1);
    }
    assert!(decoded.frames[DURATIONS.len() - 1].is_last);
}

#[test]
fn a_lossless_animation_round_trips_byte_for_byte() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode(config(color_type), &DURATIONS), color_type);
        assert_eq!(decoded.pixels.len(), DURATIONS.len());
        for (index, pixels) in decoded.pixels.iter().enumerate() {
            assert_eq!(
                pixels,
                &frame(color_type, index as u32 * 5),
                "{color_type:?} の {} 枚目",
                index + 1
            );
        }
    }
}

/// αの有無は入力の色種別が決める
#[test]
fn the_alpha_channel_follows_the_color_type() {
    let rgb = decode(
        &encode(config(ColorType::Rgb8), &DURATIONS),
        ColorType::Rgb8,
    );
    assert!(rgb.info.extra_channels.is_empty());

    let rgba = decode(
        &encode(config(ColorType::Rgba8), &DURATIONS),
        ColorType::Rgba8,
    );
    assert_eq!(rgba.info.extra_channels.len(), 1);
    assert!(!rgba.info.extra_channels[0].alpha_associated);
}

#[test]
fn a_lossy_encoding_decodes_at_the_declared_size() {
    let config = Config {
        lossless: false,
        quality: 80.0,
        ..config(ColorType::Rgba8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgba8);

    assert_eq!(decoded.info.size, (WIDTH as usize, HEIGHT as usize));
    assert!(!decoded.info.uses_original_profile);
    assert_eq!(decoded.pixels.len(), DURATIONS.len());
    for pixels in &decoded.pixels {
        assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
    }
}

/// 単葉はアニメーションを持たない静止画になる
#[test]
fn a_single_leaf_is_a_still_image() {
    let decoded = decode(&encode(config(ColorType::Rgb8), &[1]), ColorType::Rgb8);

    assert!(decoded.info.animation.is_none());
    assert_eq!(decoded.info.size, (WIDTH as usize, HEIGHT as usize));
    assert_eq!(decoded.frames.len(), 1);
    assert!(decoded.frames[0].is_last);
    assert_eq!(decoded.pixels[0], gradient_rgb(0));
}

/// 品質の上限は可逆の指定と同じ出力経路を通る
#[test]
fn the_top_quality_takes_the_lossless_path() {
    let by_quality = Config {
        lossless: false,
        quality: 100.0,
        ..config(ColorType::Rgba8)
    };
    let by_flag = Config {
        lossless: true,
        quality: 0.0,
        ..config(ColorType::Rgba8)
    };

    let decoded = decode(&encode(by_quality, &DURATIONS), ColorType::Rgba8);
    assert!(decoded.info.uses_original_profile);
    for (index, pixels) in decoded.pixels.iter().enumerate() {
        assert_eq!(pixels, &gradient_rgba(index as u32 * 5));
    }

    assert_eq!(
        encode(by_quality, &DURATIONS),
        encode(by_flag, &DURATIONS),
        "品質の上限が可逆と違うストリームを書いている"
    );
}
