//! FFI 経由で符号化した単葉とシーケンスを ffmpeg で読み直し、入力との SSIM で突き合わせる
//!
//! ffmpeg が見つからない環境では、組み立てたバイト列と符号化器の報告だけを見る。

use avif_sys::*;
use std::ffi::{CStr, c_int};
use std::io::Write;
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

/// 横方向と縦方向で滑らかに変わる不透明な RGB。`phase` は絵をずらす
fn gradient_rgb(width: u32, height: u32, phase: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let x = (x + phase) % width;
            rgb.push((x * 255 / width) as u8);
            rgb.push((y * 255 / height) as u8);
            rgb.push(((x + y) * 255 / (width + height)) as u8);
        }
    }
    rgb
}

/// `gradient_rgb` に、左から右へ薄れる α を足したもの
fn gradient_rgba(width: u32, height: u32, phase: u32) -> Vec<u8> {
    gradient_rgb(width, height, phase)
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .flat_map(|(index, pixel)| {
            let x = (index as u32) % width;
            [pixel[0], pixel[1], pixel[2], (x * 255 / width) as u8]
        })
        .collect()
}

/// 取り込む画素の並び
enum Input<'a> {
    Rgb(&'a [u8]),
    Rgba(&'a [u8]),
}

impl Input<'_> {
    fn channels(&self) -> u32 {
        match self {
            Input::Rgb(_) => 3,
            Input::Rgba(_) => 4,
        }
    }

    fn format(&self) -> c_int {
        match self {
            Input::Rgb(_) => AVIF_RGB_FORMAT_RGB,
            Input::Rgba(_) => AVIF_RGB_FORMAT_RGBA,
        }
    }

    fn pixels(&self) -> &[u8] {
        match self {
            Input::Rgb(pixels) | Input::Rgba(pixels) => pixels,
        }
    }
}

/// 符号化の結果と、積まれた AV1 ストリームの大きさ
struct Encoded {
    bytes: Vec<u8>,
    color_obu_size: usize,
    alpha_obu_size: usize,
}

/// 失敗した `avifResult` を、総称名と `encoder.diag` の文言で言い直す
///
/// # Safety
///
/// `encoder` は NULL か、生きている `avifEncoder` を指していること。
unsafe fn expect_ok(result: c_int, encoder: *const avifEncoder, what: &str) {
    if result == AVIF_RESULT_OK {
        return;
    }
    let name = unsafe { CStr::from_ptr(avifResultToString(result)) }.to_string_lossy();
    let detail = if encoder.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr((*encoder).diag.error.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    };
    panic!("{what} に失敗した: {name} ({detail})");
}

/// フレームを順に投入して .avif を組む
///
/// 1 枚なら単葉、2 枚以上ならシーケンスになる。
fn encode(width: u32, height: u32, frames: &[Input<'_>], quality: c_int) -> Encoded {
    let (first, rest) = frames.split_first().expect("フレームが要る");
    let flags = if rest.is_empty() {
        AVIF_ADD_IMAGE_FLAG_SINGLE
    } else {
        AVIF_ADD_IMAGE_FLAG_NONE
    };

    unsafe {
        let encoder = avifEncoderCreate();
        assert!(!encoder.is_null(), "avifEncoderCreate が NULL を返した");
        (*encoder).quality = quality;
        (*encoder).qualityAlpha = quality;
        (*encoder).speed = 6;
        (*encoder).maxThreads = 1;
        (*encoder).timescale = 1000;

        for frame in std::iter::once(first).chain(rest) {
            let channels = frame.channels();
            assert_eq!(frame.pixels().len(), (width * height * channels) as usize);

            let image = avifImageCreate(width, height, 8, AVIF_PIXEL_FORMAT_YUV420);
            assert!(!image.is_null(), "avifImageCreate が NULL を返した");
            (*image).yuvRange = AVIF_RANGE_FULL;
            (*image).colorPrimaries = AVIF_COLOR_PRIMARIES_BT709;
            (*image).transferCharacteristics = AVIF_TRANSFER_CHARACTERISTICS_SRGB;
            (*image).matrixCoefficients = AVIF_MATRIX_COEFFICIENTS_BT601;

            let mut rgb = MaybeUninit::<avifRGBImage>::zeroed();
            avifRGBImageSetDefaults(rgb.as_mut_ptr(), image);
            let mut rgb = rgb.assume_init();
            rgb.format = frame.format();
            rgb.pixels = frame.pixels().as_ptr().cast_mut();
            rgb.rowBytes = width * channels;
            expect_ok(avifImageRGBToYUV(image, &rgb), ptr::null(), "RGB→YUV 変換");

            // α 面の有無は取り込んだ RGB の形式だけで決まる
            assert_eq!(
                (*image).alphaPlane.is_null(),
                channels == 3,
                "α 面の有無が入力の色形式と合わない"
            );

            expect_ok(
                avifEncoderAddImage(encoder, image, 1, flags),
                encoder,
                "フレームの追加",
            );
            avifImageDestroy(image);
        }

        let mut output = avifRWData {
            data: ptr::null_mut(),
            size: 0,
        };
        expect_ok(
            avifEncoderFinish(encoder, &mut output),
            encoder,
            "ファイルの組み立て",
        );
        assert!(!output.data.is_null(), "組み立てたバイト列が空");

        let encoded = Encoded {
            bytes: std::slice::from_raw_parts(output.data, output.size).to_vec(),
            color_obu_size: (*encoder).ioStats.colorOBUSize,
            alpha_obu_size: (*encoder).ioStats.alphaOBUSize,
        };

        avifRWDataFree(&mut output);
        avifEncoderDestroy(encoder);
        encoded
    }
}

/// 一時ファイルへ書き出す。ffmpeg は標準入力の avif を受け取れない
fn temp_avif(bytes: &[u8]) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "avif-sys-smoke-{}-{}.avif",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    path
}

/// ffmpeg でデコードした生 RGB。ffmpeg が無ければ `None`
fn decode_with_ffmpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let path = temp_avif(bytes);

    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output();
    std::fs::remove_file(&path).unwrap();

    let output = match output {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("CI").is_none(),
                "ffmpeg が見つからない。デコーダが無ければ、符号化した絵が入力と合うかを確かめられない"
            );
            eprintln!("ffmpeg が見つからないため、デコードを飛ばす");
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

/// 8×8 の窓ごとに求めた SSIM の平均
///
/// 幅と高さは 8 の倍数であること。
fn plane_ssim(actual: &[u8], expected: &[u8], width: usize, height: usize) -> f64 {
    const WINDOW: usize = 8;
    const C1: f64 = 6.5025; // (0.01 * 255)^2
    const C2: f64 = 58.5225; // (0.03 * 255)^2

    assert_eq!(width % WINDOW, 0);
    assert_eq!(height % WINDOW, 0);
    assert_eq!(actual.len(), width * height);
    assert_eq!(expected.len(), width * height);

    let count = (WINDOW * WINDOW) as f64;
    let mut total = 0.0;
    let mut windows = 0.0;
    for top in (0..height).step_by(WINDOW) {
        for left in (0..width).step_by(WINDOW) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for y in top..top + WINDOW {
                for x in left..left + WINDOW {
                    let a = f64::from(actual[y * width + x]);
                    let b = f64::from(expected[y * width + x]);
                    sa += a;
                    sb += b;
                    saa += a * a;
                    sbb += b * b;
                    sab += a * b;
                }
            }
            let (ma, mb) = (sa / count, sb / count);
            let va = saa / count - ma * ma;
            let vb = sbb / count - mb * mb;
            let cov = sab / count - ma * mb;
            total += ((2.0 * ma * mb + C1) * (2.0 * cov + C2))
                / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            windows += 1.0;
        }
    }
    total / windows
}

/// R / G / B それぞれの SSIM
///
/// 彩度だけの狂いは輝度 1 本では出ないため、チャネルごとに分けて測る。
fn channel_ssim(actual: &[u8], expected: &[u8], width: usize, height: usize) -> [f64; 3] {
    std::array::from_fn(|channel| {
        let plane =
            |rgb: &[u8]| -> Vec<u8> { rgb.iter().skip(channel).step_by(3).copied().collect() };
        plane_ssim(&plane(actual), &plane(expected), width, height)
    })
}

/// ISOBMFF の先頭が期待した brand の ftyp であること
///
/// 単葉は `avif`、シーケンスは `avis` になる。
fn assert_ftyp(bytes: &[u8], brand: &[u8; 4]) {
    assert!(bytes.len() > 12, "組み立てたファイルが短すぎる");
    assert_eq!(&bytes[4..8], b"ftyp", "先頭の箱が ftyp ではない");
    assert_eq!(&bytes[8..12], brand, "major brand が違う");
}

#[test]
fn an_rgb_frame_decodes_near_the_input() {
    let (width, height) = (64u32, 64u32);
    let rgb = gradient_rgb(width, height, 0);
    let encoded = encode(width, height, &[Input::Rgb(&rgb)], 90);

    assert_ftyp(&encoded.bytes, b"avif");
    assert!(encoded.color_obu_size > 0, "色のストリームが空");

    let Some(decoded) = decode_with_ffmpeg(&encoded.bytes) else {
        return;
    };
    assert_eq!(decoded.len(), rgb.len(), "ffmpeg が返した画素数が違う");
    let ssim = channel_ssim(&decoded, &rgb, width as usize, height as usize);
    eprintln!(
        "RGB 単葉の SSIM = R {:.5} / G {:.5} / B {:.5}",
        ssim[0], ssim[1], ssim[2]
    );
    let worst = ssim.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        worst > 0.97,
        "デコードが入力から離れている: SSIM = {ssim:?}"
    );
}

/// α のストリームが落ちるのは、libavif が全面不透明を見て畳む単葉だけ。
/// 先行確保の再発を捕まえるにはシーケンスで見る必要がある。
#[test]
fn an_rgb_sequence_carries_no_alpha_stream() {
    let (width, height) = (64u32, 64u32);
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgb(width, height, n * 8)).collect();
    let inputs: Vec<Input<'_>> = frames.iter().map(|f| Input::Rgb(f)).collect();
    let encoded = encode(width, height, &inputs, 90);

    assert_ftyp(&encoded.bytes, b"avis");
    assert!(encoded.color_obu_size > 0, "色のストリームが空");
    assert_eq!(
        encoded.alpha_obu_size, 0,
        "RGB 入力のシーケンスに α のストリームが付いた"
    );
}

#[test]
fn an_rgba_sequence_carries_an_alpha_stream() {
    let (width, height) = (64u32, 64u32);
    let frames: Vec<Vec<u8>> = (0..3)
        .map(|n| gradient_rgba(width, height, n * 8))
        .collect();
    let inputs: Vec<Input<'_>> = frames.iter().map(|f| Input::Rgba(f)).collect();
    let encoded = encode(width, height, &inputs, 90);

    assert_ftyp(&encoded.bytes, b"avis");
    assert!(encoded.color_obu_size > 0, "色のストリームが空");
    assert!(
        encoded.alpha_obu_size > 0,
        "RGBA 入力のシーケンスに α のストリームが無い"
    );
}
