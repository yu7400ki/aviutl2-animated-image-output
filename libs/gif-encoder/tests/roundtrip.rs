//! 出力したGIFをデコードし、正規化した入力と画素単位で一致することを確認する
//!
//! LZWの誤りは一部のデコーダだけが読めるファイルを作るため、`gif` クレートと
//! ffmpeg の2つでデコードする。ffmpeg が見つからない環境では、そちらだけを
//! 飛ばして `gif` クレートの結果で判定する。

use gif_encoder::{ColorType, Config, Encoder, Error, FrameDelay};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// 決定的な擬似乱数列
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 16) as u8
        })
        .collect()
}

fn delay() -> FrameDelay {
    FrameDelay::new(1001, 30000).unwrap()
}

fn encode(width: u32, height: u32, color_type: ColorType, data: &[u8]) -> Vec<u8> {
    encode_with_plays(width, height, color_type, data, 0)
}

fn encode_with_plays(
    width: u32,
    height: u32,
    color_type: ColorType,
    data: &[u8],
    num_plays: u32,
) -> Vec<u8> {
    let config = Config {
        color_type,
        num_plays,
    };
    let mut encoder = Encoder::new(Vec::new(), width, height, 1, config).unwrap();
    encoder.add_frame(data, delay()).unwrap();
    encoder.finish().unwrap()
}

/// 入力を正規化し、RGBA8へ展開する
///
/// RGBA8の入力はアルファが128未満の画素を完全透過へ潰し、残りを不透明へ上げる。
fn expected_rgba(data: &[u8], color_type: ColorType) -> Vec<u8> {
    match color_type {
        ColorType::Rgb8 => data
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        ColorType::Rgba8 => data
            .chunks_exact(4)
            .flat_map(|pixel| {
                if pixel[3] < 128 {
                    [0, 0, 0, 0]
                } else {
                    [pixel[0], pixel[1], pixel[2], 255]
                }
            })
            .collect(),
    }
}

/// `gif` クレートで読み出した結果
struct Decoded {
    width: u16,
    height: u16,
    repeat: gif::Repeat,
    frames: Vec<DecodedFrame>,
}

struct DecodedFrame {
    rgba: Vec<u8>,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    delay: u16,
}

fn decode_with_gif(bytes: &[u8]) -> Decoded {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(bytes).unwrap();

    let width = decoder.width();
    let height = decoder.height();
    let repeat = decoder.repeat();

    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        frames.push(DecodedFrame {
            rgba: frame.buffer.to_vec(),
            left: frame.left,
            top: frame.top,
            width: frame.width,
            height: frame.height,
            delay: frame.delay,
        });
    }

    Decoded {
        width,
        height,
        repeat,
        frames,
    }
}

/// ffmpeg でデコードした生RGBA。ffmpeg が無ければ `None`
fn decode_with_ffmpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "gif-encoder-roundtrip-{}-{}.gif",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();

    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output();
    std::fs::remove_file(&path).unwrap();

    let output = match output {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("ffmpeg が見つからないため、そちらのデコードを飛ばす");
            return None;
        }
        Err(e) => panic!("ffmpeg の起動に失敗した: {e}"),
    };
    assert!(
        output.status.success(),
        "ffmpeg のデコードに失敗した: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut rgba = output.stdout;
    restore_transparent_marker(&mut rgba);
    Some(rgba)
}

/// ffmpeg が透過画素へ置く色を、正規化した入力が持つ完全透過の標識へ戻す
///
/// ffmpeg はカラーテーブルの透過エントリを `GIF_TRANSPARENT_COLOR`
/// (アルファ0の白) へ差し替えるため、そのままでは標識と突き合わせられない。
/// 差し替え後のこの1色だけを戻すので、他の色はそのまま突き合わせに残る。
fn restore_transparent_marker(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel == [0xFF, 0xFF, 0xFF, 0x00] {
            pixel.fill(0);
        }
    }
}

/// 出力を2つのデコーダへ通し、正規化した入力と画素単位で突き合わせる
fn round_trip(width: u32, height: u32, color_type: ColorType, data: &[u8]) -> Vec<u8> {
    let bytes = encode(width, height, color_type, data);
    let expected = expected_rgba(data, color_type);

    let decoded = decode_with_gif(&bytes);
    assert_eq!(
        (u32::from(decoded.width), u32::from(decoded.height)),
        (width, height),
        "論理画面の寸法が違う"
    );
    assert_eq!(decoded.frames.len(), 1, "フレーム数が違う");
    let frame = &decoded.frames[0];
    assert_eq!((frame.left, frame.top), (0, 0), "矩形の位置が全画面でない");
    assert_eq!(
        (u32::from(frame.width), u32::from(frame.height)),
        (width, height),
        "矩形の大きさが全画面でない"
    );
    assert_eq!(frame.delay, 3, "遅延が1/100秒へ丸められていない");
    assert_eq!(frame.rgba, expected, "`gif` クレートのデコード結果が違う");

    if let Some(raw) = decode_with_ffmpeg(&bytes) {
        assert_eq!(raw, expected, "ffmpeg のデコード結果が違う");
    }

    bytes
}

/// 1画素だけの画像
#[test]
fn a_single_pixel_frame_survives_both_decoders() {
    round_trip(1, 1, ColorType::Rgb8, &[0x12, 0x34, 0x56]);
    round_trip(1, 1, ColorType::Rgba8, &[0x12, 0x34, 0x56, 0xFF]);
    round_trip(1, 1, ColorType::Rgba8, &[0x12, 0x34, 0x56, 0x00]);
}

/// 全画素が同じ色
#[test]
fn a_uniform_frame_survives_both_decoders() {
    let data: Vec<u8> = [0x20, 0x40, 0x60].repeat(64 * 64);
    round_trip(64, 64, ColorType::Rgb8, &data);
}

/// カラーテーブルが2の冪へ埋められる色数
#[test]
fn frames_with_padded_color_tables_survive_both_decoders() {
    for colors in [1usize, 2, 3, 5, 9, 17, 129] {
        let data: Vec<u8> = (0..32 * 32)
            .flat_map(|i| {
                let value = (i % colors) as u8;
                [value, value.wrapping_mul(7), value.wrapping_mul(13)]
            })
            .collect();
        round_trip(32, 32, ColorType::Rgb8, &data);
    }
}

/// 色の和集合がちょうど256色 (透過標識を含まない)
#[test]
fn a_frame_with_exactly_256_opaque_colors_survives_both_decoders() {
    let data: Vec<u8> = (0..16 * 16).flat_map(|i| [i as u8, 0, 0, 0xFF]).collect();
    let bytes = round_trip(16, 16, ColorType::Rgba8, &data);

    // 透過標識が和集合に無いため、透過インデックスは置かない
    let decoded = decode_with_gif(&bytes);
    assert!(decoded.frames[0].rgba.chunks_exact(4).all(|p| p[3] == 255));
}

/// 素材自身の透過画素が透過インデックスになる
#[test]
fn a_frame_with_transparent_pixels_survives_both_decoders() {
    let data: Vec<u8> = (0..255 * 4)
        .flat_map(|i| {
            if i % 3 == 0 {
                [9, 9, 9, 0]
            } else {
                [(i % 255) as u8, 0x80, 0x40, 0xFF]
            }
        })
        .collect();
    round_trip(51, 20, ColorType::Rgba8, &data);
}

/// 閾値未満のアルファは完全透過へ潰れる
#[test]
fn partial_alpha_is_binarized_before_encoding() {
    let data: Vec<u8> = (0..64 * 8)
        .flat_map(|i| [(i % 200) as u8, 0x10, 0x20, (i % 256) as u8])
        .collect();
    round_trip(64, 8, ColorType::Rgba8, &data);
}

/// 縦横が異なる矩形
#[test]
fn a_non_square_frame_survives_both_decoders() {
    let data = noise(17 * 5 * 3, 11)
        .iter()
        .map(|&byte| byte & 0x0F)
        .collect::<Vec<u8>>();
    round_trip(17, 5, ColorType::Rgb8, &data);
}

/// LZWの辞書が4096で埋まる長さのフレーム
///
/// 262144画素の雑音は辞書を使い切り、Clearを出して張り直す経路を通る。
#[test]
fn a_frame_that_fills_the_lzw_dictionary_survives_both_decoders() {
    let indices = noise(512 * 512, 5);
    let data: Vec<u8> = indices
        .iter()
        .flat_map(|&index| [index, index.wrapping_mul(3), index.wrapping_mul(5)])
        .collect();
    round_trip(512, 512, ColorType::Rgb8, &data);
}

/// 設定した再生回数がループ数の欄へ落ちる
#[test]
fn the_number_of_plays_reaches_the_decoder() {
    let data = [0x10, 0x20, 0x30];
    for (num_plays, repeat) in [
        (0, gif::Repeat::Infinite),
        (2, gif::Repeat::Finite(1)),
        (10, gif::Repeat::Finite(9)),
        (u32::MAX, gif::Repeat::Finite(u16::MAX)),
    ] {
        let bytes = encode_with_plays(1, 1, ColorType::Rgb8, &data, num_plays);
        assert_eq!(
            decode_with_gif(&bytes).repeat,
            repeat,
            "再生回数 {num_plays}"
        );
    }

    // 1回だけ再生するときはアプリケーション拡張を書かない
    let bytes = encode_with_plays(1, 1, ColorType::Rgb8, &data, 1);
    assert!(
        !bytes.windows(11).any(|window| window == b"NETSCAPE2.0"),
        "1回再生でアプリケーション拡張が出た"
    );
    assert_eq!(decode_with_gif(&bytes).frames.len(), 1);
}

#[test]
fn zero_and_oversized_dimensions_are_rejected() {
    let config = Config::default();
    for (width, height) in [(0, 1), (1, 0), (65536, 1), (1, 65536)] {
        assert!(
            matches!(
                Encoder::new(Vec::new(), width, height, 1, config),
                Err(Error::InvalidDimensions { .. })
            ),
            "{width}x{height}"
        );
    }
    assert!(Encoder::new(Vec::new(), 65535, 1, 1, config).is_ok());
}

#[test]
fn a_frame_count_other_than_one_is_rejected() {
    let config = Config::default();
    assert!(matches!(
        Encoder::new(Vec::new(), 1, 1, 0, config),
        Err(Error::InvalidFrameCount)
    ));
    for num_frames in [2, 3, 100] {
        assert!(
            matches!(
                Encoder::new(Vec::new(), 1, 1, num_frames, config),
                Err(Error::UnsupportedFrameCount(count)) if count == num_frames
            ),
            "{num_frames} フレーム"
        );
    }
}

#[test]
fn more_than_256_colors_are_rejected() {
    let config = Config {
        color_type: ColorType::Rgba8,
        num_plays: 0,
    };
    let data: Vec<u8> = (0..257)
        .flat_map(|i| [i as u8, (i >> 8) as u8, 0, 0xFF])
        .collect();
    let mut encoder = Encoder::new(Vec::new(), 257, 1, 1, config).unwrap();
    assert!(matches!(
        encoder.add_frame(&data, delay()),
        Err(Error::TooManyColors)
    ));
}

/// 256色の非透過色に透過画素が加わると和集合が上限を超える
#[test]
fn a_transparent_pixel_beyond_256_opaque_colors_is_rejected() {
    let config = Config {
        color_type: ColorType::Rgba8,
        num_plays: 0,
    };
    let mut data: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 0xFF]).collect();
    data.extend_from_slice(&[0, 0, 0, 0]);
    let mut encoder = Encoder::new(Vec::new(), 257, 1, 1, config).unwrap();
    assert!(matches!(
        encoder.add_frame(&data, delay()),
        Err(Error::TooManyColors)
    ));
}

#[test]
fn a_frame_of_the_wrong_length_is_rejected() {
    let config = Config::default();
    let mut encoder = Encoder::new(Vec::new(), 4, 4, 1, config).unwrap();
    assert!(matches!(
        encoder.add_frame(&[0; 47], delay()),
        Err(Error::FrameSizeMismatch {
            expected: 48,
            actual: 47
        })
    ));
}

#[test]
fn a_missing_or_extra_frame_is_rejected() {
    let config = Config::default();
    let encoder = Encoder::new(Vec::new(), 1, 1, 1, config).unwrap();
    assert!(matches!(
        encoder.finish(),
        Err(Error::FrameCountMismatch {
            expected: 1,
            actual: 0
        })
    ));

    let mut encoder = Encoder::new(Vec::new(), 1, 1, 1, config).unwrap();
    encoder.add_frame(&[1, 2, 3], delay()).unwrap();
    assert!(matches!(
        encoder.add_frame(&[1, 2, 3], delay()),
        Err(Error::FrameCountMismatch {
            expected: 1,
            actual: 2
        })
    ));
}
