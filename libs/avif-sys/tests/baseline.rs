//! 現行の AVIF プラグインが出したファイルを、同じ呼び出し列の FFI で組み直して突き合わせる
//!
//! 取得の形を tarball から submodule へ移しても生成物が変わっていないことを見る。
//! 基準の出力かフレーム列が置かれていなければ飛ばす。

use avif_sys::*;
use std::ffi::{CStr, c_int};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::ptr;

/// 突き合わせに使う素材と、それを出した設定
const CLIP: &str = "sc_textScroll";
const BASELINE: &str = "avif_speed6.avif";
const QUALITY: c_int = 75;
const SPEED: c_int = 6;
const MAX_THREADS: c_int = 16;
const TIMESCALE: u64 = 30;
const DURATION: u64 = 1;
const REPETITION_COUNT: c_int = -1;

unsafe extern "C" {
    /// 現行の実装が変換前に呼ぶ先行確保
    fn avifImageAllocatePlanes(image: *mut avifImage, planes: u32) -> c_int;
}

/// 素材とフレーム列
struct Clip {
    width: u32,
    height: u32,
    count: usize,
    rgb: Vec<u8>,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("リポジトリの根")
        .to_path_buf()
}

/// `"key": 数値` を取り出す
fn number(json: &str, key: &str) -> usize {
    let at = json
        .find(&format!("\"{key}\""))
        .unwrap_or_else(|| panic!("{key} が無い"));
    let rest = &json[at + key.len() + 2..];
    let value = rest
        .trim_start()
        .strip_prefix(':')
        .expect("項目の区切り")
        .trim_start();
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    value[..end].parse().expect("数値")
}

/// ストレートアルファの画素を黒へ合成する。ホストが返すフレームと同じ並びになる
fn load_clip(dir: &Path) -> Option<Clip> {
    let meta = std::fs::read_to_string(dir.join("clip.json")).ok()?;
    let raw = std::fs::read(dir.join("frames.rgba")).ok()?;

    let width = number(&meta, "width") as u32;
    let height = number(&meta, "height") as u32;
    let count = number(&meta, "frames");
    assert_eq!(raw.len(), width as usize * height as usize * 4 * count);

    let mut rgb = Vec::with_capacity(raw.len() / 4 * 3);
    for px in raw.chunks_exact(4) {
        let mul = |v: u8| ((u32::from(v) * u32::from(px[3]) + 127) / 255) as u8;
        rgb.extend_from_slice(&[mul(px[0]), mul(px[1]), mul(px[2])]);
    }
    Some(Clip {
        width,
        height,
        count,
        rgb,
    })
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

/// 現行の実装と同じ順序・同じ項目でシーケンスを組む
///
/// 変換前の `avifImageAllocatePlanes` と、CICP を書かないことまで写す。
fn encode_like_the_current_plugin(clip: &Clip) -> Vec<u8> {
    let frame_len = clip.width as usize * clip.height as usize * 3;
    unsafe {
        let encoder = avifEncoderCreate();
        assert!(!encoder.is_null(), "avifEncoderCreate が NULL を返した");
        (*encoder).repetitionCount = REPETITION_COUNT;
        (*encoder).timescale = TIMESCALE;
        (*encoder).quality = QUALITY;
        (*encoder).speed = SPEED;
        (*encoder).maxThreads = MAX_THREADS;

        for index in 0..clip.count {
            let image = avifImageCreate(clip.width, clip.height, 8, AVIF_PIXEL_FORMAT_YUV420);
            assert!(!image.is_null(), "avifImageCreate が NULL を返した");
            expect_ok(
                avifImageAllocatePlanes(image, AVIF_PLANES_ALL),
                ptr::null(),
                "プレーンの確保",
            );

            let mut rgb = std::mem::MaybeUninit::<avifRGBImage>::uninit();
            avifRGBImageSetDefaults(rgb.as_mut_ptr(), image);
            let mut rgb = rgb.assume_init();
            rgb.format = AVIF_RGB_FORMAT_RGB;
            rgb.pixels = clip.rgb[index * frame_len..].as_ptr().cast_mut();
            rgb.rowBytes = clip.width * 3;
            expect_ok(avifImageRGBToYUV(image, &rgb), ptr::null(), "RGB→YUV 変換");

            expect_ok(
                avifEncoderAddImage(encoder, image, DURATION, AVIF_ADD_IMAGE_FLAG_NONE),
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
        let bytes = std::slice::from_raw_parts(output.data, output.size).to_vec();
        avifRWDataFree(&mut output);
        avifEncoderDestroy(encoder);
        bytes
    }
}

/// mvhd / tkhd / mdhd が持つ生成時刻と更新時刻の範囲
///
/// 走行ごとに変わる欄なので、突き合わせの前に潰す。
fn timestamp_ranges(bytes: &[u8]) -> Vec<Range<usize>> {
    fn walk(bytes: &[u8], span: Range<usize>, out: &mut Vec<Range<usize>>) {
        let mut at = span.start;
        while at + 8 <= span.end {
            let declared = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
            let (header, size) = match declared {
                0 => (8, span.end - at),
                1 => {
                    if at + 16 > span.end {
                        return;
                    }
                    let large = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap());
                    (16, large as usize)
                }
                _ => (8, declared),
            };
            if size < header || at + size > span.end {
                return;
            }
            let body = at + header;
            match &kind {
                b"moov" | b"trak" | b"mdia" => walk(bytes, body..at + size, out),
                b"mvhd" | b"tkhd" | b"mdhd" => {
                    let field = if bytes[body] == 1 { 8 } else { 4 };
                    out.push(body + 4..body + 4 + field * 2);
                }
                _ => {}
            }
            at += size;
        }
    }

    let mut ranges = Vec::new();
    walk(bytes, 0..bytes.len(), &mut ranges);
    ranges
}

/// 時刻の欄を 0 で潰した複製
fn without_timestamps(bytes: &[u8]) -> Vec<u8> {
    let mut masked = bytes.to_vec();
    for range in timestamp_ranges(bytes) {
        masked[range].fill(0);
    }
    masked
}

/// 食い違うバイトの位置
fn differences(actual: &[u8], expected: &[u8]) -> Vec<usize> {
    actual
        .iter()
        .zip(expected)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn the_current_output_is_reproduced_from_the_vendored_sources() {
    let root = repository_root();
    let expected = root.join(format!("private/avif/baseline-dump/{CLIP}/{BASELINE}"));
    let Ok(expected) = std::fs::read(&expected) else {
        eprintln!("基準の出力が無いため、突き合わせを飛ばす");
        return;
    };
    let Some(clip) = load_clip(&root.join(format!("private/bench/clips/{CLIP}"))) else {
        eprintln!("フレーム列が無いため、突き合わせを飛ばす");
        return;
    };

    let actual = encode_like_the_current_plugin(&clip);

    assert_eq!(actual.len(), expected.len(), "ファイルの大きさが違う");
    let raw = differences(&actual, &expected);
    eprintln!(
        "生のバイト列で食い違うのは {} 箇所: {:?}",
        raw.len(),
        &raw[..raw.len().min(32)]
    );

    let masked = differences(&without_timestamps(&actual), &without_timestamps(&expected));
    assert!(
        masked.is_empty(),
        "時刻の欄を除いても {} 箇所が食い違う: {:?}",
        masked.len(),
        &masked[..masked.len().min(32)]
    );
}
