//! 出力したアニメーションWebPをデコードし、フレームごとの合成結果が入力と
//! 一致することを確認する
//!
//! バイト一致は ffmpeg が担保する。`image-webp` は独立な2つ目のデコーダで、
//! 表示時間とループ回数の読み出しも受け持つ。ffmpeg が見つからない環境では、
//! そちらだけを飛ばして `image-webp` の結果で判定する。

use image_webp::{LoopCount, WebPDecoder};
use std::cell::RefCell;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
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
///
/// 透過の位置は種でずれる。フレームごとに違う位置が透けることが、透過画素の
/// 下に前のフレームが残る合成を突き合わせの対象にする。
fn normalized_rgba(width: u32, height: u32, seed: u32) -> Vec<u8> {
    let mut rgba = noise((width * height * 4) as usize, seed);
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        pixel[3] = if (index + seed as usize).is_multiple_of(5) {
            0
        } else {
            255
        };
        if pixel[3] == 0 {
            pixel.fill(0);
        }
    }
    rgba
}

/// 種を変えた `count` 枚のRGBA
fn rgba_frames(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| normalized_rgba(width, height, 0x5EED + index as u32))
        .collect()
}

/// 種を変えた `count` 枚の不透明なRGB
fn rgb_frames(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| noise((width * height * 3) as usize, 0xF00D + index as u32))
        .collect()
}

/// RGBをα = 255 のRGBAへ広げる
fn opaque_rgba(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect()
}

/// 投入する順に異なる遅延
///
/// 分母を1000にすると丸めが恒等になり、期待するミリ秒がそのまま添字から決まる。
fn delay_of(index: usize) -> FrameDelay {
    FrameDelay::new(index as u32 * 7 + 20, 1000).unwrap()
}

fn config(color_type: ColorType, num_plays: u32) -> Config {
    Config {
        color_type,
        lossless: true,
        quality: 100.0,
        method: 4,
        num_plays,
    }
}

/// フレーム列を符号化する
fn encode(
    width: u32,
    height: u32,
    config: Config,
    frames: &[Vec<u8>],
) -> Result<(Vec<u8>, Report), Error> {
    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config,
    )?;
    for (index, data) in frames.iter().enumerate() {
        encoder.add_frame(data, delay_of(index))?;
    }
    let (writer, report) = encoder.finish()?;
    Ok((writer.into_inner(), report))
}

/// `image-webp` で読み出した結果
struct Decoded {
    /// 合成済みのフレームごとの生RGBA
    frames: Vec<Vec<u8>>,
    /// フレームごとの表示時間 (ms)
    durations: Vec<u32>,
    loop_count: LoopCount,
}

/// `image-webp` で合成済みのフレームへデコードする
fn decode_with_image_webp(bytes: &[u8], width: u32, height: u32) -> Decoded {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).expect("image-webp が読めない");
    assert!(decoder.is_animated(), "アニメーションになっていない");
    assert_eq!(decoder.dimensions(), (width, height));
    decoder
        .set_background_color([0, 0, 0, 0])
        .expect("背景色を透明にする");

    let opaque = !decoder.has_alpha();
    let size = decoder.output_buffer_size().expect("出力の大きさ");
    let mut frames = Vec::new();
    let mut durations = Vec::new();
    for _ in 0..decoder.num_frames() {
        let mut buffer = vec![0u8; size];
        durations.push(
            decoder
                .read_frame(&mut buffer)
                .expect("image-webp のデコード"),
        );
        frames.push(if opaque { opaque_rgba(&buffer) } else { buffer });
    }

    Decoded {
        frames,
        durations,
        loop_count: decoder.loop_count(),
    }
}

/// バイト列をデコーダへ渡すための一時ファイルへ書き出す
fn temp_webp(bytes: &[u8]) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "webp-encoder-roundtrip-{}-{}.webp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    path
}

/// ffmpeg を生RGBAの書き出しで走らせる。ffmpeg が無ければ `None`
fn run_ffmpeg(bytes: &[u8]) -> Option<Output> {
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

    match output {
        Ok(output) => Some(output),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("CI").is_none(),
                "ffmpeg が見つからない。デコーダが1つでは、一部のデコーダだけが読める出力を見つけられない"
            );
            eprintln!("ffmpeg が見つからないため、そちらのデコードを飛ばす");
            None
        }
        Err(e) => panic!("ffmpeg の起動に失敗した: {e}"),
    }
}

/// ffmpeg で合成済みのフレームへデコードした生RGBA。ffmpeg が無ければ `None`
fn decode_with_ffmpeg(bytes: &[u8], width: u32, height: u32) -> Option<Vec<Vec<u8>>> {
    let output = run_ffmpeg(bytes)?;
    assert!(
        output.status.success(),
        "ffmpeg のデコードに失敗した: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let frame_len = (width * height * 4) as usize;
    assert_eq!(output.stdout.len() % frame_len, 0);
    Some(
        output
            .stdout
            .chunks_exact(frame_len)
            .map(<[u8]>::to_vec)
            .collect(),
    )
}

/// 出力を2つのデコーダへ通し、フレームごとの合成結果が入力と一致することを確かめる
fn round_trip(width: u32, height: u32, config: Config, frames: &[Vec<u8>]) -> (Vec<u8>, Report) {
    let expected: Vec<Vec<u8>> = match config.color_type {
        ColorType::Rgb8 => frames.iter().map(|rgb| opaque_rgba(rgb)).collect(),
        ColorType::Rgba8 => frames.to_vec(),
    };

    let (bytes, report) = encode(width, height, config, frames).unwrap();

    if let Some(decoded) = decode_with_ffmpeg(&bytes, width, height) {
        assert_eq!(decoded.len(), expected.len(), "ffmpeg が返したフレーム数");
        assert_eq!(decoded, expected, "ffmpeg のデコードが入力と違う");
    }

    let decoded = decode_with_image_webp(&bytes, width, height);
    assert_eq!(
        decoded.frames, expected,
        "image-webp のデコードが入力と違う"
    );

    (bytes, report)
}

/// 可逆の経路で、フレームごとの合成結果が入力へバイト一致で戻る
///
/// アニメーションのバイト一致を ffmpeg が担保できること自体を、ここが確かめる。
#[test]
fn every_frame_of_a_lossless_animation_composes_back_to_the_input() {
    let (width, height) = (61, 37);
    let frames = rgba_frames(width, height, 5);

    let (_, report) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);

    assert_eq!(
        report,
        Report {
            merged_frames: 0,
            delay_clamped: false,
            has_alpha: true,
        }
    );
}

#[test]
fn an_opaque_rgb_animation_composes_back_to_the_input() {
    let (width, height) = (48, 32);
    let frames = rgb_frames(width, height, 4);

    let (bytes, report) = round_trip(width, height, config(ColorType::Rgb8, 0), &frames);

    assert!(!report.has_alpha);
    assert_eq!(bytes[VP8X_FLAGS_OFFSET], ANIMATION);
}

/// VP8Xの位置とフラグ
const VP8X_FLAGS_OFFSET: usize = 20;
const ANIMATION: u8 = 0x02;
const ALPHA: u8 = 0x10;

/// 素材が透過を持つときだけALPHAフラグが後埋めで立つ
#[test]
fn the_alpha_flag_is_filled_in_from_the_frames() {
    let (width, height) = (16, 16);

    let transparent = rgba_frames(width, height, 3);
    let (bytes, report) = encode(width, height, config(ColorType::Rgba8, 0), &transparent).unwrap();
    assert!(report.has_alpha);
    assert_eq!(bytes[VP8X_FLAGS_OFFSET], ANIMATION | ALPHA);

    let opaque: Vec<Vec<u8>> = transparent
        .iter()
        .map(|frame| {
            frame
                .chunks_exact(4)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
                .collect()
        })
        .collect();
    let (bytes, report) = encode(width, height, config(ColorType::Rgba8, 0), &opaque).unwrap();
    assert!(!report.has_alpha);
    assert_eq!(bytes[VP8X_FLAGS_OFFSET], ANIMATION);
}

/// 表示時間はフレームごとに書いた値がそのまま読み出せる
#[test]
fn the_frame_durations_survive_the_round_trip() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 6);

    let (bytes, report) = encode(width, height, config(ColorType::Rgba8, 0), &frames).unwrap();

    let expected: Vec<u32> = (0..frames.len())
        .map(|index| index as u32 * 7 + 20)
        .collect();
    assert_eq!(
        decode_with_image_webp(&bytes, width, height).durations,
        expected
    );
    assert!(!report.delay_clamped);
}

/// 丸めの残差はフレームをまたいで持ち越される
///
/// 30fps を1フレームずつ丸めると33msが並ぶ。3フレームで100msになるのは、
/// 残差を積んだときだけ。
#[test]
fn the_rounding_residual_carries_across_frames() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 6);

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for frame in &frames {
        encoder
            .add_frame(frame, FrameDelay::new(1, 30).unwrap())
            .unwrap();
    }
    let bytes = encoder.finish().unwrap().0.into_inner();

    assert_eq!(
        decode_with_image_webp(&bytes, width, height).durations,
        [33, 34, 33, 33, 34, 33]
    );
}

/// ミリ秒に満たない遅延は下限まで切り上げ、それを `Report` に載せる
#[test]
fn a_delay_below_a_millisecond_is_raised_and_reported() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 4);

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for frame in &frames {
        encoder
            .add_frame(frame, FrameDelay::new(1, 10000).unwrap())
            .unwrap();
    }
    let (writer, report) = encoder.finish().unwrap();

    assert!(report.delay_clamped);
    let bytes = writer.into_inner();
    assert_eq!(
        decode_with_image_webp(&bytes, width, height).durations,
        [1, 1, 1, 1]
    );
}

/// 再生回数はそのまま書き、u16を超えるぶんは飽和させる
#[test]
fn the_number_of_plays_survives_the_round_trip() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 2);

    for (num_plays, expected) in [
        (0, LoopCount::Forever),
        (1, LoopCount::Times(NonZeroU16::new(1).unwrap())),
        (65535, LoopCount::Times(NonZeroU16::new(65535).unwrap())),
        (u32::MAX, LoopCount::Times(NonZeroU16::new(65535).unwrap())),
    ] {
        let (bytes, _) =
            encode(width, height, config(ColorType::Rgba8, num_plays), &frames).unwrap();
        assert_eq!(
            decode_with_image_webp(&bytes, width, height).loop_count,
            expected,
            "{num_plays}"
        );
    }
}

#[test]
fn zero_frames_are_refused() {
    assert!(matches!(
        Encoder::new(
            Cursor::new(Vec::new()),
            8,
            8,
            0,
            config(ColorType::Rgba8, 0)
        ),
        Err(Error::InvalidFrameCount)
    ));
}

#[test]
fn a_frame_count_other_than_the_declared_one_is_refused() {
    let frames = rgba_frames(8, 8, 3);

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        8,
        8,
        3,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    encoder.add_frame(&frames[0], delay_of(0)).unwrap();
    assert!(matches!(
        encoder.finish(),
        Err(Error::FrameCountMismatch {
            expected: 3,
            actual: 1
        })
    ));

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        8,
        8,
        2,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for frame in &frames[..2] {
        encoder.add_frame(frame, delay_of(0)).unwrap();
    }
    assert!(matches!(
        encoder.add_frame(&frames[2], delay_of(2)),
        Err(Error::FrameCountMismatch {
            expected: 2,
            actual: 3
        })
    ));
}

/// 書けたバイト列を共有の控えへ写す書き出し先
///
/// `budget` バイトを超える書き出しは失敗する。失敗したエンコーダも、
/// [`Encoder::finish`] を呼ばずに捨てたエンコーダも書き出し先を返さないため、
/// 閉じていないファイルはこの控えからしか見られない。
struct SharedWriter {
    remaining: usize,
    position: u64,
    written: Rc<RefCell<Vec<u8>>>,
}

impl SharedWriter {
    fn new(budget: usize) -> (Self, Rc<RefCell<Vec<u8>>>) {
        let written = Rc::new(RefCell::new(Vec::new()));
        let writer = SharedWriter {
            remaining: budget,
            position: 0,
            written: Rc::clone(&written),
        };
        (writer, written)
    }
}

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.remaining {
            self.remaining = 0;
            return Err(std::io::Error::other("書き出し失敗"));
        }
        self.remaining -= buf.len();
        let mut written = self.written.borrow_mut();
        let at = self.position as usize;
        if written.len() < at + buf.len() {
            written.resize(at + buf.len(), 0);
        }
        written[at..at + buf.len()].copy_from_slice(buf);
        self.position += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for SharedWriter {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let end = self.written.borrow().len() as i64;
        let at = match pos {
            SeekFrom::Start(at) => at as i64,
            SeekFrom::End(offset) => end + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };
        if at < 0 {
            return Err(std::io::Error::other("負の位置へのシーク"));
        }
        self.position = at as u64;
        Ok(self.position)
    }
}

/// ヘッダ (RIFF + VP8X + ANIM) のバイト数
const HEAD_BYTES: usize = 44;

/// 閉じていないファイルはRIFFのサイズが0のまま残り、厳密な読み手が弾く
///
/// 後埋めが書き戻すのはサイズ欄の4バイトとVP8Xのフラグの1バイトだけで、
/// 残りはANMFを流したときのまま変わらない。
#[test]
fn a_file_that_was_never_finished_keeps_a_zero_riff_size() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 4);

    let (writer, written) = SharedWriter::new(usize::MAX);
    let mut encoder = Encoder::new(
        writer,
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for (index, frame) in frames.iter().enumerate() {
        encoder.add_frame(frame, delay_of(index)).unwrap();
    }
    drop(encoder);
    let unfinished = written.borrow().clone();

    let (finished, _) = encode(width, height, config(ColorType::Rgba8, 0), &frames).unwrap();
    assert_eq!(unfinished.len(), finished.len());
    assert_eq!(u32::from_le_bytes(unfinished[4..8].try_into().unwrap()), 0);
    assert_eq!(
        u32::from_le_bytes(finished[4..8].try_into().unwrap()) as usize,
        finished.len() - 8
    );
    assert_eq!(unfinished[VP8X_FLAGS_OFFSET], ANIMATION);
    assert_eq!(finished[VP8X_FLAGS_OFFSET], ANIMATION | ALPHA);
    assert_eq!(
        unfinished[8..VP8X_FLAGS_OFFSET],
        finished[8..VP8X_FLAGS_OFFSET]
    );
    assert_eq!(
        unfinished[VP8X_FLAGS_OFFSET + 1..],
        finished[VP8X_FLAGS_OFFSET + 1..]
    );

    assert!(
        WebPDecoder::new(Cursor::new(&unfinished)).is_err(),
        "image-webp が閉じていないファイルを読めている"
    );
}

/// 途中で切れたチャンクの列に書き足すと読めないWebPになるため、失敗後は受け付けない
#[test]
fn a_failed_write_poisons_the_encoder() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 4);
    let (writer, _) = SharedWriter::new(HEAD_BYTES + 900);
    let mut encoder = Encoder::new(
        writer,
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();

    let failure = frames
        .iter()
        .enumerate()
        .find_map(|(index, frame)| encoder.add_frame(frame, delay_of(index)).err());
    assert!(matches!(failure, Some(Error::Io(_))), "{failure:?}");

    assert!(matches!(
        encoder.add_frame(&frames[0], delay_of(0)),
        Err(Error::Poisoned)
    ));
    assert!(matches!(encoder.finish(), Err(Error::Poisoned)));
}

/// ヘッダを書けない書き出し先では、エンコーダを作れない
#[test]
fn a_writer_that_cannot_take_the_header_is_rejected() {
    let (writer, _) = SharedWriter::new(4);
    assert!(matches!(
        Encoder::new(writer, 8, 8, 2, config(ColorType::Rgba8, 0)),
        Err(Error::Io(_))
    ));
}
