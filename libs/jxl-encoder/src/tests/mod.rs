//! 符号化した .jxl を jxl-rs と自前の復号器で読み直し、入力と設定に照らす

mod closed;
mod layers;

use crate::layers::Layers;
use crate::{ColorType, Config, Encoder, QUALITY_RANGE};
use anim_core::Rect;
use jxl::api::{self, states::Initialized};
use jxl::bit_reader::BitReader;
use jxl::headers::encodings::UnconditionalCoder;
use jxl::headers::frame_header::{BlendingMode, FrameHeader, FrameType};
use jxl::headers::toc::{Toc, TocNonserialized};
use jxl::headers::{FileHeader, JxlHeader};
use std::io::Write;

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

/// `width` x `height` の `frames` を `writer` へ符号化する
fn encode_into<W: Write>(
    writer: W,
    config: Config,
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    durations: &[u32],
) -> W {
    let mut encoder = Encoder::new(writer, width, height, frames.len() as u32, config).unwrap();
    for (frame, duration) in frames.iter().zip(durations) {
        encoder.add_frame(frame, *duration).unwrap();
    }
    encoder.finish().unwrap()
}

/// `width` x `height` の `frames` を符号化する
fn encode_frames(
    config: Config,
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    durations: &[u32],
) -> Vec<u8> {
    encode_into(Vec::new(), config, width, height, frames, durations)
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
    /// 書かれた順の通常フレームのヘッダ。表示時間0の副フレームを含む
    headers: Vec<FrameHeader>,
    /// 合成後の各フレームのキャンバス全面
    pixels: Vec<Vec<u8>>,
}

/// 表示時間を持つヘッダ
///
/// 表示時間0の副フレームは次の表示フレームへ畳まれるので、単独では表示されない。
/// 静止画は表示時間の欄を持たないので、ストリームを閉じる1枚がそのまま表示される。
fn displayed(headers: &[FrameHeader]) -> Vec<&FrameHeader> {
    headers
        .iter()
        .filter(|header| header.duration != 0 || header.is_last)
        .collect()
}

/// コードストリームを順に歩き、通常フレームのヘッダを読み出す
///
/// 表示時間0の副フレームは `scanned_frames` に現れないので、記録された位置からは
/// 届かない。libjxlがpatchのために書く参照フレームは、こちらが並べたものでは
/// ないので落とす。読めた並びが実体であることを、表示フレームと重なる欄で検める。
fn frame_headers(encoded: &[u8], frames: &[api::VisibleFrameInfo]) -> Vec<FrameHeader> {
    let mut reader = BitReader::new(encoded);
    let file_header = FileHeader::read(&mut reader).expect("ファイルヘッダの読み出し");
    let nonserialized = file_header.frame_header_nonserialized();

    let mut headers: Vec<FrameHeader> = Vec::new();
    loop {
        reader
            .jump_to_byte_boundary()
            .expect("フレームの先頭への整列");
        let header = FrameHeader::read_unconditional(&(), &mut reader, &nonserialized)
            .unwrap_or_else(|error| {
                panic!(
                    "{} 枚目のフレームヘッダの読み出し: {error}",
                    headers.len() + 1
                )
            });
        let toc = Toc::read_unconditional(
            &(),
            &mut reader,
            &TocNonserialized {
                num_entries: header.num_toc_entries() as u32,
            },
        )
        .unwrap_or_else(|error| panic!("{} 枚目のTOCの読み出し: {error}", headers.len() + 1));
        reader.jump_to_byte_boundary().expect("節の先頭への整列");
        let section_bytes: u32 = toc.entries.iter().sum();

        let is_last = header.is_last;
        if header.frame_type == FrameType::RegularFrame {
            headers.push(header);
        }
        if is_last {
            break;
        }
        reader
            .skip_bits(section_bytes as usize * 8)
            .expect("次のフレームへの読み飛ばし");
    }

    let display = displayed(&headers);
    assert_eq!(
        display.len(),
        frames.len(),
        "表示フレームの数が読み戻した可視フレームと違う"
    );
    for (header, frame) in display.iter().zip(frames) {
        assert_eq!(
            header.duration,
            frame.duration_ticks,
            "{} 枚目のヘッダが読み戻した可視フレームと違う",
            frame.index + 1
        );
        assert_eq!(
            header.is_last,
            frame.is_last,
            "{} 枚目のヘッダが読み戻した可視フレームと違う",
            frame.index + 1
        );
    }
    headers
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

/// 部分フレームは、直前のフレームが合成後を置いたスロットを土台にする
#[test]
fn a_partial_frame_chains_through_a_reference_slot() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_blocks(config(color_type)), color_type);
        let headers = &decoded.headers;
        assert_eq!(headers.len(), BLOCK_DURATIONS.len(), "{color_type:?}");

        for (index, header) in headers.iter().enumerate() {
            let at = format!("{color_type:?} の {} 枚目", index + 1);
            assert_eq!(header.blending_info.mode, BlendingMode::Replace, "{at}");

            // 全面REPLACEでは土台の欄が、最終フレームでは置き先の欄が書かれない
            if index > 0 {
                assert_eq!(
                    header.blending_info.source,
                    headers[index - 1].save_as_reference,
                    "{at} の土台が、直前のフレームの置き先と違う"
                );
                assert_ne!(header.blending_info.source, 0, "{at} の土台が空のスロット");
                if !header.is_last {
                    assert_ne!(
                        header.save_as_reference, header.blending_info.source,
                        "{at} が土台にした枠を上書きしている"
                    );
                }
            }

            if color_type == ColorType::Rgba8 {
                assert_eq!(header.ec_blending_info.len(), 1, "{at}");
                assert_eq!(
                    header.ec_blending_info[0].mode,
                    BlendingMode::Replace,
                    "{at} のαの重ね方が色と違う"
                );
                assert_eq!(
                    header.ec_blending_info[0].source, header.blending_info.source,
                    "{at} のαの土台が色と違う"
                );
            }
        }
    }
}

/// 色が動く設定でも、部分フレームの矩形の内と外でαが入力のまま残る
#[test]
fn a_lossy_partial_frame_keeps_the_alpha_exact() {
    let color_type = ColorType::Rgba8;
    let config = Config {
        quality: 40.0,
        ..config(color_type)
    };
    let frames = block_frames(color_type);
    let decoded = decode(&encode_blocks(config), color_type);

    assert_eq!(decoded.pixels.len(), frames.len());
    for (index, (pixels, source)) in decoded.pixels.iter().zip(&frames).enumerate() {
        assert_eq!(
            alpha_channel(pixels),
            alpha_channel(source),
            "{} 枚目のαが動いている",
            index + 1
        );
        assert!(
            color_channels(pixels) != color_channels(source),
            "{} 枚目の色が入力と一致していて、非可逆になっていない",
            index + 1
        );
    }
}

/// 各フレームのtick数
fn ticks(decoded: &Decoded) -> Vec<u32> {
    decoded
        .frames
        .iter()
        .map(|frame| frame.duration_ticks)
        .collect()
}

/// 差分の無いフレームは、書き出しを待っているフレームの表示時間へ畳まれる
///
/// 畳みは列の中間でも末尾でも起きる。
#[test]
fn identical_frames_fold_into_the_pending_duration() {
    let color_type = ColorType::Rgba8;
    let frames = block_frames(color_type);
    let input = [
        frames[0].clone(),
        frames[1].clone(),
        frames[1].clone(),
        frames[2].clone(),
        frames[2].clone(),
    ];
    let durations = [3, 5, 7, 11, 13];

    let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &input, &durations);
    let decoded = decode(&encoded, color_type);

    assert!(
        decoded.frames.len() < input.len(),
        "{} 枚が畳まれずに残っている",
        decoded.frames.len()
    );
    assert_eq!(ticks(&decoded), [3, 12, 24]);
    assert_eq!(
        ticks(&decoded).iter().sum::<u32>(),
        durations.iter().sum::<u32>(),
        "1周の総表示時間が動いている"
    );
    assert_eq!(rects(&decoded.headers), [WHOLE, BLOCKS[0], BLOCKS[1]]);
    assert_eq!(
        decoded.pixels,
        [frames[0].clone(), frames[1].clone(), frames[2].clone()]
    );
}

/// 変化のあと1枚だけ書き加える点 (x, y, 幅, 高さ)
///
/// 割れた表示フレームを最終フレームから離し、置き先の欄が書かれる位置へ置く。
const TRAILING_DOT: (u32, u32, u32, u32) = (1, 1, 1, 1);

/// 離れた2箇所の変化 (x, y, 幅, 高さ)
///
/// 外接矩形は 38x24 で、割ると 840 画素を書かずに済む。
const DISTANT: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (34, 20, 6, 6)];

/// 近い2箇所の変化 (x, y, 幅, 高さ)
///
/// 外接矩形は 14x20 で、割っても 40 画素しか減らない。
const NEARBY: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 20), (10, 2, 6, 20)];

/// `NEARBY` の外接矩形
const NEARBY_BOUNDS: (u32, u32, u32, u32) = (2, 2, 14, 20);

/// `scattered_frames` の各フレームの表示時間
const SCATTERED_DURATIONS: [u32; 3] = [3, 5, 7];

/// 勾配、`blocks` を反転したもの、さらに `TRAILING_DOT` を反転したものの3枚
fn scattered_frames(color_type: ColorType, blocks: &[(u32, u32, u32, u32)]) -> Vec<Vec<u8>> {
    let base = frame(color_type, 0);
    let mut scattered = base.clone();
    for block in blocks {
        invert_block(&mut scattered, color_type, *block);
    }
    let mut tail = scattered.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base, scattered, tail]
}

/// `scattered_frames` を符号化する
fn encode_scattered(config: Config, blocks: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    encode_frames(
        config,
        WIDTH,
        HEIGHT,
        &scattered_frames(config.color_type, blocks),
        &SCATTERED_DURATIONS,
    )
}

/// 離れた2箇所の変化は矩形へ割られ、近い2箇所は外接矩形のまま書かれる
#[test]
fn a_distant_pair_of_changes_is_cut_apart() {
    let color_type = ColorType::Rgb8;

    let distant = decode(&encode_scattered(config(color_type), &DISTANT), color_type);
    assert_eq!(
        rects(&distant.headers),
        [WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT]
    );

    let nearby = decode(&encode_scattered(config(color_type), &NEARBY), color_type);
    assert_eq!(rects(&nearby.headers), [WHOLE, NEARBY_BOUNDS, TRAILING_DOT]);
}

/// 割った表示フレームの合成結果が、投入したフレームと一致する
#[test]
fn a_cut_display_frame_round_trips_byte_for_byte() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = scattered_frames(color_type, &DISTANT);
        let decoded = decode(&encode_scattered(config(color_type), &DISTANT), color_type);

        assert_eq!(
            decoded.headers.len(),
            frames.len() + 1,
            "{color_type:?} 副フレームが書かれていない"
        );
        assert_eq!(
            decoded.pixels.len(),
            frames.len(),
            "{color_type:?} 副フレームが表示フレームとして数えられている"
        );
        for (index, (pixels, source)) in decoded.pixels.iter().zip(&frames).enumerate() {
            assert_eq!(pixels, source, "{color_type:?} の {} 枚目", index + 1);
        }
    }
}

/// 割っても投入したフレーム数と表示時間の並びは動かない
#[test]
fn cutting_a_frame_keeps_the_visible_frames_and_their_ticks() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_scattered(config(color_type), &DISTANT), color_type);

        assert_eq!(
            decoded.frames.len(),
            SCATTERED_DURATIONS.len(),
            "{color_type:?}"
        );
        assert_eq!(ticks(&decoded), SCATTERED_DURATIONS, "{color_type:?}");
        assert_eq!(
            ticks(&decoded).iter().sum::<u32>(),
            SCATTERED_DURATIONS.iter().sum::<u32>(),
            "{color_type:?} の1周の総表示時間が動いている"
        );
    }
}

/// 副フレームは表示時間を持たず、表示フレームと同じスロットへ連なる
///
/// 副フレームを重ねた結果は置き先の枠に入るので、表示フレームはそちらを土台にする。
#[test]
fn a_sub_frame_chains_through_the_slot_of_its_display_frame() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let decoded = decode(&encode_scattered(config(color_type), &DISTANT), color_type);
        let headers = &decoded.headers;
        assert_eq!(headers.len(), 4, "{color_type:?}");

        let (sub, display) = (&headers[1], &headers[2]);
        assert_eq!(sub.duration, 0, "{color_type:?} の副フレームが表示される");
        assert!(!sub.is_last, "{color_type:?} の副フレームで閉じている");
        assert_eq!(
            display.duration, SCATTERED_DURATIONS[1],
            "{color_type:?} の表示フレームの表示時間"
        );
        assert_ne!(
            sub.save_as_reference, 0,
            "{color_type:?} の副フレームが枠0へ入る"
        );
        assert_eq!(
            sub.save_as_reference, display.save_as_reference,
            "{color_type:?} の副フレームが表示フレームと違う枠へ置く"
        );
        assert_eq!(
            sub.blending_info.source, headers[0].save_as_reference,
            "{color_type:?} の副フレームの土台が、直前のフレームの置き先と違う"
        );
        assert_eq!(
            display.blending_info.source, sub.save_as_reference,
            "{color_type:?} の表示フレームが副フレームの置き先を土台にしていない"
        );
        assert_eq!(
            sub.blending_info.mode,
            BlendingMode::Replace,
            "{color_type:?} の副フレームの重ね方"
        );
    }
}

/// 一過性の重なり (x, y, 幅, 高さ)
///
/// 消えたあとのフレームは、重なる前のキャンバスを土台にすれば書き直す画素が無くなる。
const POPUP: (u32, u32, u32, u32) = (10, 6, 24, 18);

/// 差分が空になったフレームが書き直す矩形 (x, y, 幅, 高さ)
const UNCHANGED: (u32, u32, u32, u32) = (0, 0, 1, 1);

/// 勾配、`POPUP` を反転したもの、勾配へ戻したもの、`TRAILING_DOT` を反転したものの4枚
fn popup_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let base = frame(color_type, 0);
    let mut covered = base.clone();
    invert_block(&mut covered, color_type, POPUP);
    let mut tail = base.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base.clone(), covered, base, tail]
}

/// `popup_frames` を符号化する
fn encode_popup(config: Config) -> Vec<u8> {
    encode_frames(
        config,
        WIDTH,
        HEIGHT,
        &popup_frames(config.color_type),
        &BLOCK_DURATIONS,
    )
}

/// 各フレームが合成後のキャンバスを置く参照スロット
fn saved_slots(headers: &[FrameHeader]) -> Vec<u32> {
    headers
        .iter()
        .map(|header| header.save_as_reference)
        .collect()
}

/// 一過性の重なりが消えたフレームは、2つ前のキャンバスを土台にする
#[test]
fn a_frame_that_undoes_a_transient_overlay_restores_the_older_canvas() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = popup_frames(color_type);
        let decoded = decode(&encode_popup(config(color_type)), color_type);
        let headers = &decoded.headers;

        assert_eq!(
            rects(headers),
            [WHOLE, POPUP, UNCHANGED, TRAILING_DOT],
            "{color_type:?}"
        );
        assert_eq!(
            headers[2].blending_info.source, headers[2].save_as_reference,
            "{color_type:?} の3枚目が2つ前のキャンバスを土台にしていない"
        );
        assert_eq!(
            headers[3].blending_info.source, headers[2].save_as_reference,
            "{color_type:?} の4枚目の土台が、直前のフレームの置き先と違う"
        );
        assert_eq!(decoded.pixels, frames, "{color_type:?}");
    }
}

/// 合成後のキャンバスを置く枠は、表示フレームごとに入れ替わる
///
/// 入れ替えることで、直前のキャンバスと2つ前のキャンバスが同時に生きる。
#[test]
fn the_reference_slot_alternates_between_display_frames() {
    let color_type = ColorType::Rgb8;
    let decoded = decode(&encode_popup(config(color_type)), color_type);

    // 最終フレームは置き先の欄を持たない
    assert_eq!(saved_slots(&decoded.headers), [1, 2, 1, 0]);
}

/// 空の枠と同じ透明な黒のキャンバスに、`block` だけを反転して置いたフレーム
fn lone_block(color_type: ColorType, block: (u32, u32, u32, u32)) -> Vec<u8> {
    let mut frame = vec![0u8; (WIDTH * HEIGHT) as usize * color_type.bytes_per_pixel()];
    invert_block(&mut frame, color_type, block);
    frame
}

/// 空の枠を土台にすれば1画素で書けるフレーム列
///
/// 空の枠は透明な黒として読まれるので、2枚目は自分の置き先を土台にすると
/// 書き直す画素がほとんど無くなる。
fn lone_block_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let second = lone_block(color_type, LONE_BLOCKS[1]);
    let mut tail = second.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![lone_block(color_type, LONE_BLOCKS[0]), second, tail]
}

/// 透明な黒の上を動く塊 (x, y, 幅, 高さ)
const LONE_BLOCKS: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (10, 2, 6, 6)];

/// `LONE_BLOCKS` の外接矩形
const LONE_BOUNDS: (u32, u32, u32, u32) = (2, 2, 14, 6);

/// 2枚目のフレームは、まだ書き込まれていない枠を土台にしない
#[test]
fn the_second_frame_leaves_the_unwritten_slot_alone() {
    let color_type = ColorType::Rgb8;
    let frames = lone_block_frames(color_type);
    let encoded = encode_frames(
        config(color_type),
        WIDTH,
        HEIGHT,
        &frames,
        &SCATTERED_DURATIONS,
    );
    let decoded = decode(&encoded, color_type);
    let headers = &decoded.headers;

    assert_eq!(
        rects(headers),
        [WHOLE, LONE_BOUNDS, TRAILING_DOT],
        "2枚目が空の枠との差分で書かれている"
    );
    assert_eq!(
        headers[1].blending_info.source, headers[0].save_as_reference,
        "2枚目の土台が、直前のフレームの置き先と違う"
    );
    assert_ne!(
        headers[1].blending_info.source, headers[1].save_as_reference,
        "2枚目が空の自分の置き先を土台にしている"
    );
    assert_eq!(decoded.pixels, frames);
}

/// 勾配、全面を反転したもの、勾配へ `DISTANT` を書き加えたもの、`TRAILING_DOT` を
/// さらに反転したものの4枚
///
/// 3枚目は直前のフレームとは全面で食い違い、2つ前のキャンバスとは離れた2箇所しか
/// 違わない。
fn flash_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let base = frame(color_type, 0);
    let mut flashed = base.clone();
    invert_block(&mut flashed, color_type, WHOLE);
    let mut scattered = base.clone();
    for block in DISTANT {
        invert_block(&mut scattered, color_type, block);
    }
    let mut tail = scattered.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base, flashed, scattered, tail]
}

/// 2つ前のキャンバスを土台にした表示フレームも、離れた変化なら矩形へ割られる
#[test]
fn a_restored_display_frame_is_cut_against_the_older_canvas() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let frames = flash_frames(color_type);
        let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &frames, &BLOCK_DURATIONS);
        let decoded = decode(&encoded, color_type);
        let headers = &decoded.headers;

        assert_eq!(
            rects(headers),
            [WHOLE, WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT],
            "{color_type:?}"
        );
        assert_eq!(saved_slots(headers), [1, 2, 1, 1, 0], "{color_type:?}");

        let (sub, display) = (&headers[2], &headers[3]);
        assert_eq!(sub.duration, 0, "{color_type:?} の副フレームが表示される");
        assert_eq!(
            sub.blending_info.source, sub.save_as_reference,
            "{color_type:?} の副フレームが2つ前のキャンバスを土台にしていない"
        );
        assert_eq!(
            display.blending_info.source, sub.save_as_reference,
            "{color_type:?} の表示フレームが副フレームの置き先を土台にしていない"
        );
        assert_eq!(decoded.pixels, frames, "{color_type:?}");
    }
}

/// `u32` に収まらない表示時間は、そこでフレームを分けて持つ
#[test]
fn a_duration_beyond_the_writable_range_splits_the_frame() {
    let color_type = ColorType::Rgb8;
    let source = frame(color_type, 0);
    let durations = [u32::MAX, 5];

    let input = [source.clone(), source];
    let encoded = encode_frames(config(color_type), WIDTH, HEIGHT, &input, &durations);
    let decoded = decode(&encoded, color_type);

    assert_eq!(ticks(&decoded), durations);
    assert_eq!(decoded.pixels, input);
}

/// 層の取り出しにかけるフレーム列
struct Sequence {
    name: &'static str,
    frames: Vec<Vec<u8>>,
    durations: &'static [u32],
    /// 書かれる矩形 (x, y, 幅, 高さ)
    rects: Vec<(u32, u32, u32, u32)>,
}

/// 全面・矩形1枚・矩形2枚の3つの書き方を、直前と2つ前の2通りの土台で踏む列
fn sequences(color_type: ColorType) -> Vec<Sequence> {
    vec![
        // 2枚目は直前を、3枚目は2つ前を土台にした矩形1枚
        Sequence {
            name: "popup",
            frames: popup_frames(color_type),
            durations: &BLOCK_DURATIONS,
            rects: vec![WHOLE, POPUP, UNCHANGED, TRAILING_DOT],
        },
        // 2枚目は直前を土台に矩形2枚へ割れる
        Sequence {
            name: "scattered",
            frames: scattered_frames(color_type, &DISTANT),
            durations: &SCATTERED_DURATIONS,
            rects: vec![WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT],
        },
        // 3枚目は2つ前を土台に矩形2枚へ割れる
        Sequence {
            name: "flash",
            frames: flash_frames(color_type),
            durations: &BLOCK_DURATIONS,
            rects: vec![WHOLE, WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT],
        },
    ]
}

/// `sequence` のフレームを符号化する
fn encode_sequence(config: Config, sequence: &Sequence) -> Vec<u8> {
    encode_frames(config, WIDTH, HEIGHT, &sequence.frames, sequence.durations)
}

/// 矩形を `Layers` へ渡す形へ写す
fn rect_of((x, y, width, height): (u32, u32, u32, u32)) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// `rect` の範囲を連続したバイト列として切り出す
fn crop(frame: &[u8], color_type: ColorType, rect: Rect) -> Vec<u8> {
    let bytes_per_pixel = color_type.bytes_per_pixel();
    let row_len = rect.width as usize * bytes_per_pixel;
    let mut out = Vec::with_capacity(row_len * rect.height as usize);
    for row in 0..rect.height as usize {
        let start = ((rect.y as usize + row) * WIDTH as usize + rect.x as usize) * bytes_per_pixel;
        out.extend_from_slice(&frame[start..start + row_len]);
    }
    out
}

/// 連続したバイト列を `rect` の範囲へ書き込む
fn paste(canvas: &mut [u8], color_type: ColorType, rect: Rect, layer: &[u8]) {
    let bytes_per_pixel = color_type.bytes_per_pixel();
    let row_len = rect.width as usize * bytes_per_pixel;
    for row in 0..rect.height as usize {
        let start = ((rect.y as usize + row) * WIDTH as usize + rect.x as usize) * bytes_per_pixel;
        canvas[start..start + row_len].copy_from_slice(&layer[row * row_len..(row + 1) * row_len]);
    }
}

/// 各層を切り出した投入フレームの番号
///
/// 副フレームは、続く表示フレームと同じ投入フレームから切り出される。
fn sources(headers: &[FrameHeader]) -> Vec<usize> {
    let mut shown = 0;
    headers
        .iter()
        .map(|header| {
            let at = shown;
            if header.duration != 0 || header.is_last {
                shown += 1;
            }
            at
        })
        .collect()
}

/// 矩形を書いた順に控えながら `parts` を流し込み、返った対を集める
fn peel<'a>(
    color_type: ColorType,
    parts: impl IntoIterator<Item = &'a [u8]>,
    written: &[(u32, u32, u32, u32)],
) -> Vec<(Rect, Vec<u8>)> {
    let mut layers = Layers::new(color_type, 2).unwrap();
    for rect in written {
        layers.wrote(rect_of(*rect));
    }

    let mut peeled = Vec::new();
    for part in parts {
        layers
            .feed(part, |rect, layer| peeled.push((rect, layer.to_vec())))
            .unwrap();
    }
    peeled
}

/// 書き出しの1回ぶんずつバイト列を控える writer
#[derive(Default)]
struct Chunks(Vec<Vec<u8>>);

impl Write for Chunks {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.push(buf.to_vec());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
