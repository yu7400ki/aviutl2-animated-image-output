//! 単葉の .webp を `image-webp` でデコードして入力と突き合わせる

use image_webp::WebPDecoder;
use std::io::Cursor;
use webp_encoder::{ColorType, Config, Error};

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
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
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
    rgb.chunks_exact(3)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect()
}

/// `image-webp` でデコードした生RGBA
fn decode(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).expect("image-webp が読めない");
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
    }
}

#[test]
fn a_lossless_rgba_still_decodes_back_to_the_input() {
    let (width, height) = (61, 37);
    let rgba = normalized_rgba(width, height, 0x5EED);

    let frame =
        webp_encoder::encode(width, height, &rgba, &config(ColorType::Rgba8, true)).unwrap();

    assert_eq!(decode(frame.still(), width, height), rgba);
}

#[test]
fn a_lossless_rgb_still_decodes_back_to_the_input() {
    let (width, height) = (48, 32);
    let rgb = gradient_rgb(width, height);

    let frame = webp_encoder::encode(width, height, &rgb, &config(ColorType::Rgb8, true)).unwrap();

    assert_eq!(decode(frame.still(), width, height), opaque_rgba(&rgb));
}

/// 完全透過の画素のRGBは、正規化を通していない素材でも保たれる
#[test]
fn a_lossless_still_keeps_the_color_under_transparent_pixels() {
    let (width, height) = (16, 16);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let transparent = x < 8 && y < 8;
            rgba.extend_from_slice(&[
                (x * 16) as u8,
                (y * 16) as u8,
                0x5A,
                if transparent { 0 } else { 255 },
            ]);
        }
    }

    let frame =
        webp_encoder::encode(width, height, &rgba, &config(ColorType::Rgba8, true)).unwrap();

    assert_eq!(decode(frame.still(), width, height), rgba);
}

#[test]
fn a_lossy_still_decodes_near_the_input() {
    let (width, height) = (64, 64);
    let rgb = gradient_rgb(width, height);

    let frame = webp_encoder::encode(width, height, &rgb, &config(ColorType::Rgb8, false)).unwrap();

    let decoded = decode(frame.still(), width, height);
    assert!(mean_abs_error(&decoded, &opaque_rgba(&rgb)) < 4.0);
}

#[test]
fn the_still_carries_the_same_bitstream_as_the_frame_payload() {
    let (width, height) = (24, 18);
    let rgba = normalized_rgba(width, height, 0xF00D);

    let frame =
        webp_encoder::encode(width, height, &rgba, &config(ColorType::Rgba8, true)).unwrap();

    assert!(frame.still().ends_with(frame.image()));
    assert_eq!(frame.still().len(), 12 + frame.image().len());
}

#[test]
fn a_canvas_outside_the_limits_is_refused() {
    for (width, height) in [(0, 8), (8, 0), (16384, 8), (8, 16384)] {
        let data = vec![0u8; 0];
        assert!(matches!(
            webp_encoder::encode(width, height, &data, &config(ColorType::Rgba8, true)),
            Err(Error::InvalidDimensions { .. })
        ));
    }
}

#[test]
fn a_frame_of_another_length_is_refused() {
    let data = vec![0u8; 8 * 8 * 4 - 1];
    assert!(matches!(
        webp_encoder::encode(8, 8, &data, &config(ColorType::Rgba8, true)),
        Err(Error::FrameSizeMismatch {
            expected: 256,
            actual: 255
        })
    ));
}
