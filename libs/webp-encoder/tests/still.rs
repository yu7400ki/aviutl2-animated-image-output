//! 単葉の .webp を `image-webp` でデコードして入力と突き合わせる

use image_webp::WebPDecoder;
use std::io::Cursor;
use webp_encoder::InputError;
use webp_encoder::{ColorType, Config, Encoder, Error, FrameDelay, Report};

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

/// 完全透過の画素を `0x00000000` へ潰したRGBA
fn normalized_rgba(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut rgba = noise((width * height * 4) as usize, seed);
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        pixel[3] = if index % 5 == 0 { 0 } else { 255 };
        if pixel[3] == 0 {
            pixel.fill(0);
        }
    }
    rgba
}

/// 横方向と縦方向で滑らかに変わる不透明なRGB
fn gradient_rgb(width: u32, height: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            rgb.push((x * 255 / width) as u8);
            rgb.push((y * 255 / height) as u8);
            rgb.push(((x + y) * 255 / (width + height)) as u8);
        }
    }
    rgb
}

/// RGBをα = 255 のRGBAへ広げる
fn opaque_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect()
}

/// `image-webp` でデコードした生RGBA
fn decode(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).expect("image-webp が読めない");
    assert!(!decoder.is_animated(), "単葉がアニメーションになっている");
    assert_eq!(decoder.dimensions(), (width, height));
    let mut buffer = vec![0u8; decoder.output_buffer_size().expect("出力の大きさ")];
    decoder
        .read_image(&mut buffer)
        .expect("image-webp のデコード");
    if decoder.has_alpha() {
        buffer
    } else {
        opaque_rgba(&buffer)
    }
}

/// 画素ごとの差の平均
fn mean_abs_error(actual: &[u8], expected: &[u8]) -> f64 {
    assert_eq!(actual.len(), expected.len());
    let total: u64 = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    total as f64 / actual.len() as f64
}

fn config(color_type: ColorType, lossless: bool) -> Config {
    Config {
        color_type,
        lossless,
        quality: if lossless { 100.0 } else { 90.0 },
        method: 4,
        num_plays: 0,
    }
}

/// 1フレームだけを投入する
fn encode(
    width: u32,
    height: u32,
    data: &[u8],
    config: Config,
) -> Result<(Vec<u8>, Report), Error> {
    let mut encoder = Encoder::new(Cursor::new(Vec::new()), width, height, 1, config)?;
    encoder.add_frame(data.to_vec(), FrameDelay::new(1, 30).unwrap())?;
    let (writer, report) = encoder.finish()?;
    Ok((writer.into_inner(), report))
}

#[test]
fn a_lossless_rgba_still_decodes_back_to_the_input() {
    let (width, height) = (61, 37);
    let rgba = normalized_rgba(width, height, 0x5EED);

    let (bytes, report) = encode(width, height, &rgba, config(ColorType::Rgba8, true)).unwrap();

    assert_eq!(decode(&bytes, width, height), rgba);
    assert_eq!(
        report,
        Report {
            merged_frames: 0,
            delay_clamped: false,
        }
    );
}

#[test]
fn a_lossless_rgb_still_decodes_back_to_the_input() {
    let (width, height) = (48, 32);
    let rgb = gradient_rgb(width, height);

    let (bytes, _) = encode(width, height, &rgb, config(ColorType::Rgb8, true)).unwrap();

    assert_eq!(decode(&bytes, width, height), opaque_rgba(&rgb));
}

/// 完全透過の画素は正規化で `0x00000000` になり、半透明はそのまま戻る
#[test]
fn a_lossless_still_collapses_the_color_under_transparent_pixels() {
    let (width, height) = (16, 16);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let alpha = match x / 6 {
                0 => 0,
                1 => 128,
                _ => 255,
            };
            rgba.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 0x5A, alpha]);
        }
    }

    let (bytes, _) = encode(width, height, &rgba, config(ColorType::Rgba8, true)).unwrap();

    let expected: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            if pixel[3] == 0 {
                [0, 0, 0, 0]
            } else {
                [pixel[0], pixel[1], pixel[2], pixel[3]]
            }
        })
        .collect();
    assert_ne!(expected, rgba, "正規化で変わる画素を含んでいない");
    assert_eq!(decode(&bytes, width, height), expected);
}

#[test]
fn a_lossy_still_decodes_near_the_input() {
    let (width, height) = (64, 64);
    let rgb = gradient_rgb(width, height);

    let (bytes, _) = encode(width, height, &rgb, config(ColorType::Rgb8, false)).unwrap();

    let decoded = decode(&bytes, width, height);
    assert!(mean_abs_error(&decoded, &opaque_rgba(&rgb)) < 4.0);
}

/// 1フレームの素材は `WebPEncode` の出力そのもので、アニメーションの
/// チャンクを1つも伴わない
#[test]
fn a_single_frame_is_written_as_a_simple_file() {
    let (width, height) = (24, 18);
    let rgba = normalized_rgba(width, height, 0xF00D);

    let (bytes, _) = encode(width, height, &rgba, config(ColorType::Rgba8, true)).unwrap();

    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    assert_eq!(&bytes[12..16], b"VP8L");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
        bytes.len() - 8
    );
}

#[test]
fn a_canvas_outside_the_limits_is_refused() {
    for (width, height) in [(0, 8), (8, 0), (16384, 8), (8, 16384)] {
        assert!(matches!(
            encode(width, height, &[], config(ColorType::Rgba8, true)),
            Err(Error::Input(InputError::InvalidDimensions { .. }))
        ));
    }
}

#[test]
fn a_frame_of_another_length_is_refused() {
    let data = vec![0u8; 8 * 8 * 4 - 1];
    assert!(matches!(
        encode(8, 8, &data, config(ColorType::Rgba8, true)),
        Err(Error::Input(InputError::FrameSizeMismatch {
            expected: 256,
            actual: 255
        }))
    ));
}
