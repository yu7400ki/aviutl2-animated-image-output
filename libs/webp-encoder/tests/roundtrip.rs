//! 出力したアニメーションWebPをデコードし、フレームごとの合成結果が入力と
//! 一致することを確認する
//!
//! バイト一致は ffmpeg が担保する。`image-webp` は独立な2つ目のデコーダで、
//! 表示時間とループ回数の読み出しも受け持つ。重ねる合成だけは 0.2.4 が仕様から
//! 外れているため、`assert_close` がその逸脱の形へ絞って逃がす。ffmpeg が
//! 見つからない環境では、そちらのデコードを飛ばして `image-webp` の結果で判定する。

use image_webp::{LoopCount, WebPDecoder};
use std::cell::RefCell;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::available_parallelism;
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

/// 背景に開ける透明な窓の一辺の長さ
const WINDOW: u32 = 5;

/// 不透明な背景に透明な窓を1つ開けたRGBA
///
/// 窓を動かさないフレームは前フレームと完全に一致し、動かしたフレームは
/// 前後の窓を囲む範囲だけが変わる。窓の位置がフレームごとに違うので、
/// 透過画素の下に何が残るかもフレームごとに違う。
fn windowed_rgba(width: u32, height: u32, at: (u32, u32)) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let inside = x.wrapping_sub(at.0) < WINDOW && y.wrapping_sub(at.1) < WINDOW;
            rgba.extend_from_slice(&if inside {
                [0, 0, 0, 0]
            } else {
                [(x * 3) as u8, (y * 7) as u8, 0x40, 0xFF]
            });
        }
    }
    rgba
}

/// 動く四角の一辺の長さ
const SPRITE: u32 = 6;

/// 四角を塗る色
const SPRITE_COLOR: [u8; 4] = [0x20, 0x40, 0x60, 0xFF];

/// 透過の面を不透明な四角が1つ動くRGBA
///
/// 四角を囲む矩形の外は前のフレームと同じ透過なので、前のフレームの矩形を
/// 抜いた方が矩形が狭くなる。位置が重なるので、抜いた後と抜かない前の
/// キャンバスは矩形の中で食い違う。
fn sprite_rgba(width: u32, height: u32, at: (u32, u32)) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let inside = x.wrapping_sub(at.0) < SPRITE && y.wrapping_sub(at.1) < SPRITE;
            rgba.extend_from_slice(&if inside { SPRITE_COLOR } else { [0, 0, 0, 0] });
        }
    }
    rgba
}

/// 不透明な面を不透明な四角が1つ動くRGBA
///
/// 矩形の中の画素はすべて不透明なので重ねる形で載り、変わらなかった画素は
/// 完全透過へ置き換えられる。素材そのものには透過画素が無い。
fn patched_rgba(width: u32, height: u32, at: (u32, u32)) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let inside = x.wrapping_sub(at.0) < SPRITE && y.wrapping_sub(at.1) < SPRITE;
            rgba.extend_from_slice(&if inside {
                SPRITE_COLOR
            } else {
                [(x * 3) as u8, (y * 7) as u8, 0x40, 0xFF]
            });
        }
    }
    rgba
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

/// ANMFのヘッダが載せているフレームの置き方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    /// キャンバス上の矩形 (x, y, 幅, 高さ)
    rect: (u32, u32, u32, u32),
    /// 透過画素を下のキャンバスへ重ねるか
    blend: bool,
    /// 表示した後に矩形を背景色で抜くか
    dispose: bool,
}

/// ファイル先頭のRIFFヘッダ (FourCC、サイズ、`WEBP`) のバイト数
const FILE_HEADER: usize = 12;

/// 24bitのリトルエンディアンを読む
fn u24(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])
}

/// 出力のANMFを順に読み、フレームごとの置き方を取り出す
///
/// 決定は符号化の中に隠れるので、合成の突き合わせだけでは「どちらの候補が
/// 選ばれたか」を問えない。
fn placements(bytes: &[u8]) -> Vec<Placed> {
    let mut placed = Vec::new();
    let mut cursor = FILE_HEADER;
    while cursor + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let body = cursor + 8;
        if &bytes[cursor..cursor + 4] == b"ANMF" {
            let header = &bytes[body..body + 16];
            placed.push(Placed {
                rect: (
                    u24(&header[0..3]) * 2,
                    u24(&header[3..6]) * 2,
                    u24(&header[6..9]) + 1,
                    u24(&header[9..12]) + 1,
                ),
                blend: header[15] & 0x02 == 0,
                dispose: header[15] & 0x01 == 1,
            });
        }
        cursor = body + size + (size & 1);
    }
    placed
}

/// `image-webp` の合成が入力から離れていないことを確かめる
///
/// `image-webp` 0.2.4 は重ねる合成が仕様から外れており、完全不透明な画素の
/// RGBを1だけ下げて返す。逃がすのはその1方向だけで、次の3つは厳密一致を要求する。
///
/// - αのバイト。逸脱はRGBの3成分にしか現れない
/// - 重ねるフレームが1つも現れていない間のフレーム。上書きの合成は仕様どおり
/// - 0へ下がる余地の無い値。`expected` を下回る側だけを許す
fn assert_close(decoded: &[Vec<u8>], expected: &[Vec<u8>], bytes: &[u8]) {
    /// RGBAのαが並ぶ位置
    const ALPHA: usize = 3;

    let blended = placements(bytes)
        .iter()
        .position(|frame| frame.blend)
        .unwrap_or(usize::MAX);

    assert_eq!(
        decoded.len(),
        expected.len(),
        "image-webp が返したフレーム数"
    );
    for (index, (decoded, expected)) in decoded.iter().zip(expected).enumerate() {
        assert_eq!(decoded.len(), expected.len(), "フレーム{index}の長さ");
        for (at, (decoded, expected)) in decoded.iter().zip(expected).enumerate() {
            let slack = i32::from(index >= blended && at % 4 != ALPHA);
            let deviation = i32::from(*expected) - i32::from(*decoded);
            assert!(
                (0..=slack).contains(&deviation),
                "フレーム{index}のバイト{at}が {expected} から {decoded} へずれた"
            );
        }
    }
}

/// ANMFの列を2つ並べ、2周ぶんのアニメーションにする
///
/// ANMFは1つずつが自足しているので、並べ直すだけで折り返しを1本の
/// アニメーションとして表せる。キャンバスを持ち越すデコーダが2周目に
/// 見るものが、そのまま後半のフレームになる。
fn looped_twice(bytes: &[u8]) -> Vec<u8> {
    let mut looped = bytes.to_vec();
    looped.extend_from_slice(&bytes[HEAD_BYTES..]);
    let size = (looped.len() - 8) as u32;
    looped[4..8].copy_from_slice(&size.to_le_bytes());
    looped
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
    assert_close(&decoded.frames, &expected, &bytes);

    let placed = placements(&bytes);
    let first = placed.first().expect("フレームが1つ以上書かれている");
    assert_eq!(
        first.rect,
        (0, 0, width, height),
        "先頭フレームが全面でない"
    );
    assert!(!first.blend, "先頭フレームが重ねる形になっている");
    let last = placed.last().expect("フレームが1つ以上書かれている");
    assert!(!last.dispose, "最終フレームが矩形を抜いている");

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
        }
    );
}

#[test]
fn an_opaque_rgb_animation_composes_back_to_the_input() {
    let (width, height) = (48, 32);
    let frames = rgb_frames(width, height, 4);

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgb8, 0), &frames);

    assert_eq!(bytes[VP8X_FLAGS_OFFSET], ANIMATION);
}

/// VP8Xの位置とフラグ
const VP8X_FLAGS_OFFSET: usize = 20;
const ANIMATION: u8 = 0x02;
const ALPHA: u8 = 0x10;

/// ALPHAフラグは書いたフレームの画素から後埋めで立つ
///
/// 画素が毎フレーム総入れ替えになる素材では置き換えの相手が無いので、
/// 不透明な素材の出力にαは現れない。
#[test]
fn the_alpha_flag_is_filled_in_from_the_frames() {
    let (width, height) = (16, 16);

    let transparent = rgba_frames(width, height, 3);
    let (bytes, _) = encode(width, height, config(ColorType::Rgba8, 0), &transparent).unwrap();
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
    let (bytes, _) = encode(width, height, config(ColorType::Rgba8, 0), &opaque).unwrap();
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

/// 差分の無いフレームは符号化せず、前のフレームの表示時間へ併合する
///
/// 書いたフレームは減るが、合成の見え方も総再生時間も変わらない。
#[test]
fn a_frame_without_a_difference_extends_the_previous_one() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = [(3, 3), (3, 3), (12, 7), (12, 7), (12, 7), (5, 11)]
        .map(|at| windowed_rgba(width, height, at))
        .to_vec();

    let (bytes, report) = encode(width, height, config(ColorType::Rgba8, 0), &frames).unwrap();

    assert_eq!(report.merged_frames, 3);
    let written = [frames[0].clone(), frames[2].clone(), frames[5].clone()];

    let decoded = decode_with_image_webp(&bytes, width, height);
    assert_eq!(
        decoded.frames.len(),
        frames.len() - report.merged_frames as usize
    );
    assert_close(&decoded.frames, &written, &bytes);

    // 併合したフレームの遅延は、残したフレームの表示時間へ積まれる
    assert_eq!(decoded.durations, [20 + 27, 34 + 41 + 48, 55]);
    assert_eq!(
        decoded.durations.iter().map(|&d| u64::from(d)).sum::<u64>(),
        (0..frames.len() as u64).map(|index| index * 7 + 20).sum()
    );

    if let Some(composed) = decode_with_ffmpeg(&bytes, width, height) {
        assert_eq!(composed, written, "ffmpeg のデコードが入力と違う");
    }
}

/// 透過画素の下のRGBだけが違うフレームは、正規化を経て併合される
#[test]
fn a_frame_differing_only_under_transparent_pixels_is_merged() {
    let (width, height) = (24, 16);
    let base = windowed_rgba(width, height, (3, 3));
    let repainted: Vec<u8> = base
        .chunks_exact(4)
        .flat_map(|pixel| {
            if pixel[3] == 0 {
                [9, 8, 7, 0]
            } else {
                [pixel[0], pixel[1], pixel[2], pixel[3]]
            }
        })
        .collect();
    assert_ne!(
        repainted, base,
        "透過画素を持たない素材では正規化を問えない"
    );
    let frames = vec![
        base.clone(),
        repainted,
        windowed_rgba(width, height, (9, 6)),
    ];

    let (bytes, report) = encode(width, height, config(ColorType::Rgba8, 0), &frames).unwrap();

    assert_eq!(report.merged_frames, 1);
    let decoded = decode_with_image_webp(&bytes, width, height);
    assert_close(&decoded.frames, &[base, frames[2].clone()], &bytes);
    assert_eq!(decoded.durations, [20 + 27, 34]);
}

/// ループの折り返しで、2周目の合成が1周目と一致する
///
/// 先頭フレームが全面かつblend無しなら、持ち越したキャンバスが完全に
/// 上書きされる。blend有りにすると、2周目は先頭フレームの透過画素の下に
/// 1周目の描画が透けて食い違う。
#[test]
fn the_second_loop_composes_the_same_as_the_first() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = [(3, 3), (12, 7), (20, 13), (16, 4)]
        .map(|at| windowed_rgba(width, height, at))
        .to_vec();

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);
    let looped = looped_twice(&bytes);

    let decoded = decode_with_image_webp(&looped, width, height);
    let (first, second) = decoded.frames.split_at(frames.len());
    assert_close(first, &frames, &looped);
    assert_eq!(second, first, "2周目が1周目と食い違う");

    if let Some(composed) = decode_with_ffmpeg(&looped, width, height) {
        let (first, second) = composed.split_at(frames.len());
        assert_eq!(first, frames, "ffmpeg の1周目が入力と違う");
        assert_eq!(second, first, "ffmpeg の2周目が1周目と食い違う");
    }
}

/// 矩形を抜くフレームが並んでいても、2周目の合成が1周目と一致する
///
/// 折り返しの手前は矩形を抜かず、折り返しの先頭は全面をblend無しで上書きする。
/// 抜いた跡が持ち越されないことが、この2つで閉じる。
#[test]
fn a_loop_that_clears_rects_composes_the_same_on_the_second_pass() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = [(2, 2), (6, 6), (12, 10), (20, 14)]
        .map(|at| sprite_rgba(width, height, at))
        .to_vec();

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);
    assert!(
        placements(&bytes).iter().any(|frame| frame.dispose),
        "矩形を抜くフレームが1つも無い"
    );
    let looped = looped_twice(&bytes);

    let decoded = decode_with_image_webp(&looped, width, height);
    let (first, second) = decoded.frames.split_at(frames.len());
    assert_close(first, &frames, &looped);
    assert_eq!(second, first, "2周目が1周目と食い違う");

    if let Some(composed) = decode_with_ffmpeg(&looped, width, height) {
        let (first, second) = composed.split_at(frames.len());
        assert_eq!(first, frames, "ffmpeg の1周目が入力と違う");
        assert_eq!(second, first, "ffmpeg の2周目が1周目と食い違う");
    }
}

/// 離れた場所へ動く四角は、前のフレームの矩形を抜いた方が狭く収まる
///
/// 抜いた後のキャンバスは前のフレームを描いた後と矩形の中で食い違うので、
/// 置き換えの比較相手を取り違えると合成が入力へ戻らない。
#[test]
fn a_moving_sprite_is_carried_by_clearing_the_previous_rect() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = [(2, 2), (6, 6), (12, 10), (20, 14)]
        .map(|at| sprite_rgba(width, height, at))
        .to_vec();

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);

    assert_eq!(
        placements(&bytes),
        [
            Placed {
                rect: (0, 0, width, height),
                blend: false,
                dispose: true,
            },
            Placed {
                rect: (6, 6, SPRITE, SPRITE),
                blend: true,
                dispose: true,
            },
            Placed {
                rect: (12, 10, SPRITE, SPRITE),
                blend: true,
                dispose: true,
            },
            Placed {
                rect: (20, 14, SPRITE, SPRITE),
                blend: true,
                dispose: false,
            },
        ]
    );
}

/// 透過画素がキャンバスと食い違う素材は、矩形を抜かず重ねもしない
#[test]
fn a_transparent_window_over_an_opaque_background_keeps_the_canvas() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = [(3, 3), (12, 7), (20, 13)]
        .map(|at| windowed_rgba(width, height, at))
        .to_vec();

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);

    let placed = placements(&bytes);
    assert!(
        placed.iter().all(|frame| !frame.dispose),
        "抜いた方が狭くなる場面が無いのに抜いている: {placed:?}"
    );
    assert!(
        placed.iter().all(|frame| !frame.blend),
        "透過画素がキャンバスと食い違うのに重ねている: {placed:?}"
    );
}

/// 一致した画素の置き換えは、透過画素の無い素材にもαを持ち込む
///
/// 素材が持つ透過とVP8XのALPHAフラグは別物になる。
#[test]
fn substituting_transparency_raises_the_alpha_flag_of_an_opaque_material() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = [(2, 2), (10, 6), (18, 12)]
        .map(|at| patched_rgba(width, height, at))
        .to_vec();
    assert!(
        frames
            .iter()
            .all(|frame| frame.chunks_exact(4).all(|pixel| pixel[3] == 255)),
        "素材に透過画素がある"
    );

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);

    assert_eq!(bytes[VP8X_FLAGS_OFFSET], ANIMATION | ALPHA);

    let placed = placements(&bytes);
    assert!(
        placed[1..]
            .iter()
            .all(|frame| frame.blend && !frame.dispose),
        "不透明な素材が重ねる形で載っていない: {placed:?}"
    );
}

/// 前のフレームの矩形を抜いた跡そのものになるフレームは、1画素で表せる
#[test]
fn a_frame_equal_to_the_disposed_canvas_is_carried_by_a_single_pixel() {
    let (width, height) = (32, 24);
    let frames = vec![
        sprite_rgba(width, height, (2, 2)),
        sprite_rgba(width, height, (12, 10)),
        vec![0; (width * height * 4) as usize],
    ];

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);

    assert_eq!(
        placements(&bytes).last(),
        Some(&Placed {
            rect: (0, 0, 1, 1),
            blend: true,
            dispose: false,
        })
    );
}

/// ANMFの表示時間の欄に収まる上限 (ms)
const MAX_DURATION: u32 = 0x00FF_FFFF;

/// 欄に収まらない表示時間は、キャンバスを書き換えないフレームへ分けて載せる
///
/// 分けたフレームは透明1画素のblend有りなので、合成結果は分ける前と変わらない。
#[test]
fn a_duration_beyond_the_field_width_is_carried_by_extra_frames() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, 2);
    let delays = [
        FrameDelay::new(20_000, 1).unwrap(),
        FrameDelay::new(1, 25).unwrap(),
    ];

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for (frame, delay) in frames.iter().zip(delays) {
        encoder.add_frame(frame, delay).unwrap();
    }
    let bytes = encoder.finish().unwrap().0.into_inner();

    let decoded = decode_with_image_webp(&bytes, width, height);
    assert_eq!(
        decoded.durations,
        [MAX_DURATION, 20_000_000 - MAX_DURATION, 40]
    );
    assert_eq!(
        decoded.durations.iter().map(|&d| u64::from(d)).sum::<u64>(),
        20_000_040
    );

    if let Some(composed) = decode_with_ffmpeg(&bytes, width, height) {
        assert_eq!(
            composed,
            [frames[0].clone(), frames[0].clone(), frames[1].clone()]
        );
    }
}

/// 表示時間を分けたフレームは、矩形を抜く廃棄方法を載せない
///
/// 抜いた跡が分けた先のフレームの表示に見えてしまうため、抜いた方が狭く収まる
/// 素材でも候補から外れる。
#[test]
fn a_frame_split_across_durations_never_clears_its_rect() {
    let (width, height) = (32, 24);
    let frames = [(2, 2), (12, 10)].map(|at| sprite_rgba(width, height, at));
    let delays = [
        FrameDelay::new(20_000, 1).unwrap(),
        FrameDelay::new(1, 25).unwrap(),
    ];

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for (frame, delay) in frames.iter().zip(delays) {
        encoder.add_frame(frame, delay).unwrap();
    }
    let bytes = encoder.finish().unwrap().0.into_inner();

    assert_eq!(
        placements(&bytes),
        [
            Placed {
                rect: (0, 0, width, height),
                blend: false,
                dispose: false,
            },
            Placed {
                rect: (0, 0, 1, 1),
                blend: true,
                dispose: false,
            },
            Placed {
                rect: (2, 2, 16, 14),
                blend: false,
                dispose: false,
            },
        ]
    );

    let expected = [frames[0].clone(), frames[0].clone(), frames[1].clone()];
    assert_close(
        &decode_with_image_webp(&bytes, width, height).frames,
        &expected,
        &bytes,
    );
    if let Some(composed) = decode_with_ffmpeg(&bytes, width, height) {
        assert_eq!(composed, expected, "ffmpeg のデコードが入力と違う");
    }
}

/// 欄にちょうど収まる表示時間のフレームは、まだ矩形を抜ける
///
/// 分けずに済む上限がそのまま境目になる。
#[test]
fn a_frame_filling_the_duration_field_can_still_clear_its_rect() {
    let (width, height) = (32, 24);
    let frames = [(2, 2), (12, 10)].map(|at| sprite_rgba(width, height, at));
    let delays = [
        FrameDelay::new(MAX_DURATION, 1000).unwrap(),
        FrameDelay::new(1, 25).unwrap(),
    ];

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgba8, 0),
    )
    .unwrap();
    for (frame, delay) in frames.iter().zip(delays) {
        encoder.add_frame(frame, delay).unwrap();
    }
    let bytes = encoder.finish().unwrap().0.into_inner();

    let placed = placements(&bytes);
    assert_eq!(placed.len(), 2, "表示時間が分かれている: {placed:?}");
    assert!(placed[0].dispose, "上限ちょうどで抜けなくなっている");
    assert_eq!(placed[1].rect, (12, 10, SPRITE, SPRITE));
    assert_eq!(
        decode_with_image_webp(&bytes, width, height).durations,
        [MAX_DURATION, 40]
    );
    if let Some(composed) = decode_with_ffmpeg(&bytes, width, height) {
        assert_eq!(composed, frames, "ffmpeg のデコードが入力と違う");
    }
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

/// 仕掛かりの上限を `extra` フレーム超えるフレーム数
///
/// 符号化を待たせておけるのはワーカー数の2倍までで、そこを超えたぶんから
/// ANMFが流れ出す。閉じる前の書き出しを見るテストはこの数を要る。
fn frames_past_the_backlog(extra: usize) -> usize {
    available_parallelism().map_or(1, NonZeroUsize::get) * 2 + extra
}

/// 閉じていないファイルはRIFFのサイズが0のまま残り、厳密な読み手が弾く
///
/// 後埋めが書き戻すのはサイズ欄の4バイトとVP8Xのフラグの1バイトだけで、
/// 流したANMFはそのまま変わらない。保留中のフレームは書かれないため、
/// 閉じたファイルより短く終わる。
#[test]
fn a_file_that_was_never_finished_keeps_a_zero_riff_size() {
    let (width, height) = (16, 16);
    let frames = rgba_frames(width, height, frames_past_the_backlog(1));

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
    assert!(unfinished.len() > HEAD_BYTES);
    assert!(unfinished.len() < finished.len());
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
        finished[VP8X_FLAGS_OFFSET + 1..unfinished.len()]
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
    let frames = rgba_frames(width, height, frames_past_the_backlog(3));
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

/// 起こすワーカー数は、指した数と機械の並列度で決まる
///
/// 群れを起こす前に判定を済ませるため、フレームを投入しなくても数は決まる。
/// キャンバスの大きさは数に効かない。
#[test]
fn the_woken_workers_follow_the_number_that_was_asked_for() {
    let config = config(ColorType::Rgba8, 0);

    let asked = |workers| {
        Encoder::with_workers(Cursor::new(Vec::new()), 64, 64, 2, config, workers)
            .unwrap()
            .workers()
    };
    for workers in [1, 2, 5, 64] {
        let workers = NonZeroUsize::new(workers).unwrap();
        assert_eq!(asked(workers), workers, "指した数で起こしていない");
    }

    let available = available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let default = |width, height| {
        Encoder::new(Cursor::new(Vec::new()), width, height, 2, config)
            .unwrap()
            .workers()
    };
    assert_eq!(default(64, 64), available, "機械の並列度で起こしていない");
    assert_eq!(default(1920, 1080), available, "大きさで数が変わっている");
}

/// 最も重い動作点の可逆でも、合成結果が入力へバイト一致で戻る
///
/// メソッドは符号化の努力量で、可逆の契約は変えない。他の往復がメソッド4で
/// 通るので、6の経路をここで踏む。
#[test]
fn a_lossless_animation_at_the_heaviest_method_composes_back_to_the_input() {
    let (width, height) = (61, 37);
    let frames = rgba_frames(width, height, 5);

    let config = Config {
        method: 6,
        ..config(ColorType::Rgba8, 0)
    };
    let (_, report) = round_trip(width, height, config, &frames);

    assert_eq!(
        report,
        Report {
            merged_frames: 0,
            delay_clamped: false,
        }
    );
}

/// 不透明な四角が動き、途中で消える不透明でないRGBA
///
/// 消える四角の跡は前のフレームの矩形の外にも残るので、抜く範囲が据えた矩形
/// からずれると、抜いた跡と食い違ったフレームが出る。動く四角だけの素材では
/// 矩形と跡が重なってしまい、そのずれが表に出ない。
fn vanishing_rgba(width: u32, height: u32, index: usize) -> Vec<u8> {
    let ticker = (index as u32 * 2) % (width - 4);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let block = index % 4 != 3 && x.wrapping_sub(2) < 16 && y.wrapping_sub(2) < 16;
            let moving = x.wrapping_sub(ticker) < 4 && y + 4 >= height;
            rgba.extend_from_slice(&if block {
                [0xC0, 0x20, 0x20, 0xFF]
            } else if moving {
                [0x10, 0xE0, 0x10, 0xFF]
            } else {
                [0, 0, 0, 0]
            });
        }
    }
    rgba
}

/// 廃棄の跡が矩形の外へ残る素材でも、合成が入力へバイト一致で戻る
///
/// 抜く範囲は据えた矩形と同じでなければならない。広げると、廃棄を採った
/// 次のフレームが「抜いた跡」と食い違ったまま出る。
#[test]
fn a_vanishing_block_composes_back_across_the_disposal() {
    let (width, height) = (32, 24);
    let frames: Vec<Vec<u8>> = (0..12)
        .map(|index| vanishing_rgba(width, height, index))
        .collect();

    let (bytes, _) = round_trip(width, height, config(ColorType::Rgba8, 0), &frames);

    assert!(
        placements(&bytes).iter().any(|frame| frame.dispose),
        "矩形を抜く廃棄方法が一度も選ばれていない"
    );
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

/// 非可逆の合成を入力と突き合わせる
///
/// 可逆と違ってバイト一致は取れないので、`assert_close` の絞りは使えない。
/// 代わりに次の2つで縛る。
///
/// - **αは厳密**。非可逆でもαの面は可逆で格納される (`alpha_compression` の
///   既定) ので、ここに量子化誤差は乗らない
/// - RGBは**フレームごと**の平均絶対誤差で縛る。列全体で平均すると、1枚だけが
///   崩れた出力が他の枚数に埋もれる
///
/// `limit` は HEAD の実測から採る。呼び出し側がその実測値を持つ。
fn assert_lossy_close(decoded: &[Vec<u8>], expected: &[Vec<u8>], limit: f64) {
    /// RGBAのαが並ぶ位置
    const ALPHA_AT: usize = 3;

    assert_eq!(decoded.len(), expected.len(), "返ったフレーム数");
    for (index, (decoded, expected)) in decoded.iter().zip(expected).enumerate() {
        assert_eq!(decoded.len(), expected.len(), "フレーム{index}の長さ");

        let alpha = decoded.iter().skip(ALPHA_AT).step_by(4);
        let want = expected.iter().skip(ALPHA_AT).step_by(4);
        let (pixels, moved, worst) = alpha.zip(want).fold(
            (0usize, 0usize, 0u8),
            |(pixels, moved, worst), (alpha, want)| {
                (
                    pixels + 1,
                    moved + usize::from(alpha != want),
                    worst.max(alpha.abs_diff(*want)),
                )
            },
        );
        assert_eq!(
            moved,
            0,
            "フレーム{index}のαが {moved}/{pixels} 画素 ({:.1}%) ずれている。最大 {worst}",
            moved as f64 * 100.0 / pixels as f64
        );

        let error = mean_abs_error(decoded, expected);
        assert!(error < limit, "フレーム{index}の平均絶対誤差 {error}");
    }
}

/// 非可逆の設定
///
/// `plugins/webp` の既定は非可逆で、そちらが出荷時に通る経路になる。
fn lossy_config(color_type: ColorType) -> Config {
    Config {
        color_type,
        lossless: false,
        quality: 90.0,
        method: 4,
        num_plays: 0,
    }
}

/// 非可逆でも、フレームごとの合成が入力の近くへ戻る
///
/// 素材は不透明なので 5.3 の透過置換が blend 有りのフレームで意味を持つ形だが、
/// 置き換えた完全透過は非可逆の符号化を素通りしないため、非可逆では行わない。
/// 素材にαが無い以上、VP8XのALPHAも立ってはいけない。
///
/// 誤差の上限 3.0 は HEAD の実測から採った。フレームごとの平均絶対誤差の最大は
/// `image-webp` で 1.39、ffmpeg で 2.09。**非可逆では ffmpeg もバイト一致では
/// 戻らない** — VP8 の復号は同じでも、YUVからRGBへの変換がデコーダごとに違う。
/// 可逆で ffmpeg が担保するバイト一致は、この経路には無い。
#[test]
fn a_lossy_animation_composes_near_the_input() {
    let (width, height) = (48, 32);
    let frames: Vec<Vec<u8>> = (0..6)
        .map(|index| patched_rgba(width, height, (index * 4, index * 3)))
        .collect();
    assert!(
        frames
            .iter()
            .all(|frame| frame.chunks_exact(4).all(|p| p[3] == 255)),
        "素材が透過を含んでいる"
    );

    let (bytes, _) = encode(width, height, lossy_config(ColorType::Rgba8), &frames).unwrap();

    assert_eq!(
        bytes[VP8X_FLAGS_OFFSET], ANIMATION,
        "透過の無い素材でALPHAが立っている"
    );
    assert!(
        placements(&bytes).iter().skip(1).any(|frame| frame.blend),
        "重ねる形のフレームが1枚も無い"
    );

    assert_lossy_close(
        &decode_with_image_webp(&bytes, width, height).frames,
        &frames,
        3.0,
    );
    if let Some(composed) = decode_with_ffmpeg(&bytes, width, height) {
        assert_lossy_close(&composed, &frames, 3.0);
    }
}

/// 決定的な擬似乱数で埋めた不透明なRGBA
fn noisy_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = noise((width * height * 4) as usize, 0x5EED);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    rgba
}

/// ひとつの値で埋めた不透明なRGBA
fn flat_rgba(width: u32, height: u32, level: u8) -> Vec<u8> {
    (0..width * height)
        .flat_map(|_| [level, level, level, 255])
        .collect()
}

/// 入力が変わらなくても、画面が許容量を超えて離れていれば書き直す
///
/// 24枚とも同じ入力で、離れているのは量子化の誤差を負った画面だけ。決定は復号の
/// 届いたフレームを相手にするので書き直しはその遅れのぶん後になり、入力が変わらない
/// 限り1回で止まる。残りは表示時間へ畳まれ、総再生時間は保たれる。
#[test]
fn an_unchanged_input_is_rewritten_once_while_the_screen_stays_apart() {
    let (width, height) = (48, 32);
    const FRAMES: usize = 24;
    let frames = vec![noisy_rgba(width, height); FRAMES];

    let (bytes, report) = encode(width, height, lossy_config(ColorType::Rgba8), &frames).unwrap();

    assert_eq!(placements(&bytes).len(), 2, "画面と比べていない");
    assert_eq!(
        report.merged_frames as usize,
        FRAMES - 2,
        "書き直しが1回で止まっていない"
    );

    let durations = decode_with_image_webp(&bytes, width, height).durations;
    assert_eq!(durations.len(), 2, "畳んだフレーム数");
    let total: u32 = (0..FRAMES).map(|index| index as u32 * 7 + 20).sum();
    assert_eq!(durations.iter().sum::<u32>(), total, "畳んだ表示時間の合計");
}

/// 1刻みの変化でも矩形を立てる
///
/// 刻みが1でも40でも同じだけ書く。書き直すかどうかは変化の大きさではなく、
/// 画面が入力に届いているかで決まる。
#[test]
fn a_single_step_of_change_stands_a_rect() {
    /// 平らな面の高さ
    const LEVEL: u8 = 0x60;

    let (width, height) = (48, 32);
    for step in [1u8, 40] {
        let frames = vec![
            flat_rgba(width, height, LEVEL),
            flat_rgba(width, height, LEVEL + step),
        ];
        let (bytes, report) =
            encode(width, height, lossy_config(ColorType::Rgba8), &frames).unwrap();

        assert_eq!(placements(&bytes).len(), frames.len(), "刻み {step}");
        assert_eq!(report.merged_frames, 0, "刻み {step}");
        assert_eq!(
            decode_with_image_webp(&bytes, width, height)
                .durations
                .iter()
                .sum::<u32>(),
            47,
            "刻み {step} の総再生時間"
        );
    }
}

/// 先頭フレームは、書き直す画素がどれだけ少なくても全面で書く
///
/// 透過の面に不透明な四角を置いた素材は、まだ何も描いていないキャンバスとの
/// 差が四角だけになる。
#[test]
fn the_first_lossy_frame_covers_the_whole_canvas() {
    let (width, height) = (48, 32);
    let frames = vec![
        sprite_rgba(width, height, (2, 2)),
        sprite_rgba(width, height, (20, 14)),
    ];

    let (bytes, _) = encode(width, height, lossy_config(ColorType::Rgba8), &frames).unwrap();

    let placed = placements(&bytes);
    assert_eq!(placed[0].rect, (0, 0, width, height));
    assert!(!placed[0].blend, "先頭フレームが重ねる形になっている");
}

/// 半透明の背景に置いた四角の一辺の長さ
const PANEL_BLOCK: u32 = 6;

/// 半透明の平らな背景に、離れた2つの不透明な四角を置いたRGBA
///
/// 四角だけが `index` で明るさを変える。2つを囲む矩形が背景を広く巻き込むので、
/// 変わらない半透明の画素が毎フレーム矩形の中へ入る。**無彩色で作る** — 色差を
/// 持つ背景は 4:2:0 を通って復号結果が入力と食い違い、キャンバスと一致する
/// 画素が矩形の中に現れなくなる。
fn translucent_panel(width: u32, height: u32, index: usize) -> Vec<u8> {
    const BACKGROUND: [u8; 4] = [0x80, 0x80, 0x80, 0x80];
    let blocks = [(2, 2), (width - PANEL_BLOCK - 2, height - PANEL_BLOCK - 2)];

    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let inside = blocks
                .iter()
                .any(|at| x.wrapping_sub(at.0) < PANEL_BLOCK && y.wrapping_sub(at.1) < PANEL_BLOCK);
            let level = (index * 32) as u8;
            rgba.extend_from_slice(&if inside {
                [level, level, level, 0xFF]
            } else {
                BACKGROUND
            });
        }
    }
    rgba
}

/// 矩形が巻き込んだ半透明の未変更画素は、画面のαを動かさない
///
/// 重ねる形で載せると合成のたびにαが不透明へ寄り、画面が入力から離れる。
#[test]
fn a_translucent_background_swept_into_a_rect_keeps_its_alpha() {
    let (width, height) = (48, 32);
    let frames: Vec<Vec<u8>> = (0..8)
        .map(|index| translucent_panel(width, height, index))
        .collect();

    let (bytes, _) = encode(width, height, lossy_config(ColorType::Rgba8), &frames).unwrap();

    // 巻き込む画素があることを、2つの四角より広い矩形が並ぶことで押さえる
    let swept = placements(&bytes)
        .iter()
        .skip(1)
        .filter(|frame| frame.rect.2 * frame.rect.3 > 2 * PANEL_BLOCK * PANEL_BLOCK)
        .count();
    assert_eq!(swept, frames.len() - 1, "背景を巻き込む矩形が並んでいない");

    assert_lossy_close(
        &decode_with_image_webp(&bytes, width, height).frames,
        &frames,
        3.0,
    );
    if let Some(composed) = decode_with_ffmpeg(&bytes, width, height) {
        assert_lossy_close(&composed, &frames, 3.0);
    }
}

/// 表示時間を分けたフレームのαも、VP8XのALPHAへ数える
///
/// 分けた先は透明1画素のフレームなので、素材そのものに透過が無くても
/// ファイルはαを持つ。数え落とすとALPHAが立たず、仕様に反する。
#[test]
fn the_frames_that_split_a_duration_raise_the_alpha_flag() {
    let (width, height) = (16, 16);
    let frames = rgb_frames(width, height, 2);
    let delays = [
        FrameDelay::new(20_000, 1).unwrap(),
        FrameDelay::new(1, 25).unwrap(),
    ];

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        frames.len() as u32,
        config(ColorType::Rgb8, 0),
    )
    .unwrap();
    for (frame, delay) in frames.iter().zip(delays) {
        encoder.add_frame(frame, delay).unwrap();
    }
    let bytes = encoder.finish().unwrap().0.into_inner();

    assert_eq!(placements(&bytes).len(), 3, "表示時間が分かれていない");
    assert_eq!(bytes[VP8X_FLAGS_OFFSET], ANIMATION | ALPHA);
}
