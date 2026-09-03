//! 符号化した .jxl を jxl-rs で読み直し、入力と設定に照らす

use jxl::api::{self, states::Initialized};
use jxl::bit_reader::BitReader;
use jxl::headers::encodings::UnconditionalCoder;
use jxl::headers::frame_header::FrameHeader;
use jxl::headers::{FileHeader, JxlHeader};
use jxl_encoder::{ColorType, Config, Encoder, QUALITY_RANGE};

const WIDTH: u32 = 48;
const HEIGHT: u32 = 32;

/// 1秒あたりのtick数。分子と分母の取り違えが値に出るよう互いに離す
const TPS_NUMERATOR: u32 = 30000;
const TPS_DENOMINATOR: u32 = 1001;

/// アニメーションの再生回数。±1の混入が値に出るよう1から離す
const NUM_PLAYS: u32 = 5;

/// 各フレームの表示時間。並びの取り違えが出るよう互いに違える
const DURATIONS: [u32; 3] = [3, 5, 7];

/// 出力が排水の受け皿を超える大きさのキャンバスの一辺
const LARGE_SIDE: u32 = 256;

/// 位置から決まる雑音。圧縮が効かないので出力が大きくなる
fn noise_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut state = 0x1234_5678u32;
    (0..(width as usize * height as usize * 4))
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

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
        quality: *QUALITY_RANGE.end(),
        effort: 3,
        num_plays: NUM_PLAYS,
        tps_numerator: TPS_NUMERATOR,
        tps_denominator: TPS_DENOMINATOR,
        max_threads: 2,
    }
}

/// `width` x `height` の `frames` を符号化する
fn encode_frames(
    config: Config,
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    durations: &[u32],
) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new(), width, height, frames.len() as u32, config).unwrap();
    for (frame, duration) in frames.iter().zip(durations) {
        encoder.add_frame(frame, *duration).unwrap();
    }
    encoder.finish().unwrap()
}

/// `durations` と同じ数の勾配のフレームを符号化する
fn encode(config: Config, durations: &[u32]) -> Vec<u8> {
    let frames: Vec<Vec<u8>> = (0..durations.len())
        .map(|index| frame(config.color_type, index as u32 * 5))
        .collect();
    encode_frames(config, WIDTH, HEIGHT, &frames, durations)
}

/// 読み戻した画像の全体
struct Decoded {
    info: api::JxlBasicInfo,
    profile: api::JxlColorProfile,
    frames: Vec<api::VisibleFrameInfo>,
    /// 合成前の各フレームが書かれたときのヘッダ
    headers: Vec<FrameHeader>,
    /// 合成後の各フレームのキャンバス全面
    pixels: Vec<Vec<u8>>,
}

/// 各フレームのヘッダを、記録された位置から読み出す
///
/// 位置が実際にヘッダを指していることを、可視フレームと重なる欄で検める。
fn frame_headers(encoded: &[u8], frames: &[api::VisibleFrameInfo]) -> Vec<FrameHeader> {
    let file_header =
        FileHeader::read(&mut BitReader::new(encoded)).expect("ファイルヘッダの読み出し");
    let nonserialized = file_header.frame_header_nonserialized();

    frames
        .iter()
        .map(|frame| {
            let mut reader = BitReader::new(&encoded[frame.file_offset as usize..]);
            let header = FrameHeader::read_unconditional(&(), &mut reader, &nonserialized)
                .unwrap_or_else(|error| {
                    panic!(
                        "{} 枚目のフレームヘッダの読み出し: {error}",
                        frame.index + 1
                    )
                });
            assert_eq!(
                header.duration,
                frame.duration_ticks,
                "{} 枚目のヘッダの位置がずれている",
                frame.index + 1
            );
            assert_eq!(
                header.is_last,
                frame.is_last,
                "{} 枚目のヘッダの位置がずれている",
                frame.index + 1
            );
            header
        })
        .collect()
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
    let profile = decoder.embedded_color_profile().clone();
    let (width, height) = info.size;
    decoder.set_pixel_format(match color_type {
        ColorType::Rgb8 => api::JxlPixelFormat::rgb8(info.extra_channels.len()),
        ColorType::Rgba8 => api::JxlPixelFormat::rgba8(info.extra_channels.len()),
    });

    let bytes_per_row = width * color_type.bytes_per_pixel();
    let mut pixels = Vec::new();
    while decoder.has_more_frames() {
        let with_frame = complete(
            decoder.process(&mut input, None).unwrap(),
            "フレーム情報の読み出し",
        );
        let mut frame = vec![0u8; bytes_per_row * height];
        decoder = {
            let mut buffers = [api::JxlOutputBuffer::new(&mut frame, height, bytes_per_row)];
            complete(
                with_frame.process(&mut input, &mut buffers, None).unwrap(),
                "画素の読み出し",
            )
        };
        pixels.push(frame);
    }

    let frames = decoder.scanned_frames().to_vec();
    Decoded {
        info,
        profile,
        headers: frame_headers(encoded, &frames),
        frames,
        pixels,
    }
}

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
}

/// 約した比が書かれる。そのままの比はヘッダの値域から外れて書けない
#[test]
fn a_reducible_tps_is_written_in_its_reduced_form() {
    let config = Config {
        tps_numerator: TPS_NUMERATOR * 2,
        tps_denominator: TPS_DENOMINATOR * 2,
        ..config(ColorType::Rgb8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgb8);

    let animation = decoded
        .info
        .animation
        .expect("アニメーションになっていない");
    assert_eq!(animation.tps_numerator, TPS_NUMERATOR);
    assert_eq!(animation.tps_denominator, TPS_DENOMINATOR);
    let ticks: Vec<u32> = decoded.frames.iter().map(|f| f.duration_ticks).collect();
    assert_eq!(ticks, DURATIONS, "約分が表示時間の意味を動かしている");
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
        assert!(decoded.info.uses_original_profile, "{color_type:?}");
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
        quality: 80.0,
        ..config(ColorType::Rgba8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgba8);

    assert_eq!(decoded.info.size, (WIDTH as usize, HEIGHT as usize));
    assert!(!decoded.info.uses_original_profile);
    assert_eq!(decoded.pixels.len(), DURATIONS.len());
}

/// インターリーブされた画素からαだけを取り出す
fn alpha_channel(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).map(|pixel| pixel[3]).collect()
}

/// インターリーブされた画素から色だけを取り出す
fn color_channels(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

/// 色が動く設定でも、αだけは入力のまま残る
#[test]
fn a_lossy_encoding_keeps_the_alpha_exact() {
    let config = Config {
        quality: 40.0,
        ..config(ColorType::Rgba8)
    };
    let decoded = decode(&encode(config, &DURATIONS), ColorType::Rgba8);

    assert_eq!(decoded.pixels.len(), DURATIONS.len());
    for (index, pixels) in decoded.pixels.iter().enumerate() {
        let source = frame(ColorType::Rgba8, index as u32 * 5);
        assert_eq!(
            alpha_channel(pixels),
            alpha_channel(&source),
            "{} 枚目のαが動いている",
            index + 1
        );
        assert!(
            color_channels(pixels) != color_channels(&source),
            "{} 枚目の色が入力と一致していて、非可逆になっていない",
            index + 1
        );
    }
}

/// 品質は出力の大きさに現れる
#[test]
fn the_quality_reaches_the_output() {
    let at = |quality| {
        encode(
            Config {
                quality,
                ..config(ColorType::Rgb8)
            },
            &DURATIONS,
        )
        .len()
    };
    let (low, high) = (at(10.0), at(95.0));
    assert!(low < high, "品質10で{low}バイト、品質95で{high}バイト");
}

/// 均衡は出力の中身に現れる
///
/// 大きさは均衡に対して単調でないので、バイト列の違いで見る。
#[test]
fn the_effort_reaches_the_output() {
    let at = |effort| {
        encode(
            Config {
                effort,
                ..config(ColorType::Rgb8)
            },
            &DURATIONS,
        )
    };
    assert_ne!(at(1), at(9));
}

/// 暗黙の既定に依らず、書いた色の解釈とビット深度が読み戻せる
#[test]
fn the_color_encoding_and_the_bit_depth_are_read_back() {
    let decoded = decode(
        &encode(config(ColorType::Rgb8), &DURATIONS),
        ColorType::Rgb8,
    );

    assert_eq!(
        decoded.info.bit_depth,
        api::JxlBitDepth::Int { bits_per_sample: 8 }
    );
    assert!(
        decoded.profile == api::JxlColorProfile::Simple(api::JxlColorEncoding::srgb(false)),
        "sRGBが書かれていない"
    );
}

/// 排水の受け皿を何周も回して書いた出力が、欠けずに読み戻せる
#[test]
fn a_large_frame_round_trips_across_several_drains() {
    let config = Config {
        color_type: ColorType::Rgba8,
        ..config(ColorType::Rgba8)
    };
    let source = noise_rgba(LARGE_SIDE, LARGE_SIDE);
    let encoded = encode_frames(
        config,
        LARGE_SIDE,
        LARGE_SIDE,
        std::slice::from_ref(&source),
        &[1],
    );

    let decoded = decode(&encoded, ColorType::Rgba8);
    assert_eq!(
        decoded.info.size,
        (LARGE_SIDE as usize, LARGE_SIDE as usize)
    );
    assert_eq!(decoded.pixels[0], source);
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

/// キャンバス全体を指す矩形 (x, y, 幅, 高さ)
const WHOLE: (u32, u32, u32, u32) = (0, 0, WIDTH, HEIGHT);

/// フレームごとに書き加えるブロック (x, y, 幅, 高さ)
///
/// xとy、幅と高さの取り違えが値に出るよう互いに違え、重なりを持たせない。
/// 取り違えた矩形もキャンバスに収まるので、値を見なければ食い違いが残る。
const BLOCKS: [(u32, u32, u32, u32); 3] = [(3, 7, 5, 11), (20, 4, 13, 9), (9, 18, 6, 12)];

/// `BLOCKS` を書き加えていくフレーム列の表示時間
const BLOCK_DURATIONS: [u32; BLOCKS.len() + 1] = [3, 5, 7, 11];

/// `block` の範囲を全チャネル反転した値で埋める
///
/// 反転した値は元の値と必ず違うので、範囲がそのまま差分の外接矩形になる。
fn invert_block(frame: &mut [u8], color_type: ColorType, block: (u32, u32, u32, u32)) {
    let (x, y, width, height) = block;
    let bytes_per_pixel = color_type.bytes_per_pixel();
    for row in 0..height as usize {
        let start = ((y as usize + row) * WIDTH as usize + x as usize) * bytes_per_pixel;
        for byte in &mut frame[start..start + width as usize * bytes_per_pixel] {
            *byte = !*byte;
        }
    }
}

/// 先頭が勾配で、以降は `BLOCKS` を1つずつ書き加えたフレーム列
///
/// 隣り合うフレームはブロック1つ分しか違わないので、差分矩形は画面の一部になる。
fn block_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let mut frames = vec![frame(color_type, 0)];
    for block in BLOCKS {
        let mut next = frames.last().expect("先頭フレームが無い").clone();
        invert_block(&mut next, color_type, block);
        frames.push(next);
    }
    frames
}

/// `block_frames` の各フレームが書き直す矩形
fn block_rects() -> Vec<(u32, u32, u32, u32)> {
    std::iter::once(WHOLE).chain(BLOCKS).collect()
}

/// `block_frames` を符号化する
fn encode_blocks(config: Config) -> Vec<u8> {
    encode_frames(
        config,
        WIDTH,
        HEIGHT,
        &block_frames(config.color_type),
        &BLOCK_DURATIONS,
    )
}

/// 各フレームのヘッダが示す矩形 (x, y, 幅, 高さ)
fn rects(headers: &[FrameHeader]) -> Vec<(u32, u32, u32, u32)> {
    headers
        .iter()
        .map(|header| {
            let (width, height) = header.size();
            (
                header.x0 as u32,
                header.y0 as u32,
                width as u32,
                height as u32,
            )
        })
        .collect()
}

/// 部分フレームを重ねた合成結果が、投入したフレームと一致する
#[test]
fn a_lossless_partial_frame_animation_round_trips_byte_for_byte() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = block_frames(color_type);
        let decoded = decode(&encode_blocks(config(color_type)), color_type);

        assert_eq!(decoded.pixels.len(), frames.len(), "{color_type:?}");
        for (index, (pixels, source)) in decoded.pixels.iter().zip(&frames).enumerate() {
            assert_eq!(pixels, source, "{color_type:?} の {} 枚目", index + 1);
        }
    }
}

/// 矩形は投入した変化の位置と大きさで決まる
#[test]
fn each_partial_frame_carries_its_own_rect() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_blocks(config(color_type)), color_type);
        assert_eq!(rects(&decoded.headers), block_rects(), "{color_type:?}");
    }
}

/// 先頭フレームは、続く変化がどれだけ小さくても全面で書かれる
#[test]
fn the_first_frame_covers_the_canvas() {
    const DOT: (u32, u32, u32, u32) = (1, 2, 1, 1);

    let color_type = ColorType::Rgb8;
    let base = frame(color_type, 0);
    let mut changed = base.clone();
    invert_block(&mut changed, color_type, DOT);

    let input = [base, changed];
    let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &input, &[3, 5]);
    let decoded = decode(&encoded, color_type);

    assert_eq!(rects(&decoded.headers), [WHOLE, DOT]);
    assert_eq!(decoded.pixels, input);
}
