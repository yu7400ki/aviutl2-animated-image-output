//! FFI 経由で符号化した1枚を、`image-webp` と ffmpeg の2つでデコードして
//! 突き合わせる
//!
//! ffmpeg が見つからない環境では、そちらのデコードだけを飛ばす。

use image_webp::WebPDecoder;
use std::ffi::{c_int, c_void};
use std::io::{Cursor, Write};
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use webp_sys::*;

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

/// 完全透過の画素を `0x00000000` へ潰した RGBA
fn normalized_rgba(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut rgba = noise((width * height * 4) as usize, seed);
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        pixel[3] = if index % 7 == 0 { 0 } else { 255 };
        if pixel[3] == 0 {
            pixel.fill(0);
        }
    }
    rgba
}

/// 横方向と縦方向で滑らかに変わる不透明な RGB
fn gradient_rgb(width: u32, height: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            rgb.push((x * 255 / width.max(1)) as u8);
            rgb.push((y * 255 / height.max(1)) as u8);
            rgb.push(((x + y) * 255 / (width + height).max(1)) as u8);
        }
    }
    rgb
}

/// 符号化の設定
struct Config {
    lossless: bool,
    quality: f32,
}

/// 取り込む画素の並び
enum Input<'a> {
    Rgba(&'a [u8]),
    Rgb(&'a [u8]),
}

/// 単葉の .webp を組む
fn encode(width: u32, height: u32, input: Input<'_>, config: Config) -> Vec<u8> {
    unsafe {
        let mut raw = MaybeUninit::<WebPConfig>::uninit();
        assert_ne!(WebPConfigInit(raw.as_mut_ptr()), 0);
        let mut raw = raw.assume_init();
        raw.lossless = c_int::from(config.lossless);
        raw.exact = c_int::from(config.lossless);
        raw.quality = config.quality;
        raw.method = 4;
        assert_ne!(WebPValidateConfig(&raw), 0, "設定が値域に収まらない");

        let mut picture = MaybeUninit::<WebPPicture>::uninit();
        assert_ne!(WebPPictureInitARGB(picture.as_mut_ptr()), 0);
        let mut picture = picture.assume_init();
        picture.width = width as c_int;
        picture.height = height as c_int;

        let imported = match input {
            Input::Rgba(rgba) => {
                WebPPictureImportRGBA(&mut picture, rgba.as_ptr(), (width * 4) as c_int)
            }
            Input::Rgb(rgb) => {
                WebPPictureImportRGB(&mut picture, rgb.as_ptr(), (width * 3) as c_int)
            }
        };
        assert_ne!(imported, 0, "画素の取り込みに失敗した");

        let mut writer = MaybeUninit::<WebPMemoryWriter>::uninit();
        WebPMemoryWriterInit(writer.as_mut_ptr());
        let mut writer = writer.assume_init();
        picture.writer = Some(WebPMemoryWrite);
        picture.custom_ptr = (&raw mut writer).cast::<c_void>();

        let encoded = WebPEncode(&raw, &mut picture);
        let error_code = picture.error_code;
        WebPPictureFree(&mut picture);
        assert_ne!(encoded, 0, "符号化に失敗した: error_code = {error_code}");

        let bytes = std::slice::from_raw_parts(writer.mem, writer.size).to_vec();
        WebPMemoryWriterClear(&mut writer);
        bytes
    }
}

/// `image-webp` でデコードした生 RGBA
fn decode_with_image_webp(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
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

/// RGB を α = 255 の RGBA へ広げる
fn opaque_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect()
}

/// 一時ファイルへ書き出す。ffmpeg は標準入力の webp を受け取れない
fn temp_webp(bytes: &[u8]) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "webp-sys-smoke-{}-{}.webp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    path
}

/// ffmpeg でデコードした生 RGBA。ffmpeg が無ければ `None`
fn decode_with_ffmpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let path = temp_webp(bytes);

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
            assert!(
                std::env::var_os("CI").is_none(),
                "ffmpeg が見つからない。デコーダが1つでは、一部のデコーダだけが読める出力を見つけられない"
            );
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
    Some(output.stdout)
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

#[test]
fn a_lossless_rgba_frame_survives_both_decoders() {
    let (width, height) = (61, 37);
    let rgba = normalized_rgba(width, height, 0x5EED);
    let bytes = encode(
        width,
        height,
        Input::Rgba(&rgba),
        Config {
            lossless: true,
            quality: 100.0,
        },
    );

    assert_eq!(decode_with_image_webp(&bytes, width, height), rgba);
    if let Some(decoded) = decode_with_ffmpeg(&bytes) {
        assert_eq!(decoded, rgba, "ffmpeg のデコードが入力と違う");
    }
}

#[test]
fn a_lossless_rgb_frame_survives_both_decoders() {
    let (width, height) = (48, 32);
    let rgb = gradient_rgb(width, height);
    let expected = opaque_rgba(&rgb);
    let bytes = encode(
        width,
        height,
        Input::Rgb(&rgb),
        Config {
            lossless: true,
            quality: 100.0,
        },
    );

    assert_eq!(decode_with_image_webp(&bytes, width, height), expected);
    if let Some(decoded) = decode_with_ffmpeg(&bytes) {
        assert_eq!(decoded, expected, "ffmpeg のデコードが入力と違う");
    }
}

#[test]
fn a_lossy_frame_decodes_near_the_input_in_both_decoders() {
    let (width, height) = (64, 64);
    let rgb = gradient_rgb(width, height);
    let expected = opaque_rgba(&rgb);
    let bytes = encode(
        width,
        height,
        Input::Rgb(&rgb),
        Config {
            lossless: false,
            quality: 90.0,
        },
    );

    // 非可逆は YUV の上げ方がデコーダごとに違うため、突き合わせは入力との距離で行う
    let decoded = decode_with_image_webp(&bytes, width, height);
    assert!(
        mean_abs_error(&decoded, &expected) < 4.0,
        "image-webp のデコードが入力から離れている"
    );
    if let Some(decoded) = decode_with_ffmpeg(&bytes) {
        assert!(
            mean_abs_error(&decoded, &expected) < 4.0,
            "ffmpeg のデコードが入力から離れている"
        );
    }
}
