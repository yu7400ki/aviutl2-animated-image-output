//! 符号化した .avif を ffmpeg / ffprobe で読み直し、入力と設定に照らす
//!
//! 非可逆しか無いので画素の一致は主張せず、絵は SSIM の閾値で見る。
//! 表示時間とループ回数は ISOBMFF の箱を直接読む。
//!
//! ffmpeg / ffprobe が見つからない環境では、外部の道具に依る主張を飛ばす。

use avif_encoder::{ColorType, Config, Encoder, YuvFormat};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

/// 試験に使うキャンバス。SSIM の窓 8 で割り切れること
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;

/// 1秒あたりの時間刻み数。ミリ秒との取り違えが数に出るよう 1000 から外す
const TIMESCALE: u32 = 30000;

/// 1フレームの表示時間 (30000 刻みで 1/29.97 秒)
const DURATION: u32 = 1001;

/// 横方向と縦方向で滑らかに変わる不透明な RGB。`phase` は絵をずらす
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

/// `gradient_rgb` に、左から右へ薄れる α を足したもの
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

/// `gradient_rgb` に、全面不透明な α を足したもの
fn opaque_rgba(phase: u32) -> Vec<u8> {
    gradient_rgb(phase)
        .chunks_exact(3)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect()
}

fn config(color_type: ColorType, num_plays: u32) -> Config {
    Config {
        color_type,
        quality: 90,
        speed: 6,
        yuv_format: YuvFormat::Yuv420,
        num_plays,
        timescale: TIMESCALE,
        max_threads: 1,
    }
}

/// フレームを順に投入して .avif を組む
fn encode(frames: &[Vec<u8>], config: Config) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new(), WIDTH, HEIGHT, frames.len() as u32, config).unwrap();
    for frame in frames {
        encoder.add_frame(frame, DURATION).unwrap();
    }
    encoder.finish().unwrap()
}

/// 一時ファイルへ書き出す。ffmpeg は標準入力の avif を受け取れない
fn temp_avif(bytes: &[u8]) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "avif-encoder-roundtrip-{}-{}.avif",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create(&path)
        .unwrap()
        .write_all(bytes)
        .unwrap();
    path
}

/// `bytes` を一時ファイルに置いて `program` を回す。道具が無ければ `None`
fn run(program: &str, bytes: &[u8], args: &[&str]) -> Option<Output> {
    let path = temp_avif(bytes);
    let mut command = Command::new(program);
    command.args(["-v", "error", "-i"]).arg(&path).args(args);
    let output = command.output();
    std::fs::remove_file(&path).unwrap();

    match output {
        Ok(output) => {
            assert!(
                output.status.success(),
                "{program} が失敗した: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Some(output)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("CI").is_none(),
                "{program} が見つからない。読み直せなければ、書いた内容を確かめられない"
            );
            eprintln!("{program} が見つからないため、この主張を飛ばす");
            None
        }
        Err(e) => panic!("{program} の起動に失敗した: {e}"),
    }
}

/// `stream` 番のストリームを RGB24 の生画素へ展開する
fn decode(bytes: &[u8], stream: u32) -> Option<Vec<u8>> {
    let map = format!("0:{stream}");
    let output = run(
        "ffmpeg",
        bytes,
        &[
            "-map",
            &map,
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ],
    )?;
    Some(output.stdout)
}

/// ffprobe が並べた項目を 1 行ずつ返す
fn probe(bytes: &[u8], args: &[&str]) -> Option<Vec<String>> {
    let output = run("ffprobe", bytes, args)?;
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect(),
    )
}

/// ffprobe が数えた AV1 ストリームの本数を `expected` と突き合わせる
fn assert_stream_count(bytes: &[u8], expected: usize) {
    let Some(lines) = probe(bytes, &["-show_entries", "stream=index", "-of", "csv=p=0"]) else {
        return;
    };
    assert_eq!(lines.len(), expected, "AV1 ストリームの本数が違う");
}

/// `stream` 番のストリームに載る色の指定を、名前順に並べて返す
fn cicp(bytes: &[u8], stream: u32) -> Option<Vec<String>> {
    let mut lines = probe(
        bytes,
        &[
            "-select_streams",
            &stream.to_string(),
            "-show_entries",
            "stream=color_primaries,color_transfer,color_space,color_range",
            "-of",
            "default=noprint_wrappers=1",
        ],
    )?;
    lines.sort();
    Some(lines)
}

/// `data` に並ぶ箱を、型と中身の組で返す
fn children(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut boxes = Vec::new();
    let mut rest = data;
    while rest.len() >= 8 {
        let size = u32::from_be_bytes(rest[0..4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = rest[4..8].try_into().unwrap();
        assert_ne!(size, 1, "64bit の大きさを持つ箱は現れない");
        let size = if size == 0 { rest.len() } else { size };
        assert!(
            (8..=rest.len()).contains(&size),
            "箱の大きさが範囲の外: {size}"
        );
        boxes.push((kind, &rest[8..size]));
        rest = &rest[size..];
    }
    boxes
}

/// `data` の直下から `kind` の箱を探す
fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data)
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, body)| body)
}

/// version 1 の mvhd / mdhd が持つ timescale と duration
fn header(body: &[u8]) -> (u32, u64) {
    assert_eq!(body[0], 1, "version 1 の箱ではない");
    (
        u32::from_be_bytes(body[20..24].try_into().unwrap()),
        u64::from_be_bytes(body[24..32].try_into().unwrap()),
    )
}

/// moov から読み出した時間の欄
struct Timing {
    /// mvhd の timescale
    timescale: u32,
    /// mvhd の duration。ループを含む全体の長さで、無限ループでは `u64::MAX`
    total: u64,
    /// trak ごとの mdhd の duration。1周の長さ
    media: Vec<u64>,
}

impl Timing {
    /// moov を持たない単葉では `None`
    fn read(bytes: &[u8]) -> Option<Self> {
        let moov = find(bytes, b"moov")?;
        let (timescale, total) = header(find(moov, b"mvhd").expect("mvhd が無い"));
        let media = children(moov)
            .into_iter()
            .filter(|(kind, _)| kind == b"trak")
            .map(|(_, trak)| {
                let mdia = find(trak, b"mdia").expect("mdia が無い");
                header(find(mdia, b"mdhd").expect("mdhd が無い")).1
            })
            .collect();
        Some(Timing {
            timescale,
            total,
            media,
        })
    }

    /// 1周の長さに対する全体の長さの比。無限ループでは `None`
    fn plays(&self) -> Option<u64> {
        let loop_length = self.media[0];
        (self.total != u64::MAX).then(|| {
            assert_eq!(self.total % loop_length, 0, "全体の長さが1周で割り切れない");
            self.total / loop_length
        })
    }
}

/// ISOBMFF の先頭が期待した brand の ftyp であること
fn assert_ftyp(bytes: &[u8], brand: &[u8; 4]) {
    assert!(bytes.len() > 12, "組み立てたファイルが短すぎる");
    assert_eq!(&bytes[4..8], b"ftyp", "先頭の箱が ftyp ではない");
    assert_eq!(&bytes[8..12], brand, "major brand が違う");
}

/// 8×8 の窓ごとに求めた SSIM の平均
fn plane_ssim(actual: &[u8], expected: &[u8], width: usize, height: usize) -> f64 {
    const WINDOW: usize = 8;
    const C1: f64 = 6.5025; // (0.01 * 255)^2
    const C2: f64 = 58.5225; // (0.03 * 255)^2

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

/// R / G / B それぞれで測った SSIM のうち最も低いもの
///
/// 彩度だけの狂いは輝度 1 本では出ないため、チャネルごとに分けて測る。
fn worst_channel_ssim(actual: &[u8], expected: &[u8]) -> f64 {
    let plane = |rgb: &[u8], channel: usize| -> Vec<u8> {
        rgb.iter().skip(channel).step_by(3).copied().collect()
    };
    (0..3)
        .map(|channel| {
            plane_ssim(
                &plane(actual, channel),
                &plane(expected, channel),
                WIDTH as usize,
                HEIGHT as usize,
            )
        })
        .fold(f64::INFINITY, f64::min)
}

/// 展開した生画素を、投入したフレームと突き合わせる
fn assert_frames_match(decoded: &[u8], frames: &[Vec<u8>]) {
    let frame_len = (WIDTH * HEIGHT * 3) as usize;
    assert_eq!(
        decoded.len(),
        frame_len * frames.len(),
        "展開されたフレーム数が投入した数と違う"
    );
    for (index, (actual, expected)) in decoded.chunks_exact(frame_len).zip(frames).enumerate() {
        // RGBA を投入していても展開は RGB24 なので、α を落として比べる
        let expected: Vec<u8> = if expected.len() == frame_len {
            expected.clone()
        } else {
            expected
                .chunks_exact(4)
                .flat_map(|pixel| pixel[..3].to_vec())
                .collect()
        };
        let ssim = worst_channel_ssim(actual, &expected);
        assert!(ssim > 0.97, "{index} 番のフレームの SSIM が低い: {ssim}");
    }
}

#[test]
fn a_sequence_keeps_its_frame_count_and_the_length_of_one_loop() {
    let frames: Vec<Vec<u8>> = (0..5).map(|n| gradient_rgb(n * 8)).collect();
    let bytes = encode(&frames, config(ColorType::Rgb8, 1));

    assert_ftyp(&bytes, b"avis");
    let timing = Timing::read(&bytes).expect("シーケンスに moov が無い");
    assert_eq!(timing.timescale, TIMESCALE);
    assert_eq!(
        timing.media,
        vec![u64::from(DURATION) * frames.len() as u64],
        "1周の総再生時間が投入した表示時間の和と違う"
    );

    let Some(decoded) = decode(&bytes, 1) else {
        return;
    };
    assert_frames_match(&decoded, &frames);
}

/// 変換前にプレーンを先に確保すると、RGB 入力にも α のストリームが付く
#[test]
fn an_rgb_sequence_carries_only_the_color_stream() {
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgb(n * 8)).collect();
    let bytes = encode(&frames, config(ColorType::Rgb8, 1));

    let timing = Timing::read(&bytes).unwrap();
    assert_eq!(timing.media.len(), 1, "シーケンスのトラック数が違う");
    assert_stream_count(&bytes, 2);
}

#[test]
fn an_rgba_sequence_carries_a_separate_alpha_stream() {
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgba(n * 8)).collect();
    let bytes = encode(&frames, config(ColorType::Rgba8, 1));

    let timing = Timing::read(&bytes).unwrap();
    assert_eq!(timing.media.len(), 2, "シーケンスのトラック数が違う");
    assert_stream_count(&bytes, 4);
}

/// CP=1 / TC=13 / MC=6 / full range を明示して書く
#[test]
fn the_color_description_is_written_explicitly() {
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgb(n * 8)).collect();
    let bytes = encode(&frames, config(ColorType::Rgb8, 1));

    let Some(tags) = cicp(&bytes, 1) else {
        return;
    };
    assert_eq!(
        tags,
        [
            "color_primaries=bt709",
            "color_range=pc",
            "color_space=smpte170m",
            "color_transfer=iec61966-2-1",
        ]
    );
}

#[test]
fn the_play_count_is_the_ratio_of_the_whole_to_one_loop() {
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgb(n * 8)).collect();
    for num_plays in [1, 2, 7] {
        let bytes = encode(&frames, config(ColorType::Rgb8, num_plays));
        let timing = Timing::read(&bytes).unwrap();
        assert_eq!(timing.plays(), Some(u64::from(num_plays)), "{num_plays} 回");
    }
}

#[test]
fn a_zero_play_count_becomes_an_endless_loop() {
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgb(n * 8)).collect();
    let bytes = encode(&frames, config(ColorType::Rgb8, 0));
    let timing = Timing::read(&bytes).unwrap();
    assert_eq!(timing.total, u64::MAX, "無限ループの印が立っていない");
    assert_eq!(timing.plays(), None);
}

#[test]
fn a_lone_frame_becomes_a_still_image() {
    let frames = vec![gradient_rgb(0)];
    let bytes = encode(&frames, config(ColorType::Rgb8, 1));

    assert_ftyp(&bytes, b"avif");
    assert!(
        Timing::read(&bytes).is_none(),
        "単葉にトラックが書かれている"
    );
    assert_stream_count(&bytes, 1);

    let Some(decoded) = decode(&bytes, 0) else {
        return;
    };
    assert_frames_match(&decoded, &frames);
}

/// 全面不透明な α のペイロードは、単葉として符号化したときだけ落ちる
///
/// 1枚しか積まないファイルは単葉として組まれるので、単葉であることは
/// トラックの有無には出ない。落ちた α だけが単葉としての符号化を示す。
#[test]
fn an_opaque_alpha_is_dropped_from_a_lone_frame() {
    let still = encode(&[opaque_rgba(0)], config(ColorType::Rgba8, 1));
    assert_ftyp(&still, b"avif");
    assert_stream_count(&still, 1);

    let sequence: Vec<Vec<u8>> = (0..3).map(opaque_rgba).collect();
    let sequence = encode(&sequence, config(ColorType::Rgba8, 1));
    assert_stream_count(&sequence, 4);
}

#[test]
fn an_rgba_sequence_decodes_near_the_input() {
    let frames: Vec<Vec<u8>> = (0..3).map(|n| gradient_rgba(n * 8)).collect();
    let bytes = encode(&frames, config(ColorType::Rgba8, 1));

    let Some(decoded) = decode(&bytes, 2) else {
        return;
    };
    assert_frames_match(&decoded, &frames);
}
