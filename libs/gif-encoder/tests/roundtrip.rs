//! 出力したGIFをデコードし、フレームごとの合成結果が正規化した入力と画素単位で
//! 一致することを確認する
//!
//! LZWの誤りは一部のデコーダだけが読めるファイルを作るため、`gif` クレートと
//! ffmpeg の2つでデコードする。ffmpeg が見つからない環境では、そちらだけを
//! 飛ばして `gif` クレートの結果で判定する。

use gif_encoder::{
    ColorType, Config, DEFAULT_MAX_SPOOL_BYTES, Encoder, Error, FrameDelay, PaletteKind, Report,
};
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

/// 投入する順に異なる遅延
///
/// 分母を100にすると丸めが恒等になり、期待する1/100秒がそのまま添字から決まる。
fn delay_of(index: usize) -> FrameDelay {
    FrameDelay::new(index as u32 + 2, 100).unwrap()
}

fn encode(
    width: u32,
    height: u32,
    color_type: ColorType,
    frames: &[Vec<u8>],
    num_plays: u32,
) -> Result<(Vec<u8>, Report), Error> {
    let config = Config {
        color_type,
        num_plays,
        ..Config::default()
    };
    encode_with(width, height, config, frames)
}

/// 設定を指定してフレーム列を符号化する
fn encode_with(
    width: u32,
    height: u32,
    config: Config,
    frames: &[Vec<u8>],
) -> Result<(Vec<u8>, Report), Error> {
    let mut encoder = Encoder::new(Vec::new(), width, height, frames.len() as u32, config)?;
    for (index, data) in frames.iter().enumerate() {
        encoder.add_frame(data, delay_of(index))?;
    }
    encoder.finish()
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
    palette: Option<Vec<u8>>,
    frames: Vec<DecodedFrame>,
}

struct DecodedFrame {
    rgba: Vec<u8>,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    delay: u16,
    dispose: gif::DisposalMethod,
    transparent: Option<u8>,
}

impl DecodedFrame {
    /// 画像記述子が示す矩形
    fn rect(&self) -> (u16, u16, u16, u16) {
        (self.left, self.top, self.width, self.height)
    }
}

fn decode_with_gif(bytes: &[u8]) -> Decoded {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(bytes).unwrap();

    let width = decoder.width();
    let height = decoder.height();
    let repeat = decoder.repeat();
    let palette = decoder.global_palette().map(<[u8]>::to_vec);

    let mut frames = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        frames.push(DecodedFrame {
            rgba: frame.buffer.to_vec(),
            left: frame.left,
            top: frame.top,
            width: frame.width,
            height: frame.height,
            delay: frame.delay,
            dispose: frame.dispose,
            transparent: frame.transparent,
        });
    }

    Decoded {
        width,
        height,
        repeat,
        palette,
        frames,
    }
}

/// デコードしたフレームを廃棄方法に従って合成し、フレームごとの画面を返す
///
/// 透過インデックスに当たった画素はキャンバスを書き換えない。デコーダは
/// その画素をアルファ0で返すので、アルファを持つ画素だけを写す。
///
/// 廃棄はフレームを表示した後に効く。Background は仕様上「背景色で塗り直す」
/// だが、現代のデコーダは透過で抜くため、そちらに合わせる。
fn compose(decoded: &Decoded) -> Vec<Vec<u8>> {
    let stride = usize::from(decoded.width) * 4;
    let mut canvas = vec![0u8; stride * usize::from(decoded.height)];
    let mut screens = Vec::new();

    for frame in &decoded.frames {
        let before = canvas.clone();
        for y in 0..usize::from(frame.height) {
            for x in 0..usize::from(frame.width) {
                let at = (y * usize::from(frame.width) + x) * 4;
                let pixel = &frame.rgba[at..at + 4];
                if pixel[3] == 0 {
                    continue;
                }
                let to = (y + usize::from(frame.top)) * stride + (x + usize::from(frame.left)) * 4;
                canvas[to..to + 4].copy_from_slice(pixel);
            }
        }
        screens.push(canvas.clone());

        match frame.dispose {
            gif::DisposalMethod::Keep => {}
            gif::DisposalMethod::Background => {
                for y in 0..usize::from(frame.height) {
                    let row = (y + usize::from(frame.top)) * stride + usize::from(frame.left) * 4;
                    canvas[row..row + usize::from(frame.width) * 4].fill(0);
                }
            }
            gif::DisposalMethod::Previous => canvas = before,
            gif::DisposalMethod::Any => panic!("扱えない廃棄方法が出た"),
        }
    }
    screens
}

/// ffmpeg で合成済みのフレームへデコードした生RGBA。ffmpeg が無ければ `None`
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

/// 出力を2つのデコーダへ通し、フレームごとの合成結果が入力と一致することを確かめる
fn round_trip(
    width: u32,
    height: u32,
    color_type: ColorType,
    frames: &[Vec<u8>],
) -> (Vec<u8>, Report) {
    round_trip_within(width, height, color_type, frames, 0)
}

/// 出力を2つのデコーダへ通し、合成結果が入力から `tolerance` 以内であることを
/// 確かめる
///
/// 量子化の経路では入力と一致しない。構造 (寸法・フレーム数・遅延・透過の位置)
/// はそのまま確かめ、色だけをチャネルあたりの最大誤差で見る。
fn round_trip_within(
    width: u32,
    height: u32,
    color_type: ColorType,
    frames: &[Vec<u8>],
    tolerance: u8,
) -> (Vec<u8>, Report) {
    let config = Config {
        color_type,
        ..Config::default()
    };
    round_trip_config(width, height, config, frames, tolerance)
}

/// 設定を指定して往復させる
fn round_trip_config(
    width: u32,
    height: u32,
    config: Config,
    frames: &[Vec<u8>],
    tolerance: u8,
) -> (Vec<u8>, Report) {
    let color_type = config.color_type;
    let (bytes, report) = encode_with(width, height, config, frames).unwrap();
    if matches!(report.palette, PaletteKind::Exact { .. }) {
        assert_eq!(
            report.approximated_pixels, 0,
            "可逆の経路で近似した画素がある"
        );
    }
    let expected: Vec<Vec<u8>> = frames
        .iter()
        .map(|data| expected_rgba(data, color_type))
        .collect();

    // 溜めきれる素材では和集合が全フレームを覆うので、写す先の黒を足すのは
    // 素材のどこにも不透明な画素が無いときに限る
    let opaque = expected
        .iter()
        .flat_map(|frame| frame.chunks_exact(4))
        .any(|pixel| pixel[3] != 0);
    assert_eq!(report.black_fallback, !opaque, "写す先の黒の足し方が違う");

    let decoded = decode_with_gif(&bytes);
    assert_eq!(
        (u32::from(decoded.width), u32::from(decoded.height)),
        (width, height),
        "論理画面の寸法が違う"
    );
    assert_eq!(decoded.frames.len(), frames.len(), "フレーム数が違う");
    for (index, frame) in decoded.frames.iter().enumerate() {
        assert_eq!(
            frame.delay,
            index as u16 + 2,
            "{index} 番目のフレームの遅延が違う"
        );
    }

    for (index, (actual, expected)) in compose(&decoded).iter().zip(&expected).enumerate() {
        assert_close(
            actual,
            expected,
            tolerance,
            &format!("`gif` クレートの {index} 番目"),
        );
    }

    if let Some(raw) = decode_with_ffmpeg(&bytes) {
        let frame_len = width as usize * height as usize * 4;
        assert_eq!(
            raw.len(),
            frame_len * frames.len(),
            "ffmpeg が返したフレーム数が違う"
        );
        for (index, (actual, expected)) in raw.chunks_exact(frame_len).zip(&expected).enumerate() {
            assert_close(
                actual,
                expected,
                tolerance,
                &format!("ffmpeg の {index} 番目"),
            );
        }
    }

    (bytes, report)
}

/// 合成結果が入力から `tolerance` 以内であることを確かめる
///
/// 透過の位置に許容は無い。2値なので近いも遠いも無く、食い違えば別の絵になる。
fn assert_close(actual: &[u8], expected: &[u8], tolerance: u8, decoder: &str) {
    assert_transparency(actual, expected, decoder);
    if tolerance == 0 {
        assert_eq!(actual, expected, "{decoder} の合成結果が違う");
        return;
    }

    for (at, (actual, expected)) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        if expected[3] == 0 {
            continue;
        }
        for channel in 0..3 {
            let error = actual[channel].abs_diff(expected[channel]);
            assert!(
                error <= tolerance,
                "{decoder} の {at} 画素目のずれが大きい: {error} > {tolerance}"
            );
        }
    }
}

/// 透過画素の位置が正規化した入力と一致する
///
/// デコーダは透過インデックスに当たった画素だけをアルファ0で返す。色ではなく
/// 位置だけを見るので、透過インデックスが別のエントリを指す誤りは、
/// カラーテーブルの色の一致とは独立に落ちる。
fn assert_transparency(actual: &[u8], expected: &[u8], decoder: &str) {
    let positions = |rgba: &[u8]| -> Vec<usize> {
        rgba.chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[3] == 0)
            .map(|(at, _)| at)
            .collect()
    };
    assert_eq!(
        positions(actual),
        positions(expected),
        "{decoder} で透過画素の位置が違う"
    );
}

/// 6-6-6のビンが1つずつ違うRGBA8の画素
fn distinct_bin(index: u32) -> [u8; 4] {
    [(index % 64 * 4) as u8, (index / 64 * 4) as u8, 0, u8::MAX]
}

/// 一様な色で埋めたフレーム
fn solid(width: u32, height: u32, color_type: ColorType, pixel: &[u8]) -> Vec<u8> {
    assert_eq!(pixel.len(), color_type.bytes_per_pixel());
    pixel.repeat(width as usize * height as usize)
}

/// 指定した画素を書き換える
fn set_pixel(frame: &mut [u8], width: u32, color_type: ColorType, x: u32, y: u32, pixel: &[u8]) {
    let bpp = color_type.bytes_per_pixel();
    let at = (y as usize * width as usize + x as usize) * bpp;
    frame[at..at + bpp].copy_from_slice(pixel);
}

/// 各フレームの画像記述子が示す矩形
fn rects(bytes: &[u8]) -> Vec<(u16, u16, u16, u16)> {
    decode_with_gif(bytes)
        .frames
        .iter()
        .map(DecodedFrame::rect)
        .collect()
}

/// 各フレームのグラフィック制御拡張が示す廃棄方法
fn disposals(bytes: &[u8]) -> Vec<gif::DisposalMethod> {
    decode_with_gif(bytes)
        .frames
        .iter()
        .map(|frame| frame.dispose)
        .collect()
}

/// 1画素だけの画像
#[test]
fn a_single_pixel_frame_survives_both_decoders() {
    round_trip(1, 1, ColorType::Rgb8, &[vec![0x12, 0x34, 0x56]]);
    round_trip(1, 1, ColorType::Rgba8, &[vec![0x12, 0x34, 0x56, 0xFF]]);
    round_trip(1, 1, ColorType::Rgba8, &[vec![0x12, 0x34, 0x56, 0x00]]);
}

/// 全画素が同じ色
#[test]
fn a_uniform_frame_survives_both_decoders() {
    let data: Vec<u8> = [0x20, 0x40, 0x60].repeat(64 * 64);
    round_trip(64, 64, ColorType::Rgb8, &[data]);
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
        round_trip(32, 32, ColorType::Rgb8, &[data]);
    }
}

/// 色の和集合がちょうど256色 (透過標識を含まない)
#[test]
fn a_frame_with_exactly_256_opaque_colors_survives_both_decoders() {
    let data: Vec<u8> = (0..16 * 16).flat_map(|i| [i as u8, 0, 0, 0xFF]).collect();
    let (bytes, report) = round_trip(16, 16, ColorType::Rgba8, &[data]);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 256 });

    // 透過標識が和集合に無いため、透過インデックスは置かない
    let decoded = decode_with_gif(&bytes);
    assert_eq!(decoded.frames[0].transparent, None);
    assert!(decoded.frames[0].rgba.chunks_exact(4).all(|p| p[3] == 255));
}

/// 素材自身の透過画素が透過インデックスになる
///
/// 先頭画素を不透明にして、透過標識が添字0以外のエントリへ落ちる場合を踏む。
#[test]
fn a_frame_with_transparent_pixels_survives_both_decoders() {
    let data: Vec<u8> = (0..255 * 4)
        .flat_map(|i| {
            if i % 3 == 1 {
                [9, 9, 9, 0]
            } else {
                [(i % 255) as u8, 0x80, 0x40, 0xFF]
            }
        })
        .collect();
    let (bytes, _) = round_trip(51, 20, ColorType::Rgba8, &[data]);

    let decoded = decode_with_gif(&bytes);
    let transparent = decoded.frames[0]
        .transparent
        .expect("透過インデックスが無い");
    assert_ne!(
        transparent, 0,
        "透過標識が添字0へ落ちている素材になっている"
    );

    let palette = decoded.palette.expect("グローバルカラーテーブルが無い");
    let at = usize::from(transparent) * 3;
    assert_eq!(
        &palette[at..at + 3],
        [0, 0, 0],
        "透過インデックスが標識のエントリを指していない"
    );
}

/// 閾値未満のアルファは完全透過へ潰れ、潰した画素数がレポートに載る
#[test]
fn partial_alpha_is_binarized_before_encoding() {
    let data: Vec<u8> = (0..64 * 8)
        .flat_map(|i| [(i % 200) as u8, 0x10, 0x20, (i % 256) as u8])
        .collect();
    let squashed = data.chunks_exact(4).filter(|p| p[3] < 128).count() as u64;
    assert!(squashed > 0, "潰れる画素が無い素材になっている");

    let (_, report) = round_trip(64, 8, ColorType::Rgba8, &[data]);
    assert_eq!(report.binarized_pixels, squashed);
}

/// 縦横が異なる矩形
#[test]
fn a_non_square_frame_survives_both_decoders() {
    let data = noise(17 * 5 * 3, 11)
        .iter()
        .map(|&byte| byte & 0x0F)
        .collect::<Vec<u8>>();
    round_trip(17, 5, ColorType::Rgb8, &[data]);
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
    round_trip(512, 512, ColorType::Rgb8, &[data]);
}

/// 複数フレームがグローバルカラーテーブル1枚で可逆に出る
#[test]
fn multiple_frames_share_one_exact_color_table() {
    const WIDTH: u32 = 24;
    const HEIGHT: u32 = 16;

    let frames: Vec<Vec<u8>> = (0..6)
        .map(|seed| {
            noise((WIDTH * HEIGHT) as usize * 3, seed + 1)
                .iter()
                .map(|&byte| byte & 0x03)
                .collect()
        })
        .collect();

    let (_, report) = round_trip(WIDTH, HEIGHT, ColorType::Rgb8, &frames);
    let PaletteKind::Exact { colors } = report.palette else {
        panic!("全フレームを見て据えていない: {:?}", report.palette)
    };
    assert!(colors <= 64, "和集合が {colors} 色まで広がっている");
}

/// 動く画素の差分矩形だけが書かれ、先頭フレームは全画面になる
#[test]
fn later_frames_are_written_as_difference_rects() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 6;
    let color = ColorType::Rgb8;

    let background = solid(WIDTH, HEIGHT, color, &[0x10, 0x20, 0x30]);
    let mut frames = vec![background.clone()];
    for (x, y) in [(3u32, 2u32), (5, 4), (0, 0), (7, 5)] {
        let mut frame = background.clone();
        set_pixel(&mut frame, WIDTH, color, x, y, &[0xF0, 0xE0, 0xD0]);
        frames.push(frame);
    }

    let (bytes, _) = round_trip(WIDTH, HEIGHT, color, &frames);
    assert_eq!(
        rects(&bytes),
        [
            (0, 0, WIDTH as u16, HEIGHT as u16),
            (3, 2, 1, 1),
            (3, 2, 3, 3),
            (0, 0, 6, 5),
            (0, 0, 8, 6),
        ]
    );
}

/// 差分の無いフレームは1画素の矩形になり、フレーム数と遅延はそのまま保たれる
#[test]
fn identical_frames_are_written_as_a_unit_rect() {
    const WIDTH: u32 = 5;
    const HEIGHT: u32 = 4;
    let color = ColorType::Rgb8;

    let frame = solid(WIDTH, HEIGHT, color, &[0x40, 0x50, 0x60]);
    let frames = vec![frame.clone(), frame.clone(), frame.clone(), frame];

    let (bytes, _) = round_trip(WIDTH, HEIGHT, color, &frames);
    assert_eq!(
        rects(&bytes),
        [
            (0, 0, WIDTH as u16, HEIGHT as u16),
            (0, 0, 1, 1),
            (0, 0, 1, 1),
            (0, 0, 1, 1),
        ]
    );
}

/// 差分矩形の中で変わっていない画素は透過インデックスで埋まる
///
/// 離れた2画素を変えると、外接矩形はその間の変わっていない画素を含む。
#[test]
fn unchanged_pixels_inside_the_rect_are_written_as_transparent() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 6;
    let color = ColorType::Rgb8;

    let first = solid(WIDTH, HEIGHT, color, &[0x10, 0x20, 0x30]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0xF0, 0xE0, 0xD0]);
    set_pixel(&mut second, WIDTH, color, 5, 4, &[0xD0, 0xE0, 0xF0]);

    let (bytes, _) = round_trip(WIDTH, HEIGHT, color, &[first, second]);
    let decoded = decode_with_gif(&bytes);
    assert_eq!(decoded.frames[1].rect(), (1, 1, 5, 4));

    // 矩形の中で透過になった画素は、変えた2画素を除いた全部
    let opaque: Vec<usize> = decoded.frames[1]
        .rgba
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[3] != 0)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        opaque,
        [0, 5 * 3 + 4],
        "透過ランが変わった画素まで覆っている"
    );
}

/// 和集合がちょうど256色の経路では、透過ランを諦めて差分矩形だけで書く
#[test]
fn a_full_opaque_union_writes_every_pixel_of_the_rect() {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 16;
    let color = ColorType::Rgb8;

    let first: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [i as u8, 0x40, 0x80])
        .collect();
    let second: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [(255 - i) as u8, 0x40, 0x80])
        .collect();
    // 差分の無いフレームを挟み、透過インデックスを持たないまま1画素を書く経路を踏む
    let frames = vec![first.clone(), first, second];

    let (bytes, report) = round_trip(WIDTH, HEIGHT, color, &frames);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 256 });

    let decoded = decode_with_gif(&bytes);
    assert_eq!(decoded.frames[1].rect(), (0, 0, 1, 1));
    for (index, frame) in decoded.frames.iter().enumerate() {
        assert_eq!(
            frame.transparent, None,
            "{index} 番目に透過インデックスが出た"
        );
        assert!(
            frame.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
            "{index} 番目に透過画素が出た"
        );
    }
}

/// 素材自身の透過画素と未変更画素のランは同じエントリを使う
#[test]
fn the_marker_entry_carries_both_kinds_of_transparency() {
    const WIDTH: u32 = 6;
    const HEIGHT: u32 = 4;
    let color = ColorType::Rgba8;

    let mut first = solid(WIDTH, HEIGHT, color, &[0x20, 0x40, 0x60, 0xFF]);
    set_pixel(&mut first, WIDTH, color, 0, 0, &[0, 0, 0, 0]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0x11, 0x22, 0x33, 0xFF]);
    set_pixel(&mut second, WIDTH, color, 4, 3, &[0x44, 0x55, 0x66, 0xFF]);

    let (bytes, _) = round_trip(WIDTH, HEIGHT, color, &[first, second]);
    let decoded = decode_with_gif(&bytes);

    // 標識のエントリが両方のフレームの透過インデックスになっている
    let palette = decoded.palette.expect("グローバルカラーテーブルが無い");
    let marker = palette
        .chunks_exact(3)
        .position(|entry| entry == [0, 0, 0])
        .expect("標識のエントリが無い");
    for (index, frame) in decoded.frames.iter().enumerate() {
        assert_eq!(
            frame.transparent,
            Some(marker as u8),
            "{index} 番目の透過インデックスが標識を指していない"
        );
    }

    // 2枚目の矩形は変わっていない画素で埋まり、透過で書かれている
    assert_eq!(decoded.frames[1].rect(), (1, 1, 4, 3));
    let opaque = decoded.frames[1]
        .rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[3] != 0)
        .count();
    assert_eq!(opaque, 2, "透過ランが変わった画素まで覆っている");
}

/// 瞬き (A→B→A) で戻った画素が、キャンバスの取り違えで潰れないこと
///
/// 常に変わり続ける画素を端に置いて矩形を広げ、瞬く画素を矩形の中へ入れる。
#[test]
fn a_blinking_pixel_returns_to_its_first_color() {
    const WIDTH: u32 = 6;
    const HEIGHT: u32 = 3;
    let color = ColorType::Rgb8;

    let background = solid(WIDTH, HEIGHT, color, &[0x11, 0x22, 0x33]);
    let blink = [[0xA0u8, 0xB0, 0xC0], [0x0A, 0x0B, 0x0C]];
    let frames: Vec<Vec<u8>> = (0..6)
        .map(|index| {
            let mut frame = background.clone();
            set_pixel(&mut frame, WIDTH, color, 1, 1, &blink[index % 2]);
            set_pixel(
                &mut frame,
                WIDTH,
                color,
                WIDTH - 1,
                HEIGHT - 1,
                &[index as u8, 0x77, 0x88],
            );
            frame
        })
        .collect();

    round_trip(WIDTH, HEIGHT, color, &frames);
}

/// 透過画素が増えていく素材は、キャンバスを残したまま表現できる
#[test]
fn frames_that_only_add_paint_survive_both_decoders() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    let color = ColorType::Rgba8;

    let mut frame = solid(WIDTH, HEIGHT, color, &[0, 0, 0, 0]);
    let mut frames = vec![frame.clone()];
    for index in 0..(WIDTH * HEIGHT) {
        let (x, y) = (index % WIDTH, index / WIDTH);
        set_pixel(
            &mut frame,
            WIDTH,
            color,
            x,
            y,
            &[index as u8, 0x30, 0x60, 0xFF],
        );
        frames.push(frame.clone());
    }

    round_trip(WIDTH, HEIGHT, color, &frames);
}

/// 不透明な画素が透過になる遷移は、保留中の矩形を透過へ抜いて表現する
///
/// 透過インデックスはキャンバスを書き換えないため、キャンバスを残す廃棄方法では
/// 抜けない。抜きたい画素が保留中のフレームの矩形の中にあるので、その矩形を
/// 丸ごと抜く廃棄方法で足りる。
#[test]
fn an_opaque_pixel_turning_transparent_clears_the_pending_rect() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgba8;

    let first = solid(WIDTH, HEIGHT, color, &[0x20, 0x40, 0x60, 0xFF]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0x21, 0x41, 0x61, 0xFF]);
    let mut third = second.clone();
    set_pixel(&mut third, WIDTH, color, 1, 1, &[0, 0, 0, 0]);

    let (bytes, _) = round_trip(WIDTH, HEIGHT, color, &[first, second, third]);
    assert_eq!(
        disposals(&bytes),
        [
            gif::DisposalMethod::Keep,
            gif::DisposalMethod::Background,
            gif::DisposalMethod::Keep,
        ]
    );
    assert_eq!(rects(&bytes)[1], (1, 1, 1, 1), "抜く矩形が広すぎる");
}

/// 不透明領域と透過領域にまたがるパネルが現れて消える素材
///
/// パネルが消えるフレームでは、パネルが不透明にした画素を抜くことになる。
/// 抜いた先を書き直す範囲は、矩形を丸ごと抜くより描く直前へ戻す方が狭い。
fn panel_over_an_edge() -> Vec<Vec<u8>> {
    const WIDTH: u32 = 16;
    let color = ColorType::Rgba8;

    // 左半分は画素ごとに違う色。書き直しの高くつく相手にする
    let mut base = solid(WIDTH, 8, color, &[0, 0, 0, 0]);
    for y in 0..8 {
        for x in 0..8 {
            let value = (y * 8 + x) as u8;
            set_pixel(&mut base, WIDTH, color, x, y, &[value, 0x40, 0x60, 0xFF]);
        }
    }

    let mut panel = base.clone();
    for y in 2..6 {
        for x in 4..12 {
            set_pixel(&mut panel, WIDTH, color, x, y, &[0x90, 0x30, 0x10, 0xFF]);
        }
    }

    vec![
        base.clone(),
        panel.clone(),
        base.clone(),
        panel,
        base.clone(),
    ]
}

/// 抜く候補が2つ立ったら、圧縮後の小さい方を採る
///
/// 描く直前へ戻すと画面が投入されたフレームと一致し、書き直す画素が無くなる。
/// 矩形を丸ごと抜く方は、抜いた画素を色ごと書き直すことになる。
#[test]
fn the_smaller_of_the_two_clearing_candidates_wins() {
    let frames = panel_over_an_edge();
    let (bytes, report) = round_trip(16, 8, ColorType::Rgba8, &frames);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 66 });

    use gif::DisposalMethod::{Keep, Previous};
    assert_eq!(
        disposals(&bytes),
        [Keep, Previous, Keep, Previous, Keep],
        "描く直前へ戻す候補が採られていない"
    );
    assert_eq!(
        rects(&bytes)[2],
        (0, 0, 1, 1),
        "戻した画面との差分が残っている"
    );
}

/// 抜いた矩形の中に、投入されたフレームが同じ色のまま残す不透明画素がある素材
///
/// その画素は抜かれた画面では書き直しの対象になる。抜いた画面を作らずに
/// 保留中のフレームを描いた後の画面と比べると「未変更」と読めてしまい、
/// 透過インデックスを書いて画面から消える。
fn a_kept_color_inside_the_cleared_rect() -> Vec<Vec<u8>> {
    const WIDTH: u32 = 8;
    let color = ColorType::Rgba8;
    let base = [0x10, 0x20, 0x30, 0xFF];
    let painted = [0x40, 0x50, 0x60, 0xFF];

    let first = solid(WIDTH, 2, color, &base);
    let mut second = first.clone();
    for x in 1..5 {
        set_pixel(&mut second, WIDTH, color, x, 0, &painted);
    }

    // 塗った画素のうち x=1 だけを残し、矩形の外の x=6 を透過にして矩形を広げさせる
    let mut third = second.clone();
    for x in 2..5 {
        set_pixel(&mut third, WIDTH, color, x, 0, &base);
    }
    set_pixel(&mut third, WIDTH, color, 6, 0, &[0, 0, 0, 0]);

    vec![first, second, third]
}

/// 抜いた矩形の中の画素は、色が変わっていなくても書き直される
#[test]
fn a_color_kept_across_a_cleared_rect_is_written_again() {
    let frames = a_kept_color_inside_the_cleared_rect();
    let (bytes, _) = round_trip(8, 2, ColorType::Rgba8, &frames);

    assert_eq!(
        disposals(&bytes),
        [
            gif::DisposalMethod::Keep,
            gif::DisposalMethod::Background,
            gif::DisposalMethod::Keep,
        ]
    );
    assert_eq!(rects(&bytes)[1], (1, 0, 6, 1), "抜く矩形が広がっていない");

    // 抜いた矩形の左端は投入されたフレームでも同じ色だが、抜かれた以上は書き直す
    let decoded = decode_with_gif(&bytes);
    let last = &decoded.frames[2];
    assert!(
        last.left <= 1 && u32::from(last.left) + u32::from(last.width) > 1,
        "書き直す矩形が抜いた画素を覆っていない"
    );
}

/// 毎フレーム変わるティッカーの隣で、離れた静止物が消える素材
///
/// 静止物は保留中のフレームの矩形の外にあり、そのフレームを描く直前にも
/// 不透明なので、どちらの候補でも抜けない。
fn a_still_object_vanishing() -> Vec<Vec<u8>> {
    const WIDTH: u32 = 16;
    let color = ColorType::Rgba8;

    let frame = |ticker: u8, object: bool| {
        let mut data = solid(WIDTH, 8, color, &[0, 0, 0, 0]);
        for y in 0..2 {
            for x in 0..4 {
                set_pixel(&mut data, WIDTH, color, x, y, &[ticker, 0x20, 0x30, 0xFF]);
            }
        }
        if object {
            for y in 2..4 {
                for x in 8..12 {
                    set_pixel(&mut data, WIDTH, color, x, y, &[0x11, 0x99, 0x55, 0xFF]);
                }
            }
        }
        data
    };

    vec![frame(0x40, true), frame(0x50, true), frame(0x60, false)]
}

/// どの候補でも抜けない画素があれば、保留中のフレームの矩形を広げる
///
/// 広げた矩形は保留中のフレームを描く直前の画面との差分として符号化し直す。
/// 広げた分は描く直前と一致する画素なので透過ランに潰れ、画面は変わらない。
#[test]
fn a_pixel_outside_the_pending_rect_widens_it() {
    let frames = a_still_object_vanishing();
    let (bytes, _) = round_trip(16, 8, ColorType::Rgba8, &frames);

    assert_eq!(
        disposals(&bytes),
        [
            gif::DisposalMethod::Keep,
            gif::DisposalMethod::Background,
            gif::DisposalMethod::Keep,
        ]
    );
    assert_eq!(
        rects(&bytes),
        [(0, 0, 16, 8), (0, 0, 12, 4), (0, 0, 4, 2)],
        "保留中のフレームの矩形が広がっていない"
    );
}

/// 透過を持たない素材はキャンバスを残したまま流れる
///
/// 候補 2 と 3 が立つのは「不透明 → 透過」の遷移を含むフレームだけで、
/// 不透明な素材の出力は候補が増えても変わらない。
#[test]
fn an_opaque_animation_keeps_every_frame() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    let frames = moving_sprite(WIDTH, HEIGHT, 6);

    let (bytes, _) = round_trip(WIDTH, HEIGHT, ColorType::Rgb8, &frames);
    assert!(
        disposals(&bytes)
            .iter()
            .all(|&disposal| disposal == gif::DisposalMethod::Keep),
        "不透明な素材でキャンバスを抜いている"
    );
}

/// 透過インデックスを持たないテーブルでは、後から現れた透過画素を弾く
///
/// 和集合がちょうど256色を埋めた先頭区間から据えると透過スロットが取れない。
/// 廃棄方法は画面から画素を抜けるが、抜いた位置を書かずに済ませる添字が無く、
/// 透過の位置そのものを表現できない。
#[test]
fn a_transparent_pixel_without_a_transparent_index_is_rejected() {
    let color = ColorType::Rgba8;
    let first: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 0xFF]).collect();
    let mut second = first.clone();
    set_pixel(&mut second, 16, color, 1, 1, &[0, 0, 0, 0]);

    let config = Config {
        color_type: color,
        max_spool_bytes: 0,
        ..Config::default()
    };
    assert!(matches!(
        encode_with(16, 16, config, &[first, second]),
        Err(Error::UnsupportedTransparency)
    ));
}

/// スプールの上限で使う素材
///
/// 全フレームが先頭フレームと同じ2色しか使わないため、先頭区間だけで据えた
/// カラーテーブルでも以降のフレームを覆える。
fn moving_sprite(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
    let color = ColorType::Rgb8;
    (0..count)
        .map(|index| {
            let mut frame = solid(width, height, color, &[0x30, 0x50, 0x70]);
            let at = index as u32 % (width * height);
            set_pixel(
                &mut frame,
                width,
                color,
                at % width,
                at / width,
                &[0xF0, 0xF0, 0xF0],
            );
            frame
        })
        .collect()
}

/// スプールの上限に達しても、溜めた区間と以降のフレームが同じ判定で書かれる
#[test]
fn frames_beyond_the_spool_limit_survive_both_decoders() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    let frames = moving_sprite(WIDTH, HEIGHT, 9);

    // 上限まで溜めた出力は、溜めきった出力とフレームごとに一致する
    let (whole, report) = round_trip(WIDTH, HEIGHT, ColorType::Rgb8, &frames);
    assert!(matches!(report.palette, PaletteKind::Exact { .. }));
    let composed = compose(&decode_with_gif(&whole));

    for limit in [0, 64, 256, DEFAULT_MAX_SPOOL_BYTES] {
        let config = Config {
            max_spool_bytes: limit,
            ..Config::default()
        };
        let (bytes, report) = encode_with(WIDTH, HEIGHT, config, &frames).unwrap();
        let decoded = decode_with_gif(&bytes);

        // 小さい上限では溜めきれず、先頭区間の色で決着する
        let settled_early = matches!(report.palette, PaletteKind::ExactFromPrefix { .. });
        assert_eq!(
            settled_early,
            limit < DEFAULT_MAX_SPOOL_BYTES,
            "上限 {limit} の決着の仕方が違う: {:?}",
            report.palette
        );

        assert_eq!(decoded.frames.len(), frames.len(), "上限 {limit}");
        assert_eq!(compose(&decoded), composed, "上限 {limit} の合成結果が違う");
        for (index, frame) in decoded.frames.iter().enumerate() {
            assert_eq!(
                frame.delay,
                index as u16 + 2,
                "上限 {limit} の {index} 番目"
            );
        }
        assert!(
            report.peak_spool_bytes >= (WIDTH * HEIGHT) as usize * 3,
            "上限 {limit} で先頭フレームが溜まっていない"
        );

        if let Some(raw) = decode_with_ffmpeg(&bytes) {
            let frame_len = (WIDTH * HEIGHT) as usize * 4;
            assert_eq!(raw.len(), frame_len * frames.len(), "上限 {limit}");
            for (index, actual) in raw.chunks_exact(frame_len).enumerate() {
                assert_eq!(
                    actual, composed[index],
                    "ffmpeg の {index} 番目 (上限 {limit})"
                );
            }
        }
    }
}

/// 上限が最後のフレームで効いても、そのフレームは書き出しの経路を通る
#[test]
fn a_limit_reached_on_the_last_frame_still_writes_it() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    let frames = moving_sprite(WIDTH, HEIGHT, 2);
    let (whole, _) = round_trip(WIDTH, HEIGHT, ColorType::Rgb8, &frames);

    let config = Config {
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (bytes, report) = encode_with(WIDTH, HEIGHT, config, &frames).unwrap();
    assert!(
        matches!(report.palette, PaletteKind::ExactFromPrefix { .. }),
        "上限が効いていない: {:?}",
        report.palette
    );
    assert_eq!(
        compose(&decode_with_gif(&bytes)),
        compose(&decode_with_gif(&whole)),
        "溜めきった出力と合成結果が違う"
    );
}

/// 上限に達した経路では、先頭区間から据えたことがレポートに出る
#[test]
fn a_table_settled_from_a_prefix_is_reported() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    let frames = moving_sprite(WIDTH, HEIGHT, 5);

    let config = Config {
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (_, report) = encode_with(WIDTH, HEIGHT, config, &frames).unwrap();
    assert_eq!(report.palette, PaletteKind::ExactFromPrefix { colors: 2 });

    // 先頭フレームだけを溜めたぶんが山になる
    let region = (WIDTH * HEIGHT) as usize * 3;
    assert!(report.peak_spool_bytes >= region);
    assert!(report.peak_spool_bytes < region * 2);
}

/// 溜めきれず先頭区間だけから量子化したことがレポートに出る
#[test]
fn a_table_quantized_from_a_prefix_is_reported() {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 8;

    let frames = quantized_frames(WIDTH, HEIGHT, 3);
    let config = Config {
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (_, report) = encode_with(WIDTH, HEIGHT, config, &frames).unwrap();

    assert!(
        matches!(report.palette, PaletteKind::QuantizedFromPrefix { .. }),
        "{:?}",
        report.palette
    );
}

/// 透過だけの先頭区間から据えたテーブルでも、後から現れた不透明色を写せる
///
/// 和集合が透過標識だけだと写す先の候補が1つも残らない。カラーテーブルは
/// 2エントリ未満を書けないため、埋め草の黒がその候補になる。
#[test]
fn an_opaque_pixel_after_a_fully_transparent_prefix_is_mapped_to_the_padding() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgba8;

    let first = solid(WIDTH, HEIGHT, color, &[0, 0, 0, 0]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0x10, 0x20, 0x30, 0xFF]);

    let config = Config {
        color_type: color,
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (bytes, report) = encode_with(WIDTH, HEIGHT, config, &[first, second]).unwrap();
    assert_eq!(report.palette, PaletteKind::ExactFromPrefix { colors: 1 });
    assert_eq!(report.approximated_pixels, 1);
    assert!(report.black_fallback, "写す先の黒を足したことが出ていない");

    let decoded = decode_with_gif(&bytes);
    let screen = &compose(&decoded)[1];
    let at = ((WIDTH + 1) * 4) as usize;
    assert_eq!(
        screen[at..at + 4],
        [0, 0, 0, 0xFF],
        "不透明な黒へ写っていない"
    );
}

/// 先頭区間から据えたテーブルに無い色が後から現れたら、最近傍へ写す
#[test]
fn a_color_appearing_after_the_settlement_is_mapped_to_its_nearest() {
    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    let color = ColorType::Rgb8;

    let first = solid(WIDTH, HEIGHT, color, &[0x30, 0x50, 0x70]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 2, 1, &[0xF0, 0xF0, 0xF0]);

    let config = Config {
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (bytes, report) = encode_with(WIDTH, HEIGHT, config, &[first, second]).unwrap();
    assert_eq!(report.palette, PaletteKind::ExactFromPrefix { colors: 1 });

    // 据えたテーブルの非透過色は1つしかなく、後から現れた色もそこへ写る
    let decoded = decode_with_gif(&bytes);
    let solid_screen = solid(WIDTH, HEIGHT, ColorType::Rgba8, &[0x30, 0x50, 0x70, 0xFF]);
    for (index, screen) in compose(&decoded).iter().enumerate() {
        assert_eq!(screen, &solid_screen, "{index} 番目の合成結果が違う");
    }
}

/// 先頭区間が上限いっぱいの色で、257色目が後から現れたら最近傍へ写す
///
/// 溜めた区間の和集合が上限を埋めていると透過スロットが取れず、矩形の中の
/// 未変更画素も添字を引く。潰されない画素で載っていない色に当たる経路になる。
#[test]
fn a_257th_color_after_a_full_prefix_is_mapped_to_its_nearest() {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 16;
    let color = ColorType::Rgb8;

    let first: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [i as u8, 0x40, 0x80])
        .collect();
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 3, 2, &[0x00, 0x41, 0x80]);

    let config = Config {
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (bytes, report) = encode_with(WIDTH, HEIGHT, config, &[first, second]).unwrap();
    assert_eq!(report.palette, PaletteKind::ExactFromPrefix { colors: 256 });

    // 赤だけが候補ごとに違い、ビンの中心 (実値1.5) に最も近いのは実値1の色
    let decoded = decode_with_gif(&bytes);
    let screen = &compose(&decoded)[1];
    let at = ((2 * WIDTH + 3) * 4) as usize;
    assert_eq!(screen[at..at + 4], [0x01, 0x40, 0x80, 0xFF]);
}

/// 全画素不透明の先頭区間の後に透過画素が現れても、廃棄方法で表現する
///
/// テーブルには透過ラン用のスロットが載っているため、標識を書く先はある。
/// 先頭フレームの矩形は論理画面全体なので、それを抜けば遷移が表現できる。
#[test]
fn a_transparent_pixel_after_an_opaque_prefix_clears_the_screen() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgba8;

    let first = solid(WIDTH, HEIGHT, color, &[0x20, 0x40, 0x60, 0xFF]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0, 0, 0, 0]);

    let config = Config {
        color_type: color,
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (bytes, report) = round_trip_config(WIDTH, HEIGHT, config, &[first, second], 0);
    assert_eq!(report.palette, PaletteKind::ExactFromPrefix { colors: 1 });
    assert_eq!(
        disposals(&bytes),
        [gif::DisposalMethod::Background, gif::DisposalMethod::Keep]
    );
}

/// 溜めた区間の最後のフレームは、続きを見るまで書き出さない
///
/// 上限で決着した直後のフレームが「不透明 → 透過」の遷移を持つとき、その判定は
/// 保留したままの最後のフレームに対して行われる。区間の中で書き出してしまうと
/// 遷移を判定する相手が無くなり、廃棄方法を選べない。
#[test]
fn the_last_spooled_frame_is_carried_into_streaming() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgba8;

    // 標識を先頭フレームへ入れて、先頭区間の和集合が2枚目の色を覆うようにする
    let mut first = solid(WIDTH, HEIGHT, color, &[0x20, 0x40, 0x60, 0xFF]);
    set_pixel(&mut first, WIDTH, color, 0, 0, &[0, 0, 0, 0]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0, 0, 0, 0]);

    let config = Config {
        color_type: color,
        max_spool_bytes: 0,
        ..Config::default()
    };
    let (bytes, _) = round_trip_config(WIDTH, HEIGHT, config, &[first, second], 0);
    assert_eq!(
        disposals(&bytes),
        [gif::DisposalMethod::Background, gif::DisposalMethod::Keep],
        "溜めた区間の最後のフレームが廃棄方法を選べていない"
    );
}

/// 設定した再生回数がループ数の欄へ落ちる
#[test]
fn the_number_of_plays_reaches_the_decoder() {
    let frames = vec![vec![0x10, 0x20, 0x30]];
    for (num_plays, repeat) in [
        (0, gif::Repeat::Infinite),
        (2, gif::Repeat::Finite(1)),
        (10, gif::Repeat::Finite(9)),
        (u32::MAX, gif::Repeat::Finite(u16::MAX)),
    ] {
        let (bytes, _) = encode(1, 1, ColorType::Rgb8, &frames, num_plays).unwrap();
        assert_eq!(
            decode_with_gif(&bytes).repeat,
            repeat,
            "再生回数 {num_plays}"
        );
    }

    // 1回だけ再生するときはアプリケーション拡張を書かない
    let (bytes, _) = encode(1, 1, ColorType::Rgb8, &frames, 1).unwrap();
    assert!(
        !bytes.windows(11).any(|window| window == b"NETSCAPE2.0"),
        "1回再生でアプリケーション拡張が出た"
    );
    assert_eq!(decode_with_gif(&bytes).frames.len(), 1);
}

/// 1/100秒で表せない遅延は丸められ、下限に届かない遅延は切り上げられる
#[test]
fn delays_are_rounded_and_raised_to_the_lower_bound() {
    let config = Config::default();
    let frames = [vec![0x10, 0x20, 0x30], vec![0x40, 0x50, 0x60]];

    let mut encoder = Encoder::new(Vec::new(), 1, 1, 2, config).unwrap();
    for data in &frames {
        encoder
            .add_frame(data, FrameDelay::new(1001, 30000).unwrap())
            .unwrap();
    }
    let (bytes, report) = encoder.finish().unwrap();
    assert!(!report.delay_clamped);
    let decoded = decode_with_gif(&bytes);
    assert_eq!(
        decoded.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
        [3, 3]
    );

    let mut encoder = Encoder::new(Vec::new(), 1, 1, 2, config).unwrap();
    for data in &frames {
        encoder
            .add_frame(data, FrameDelay::new(1, 100).unwrap())
            .unwrap();
    }
    let (bytes, report) = encoder.finish().unwrap();
    assert!(report.delay_clamped, "切り上げがレポートに載っていない");
    let decoded = decode_with_gif(&bytes);
    assert_eq!(
        decoded.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
        [2, 2]
    );
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
fn a_frame_count_of_zero_is_rejected() {
    assert!(matches!(
        Encoder::new(Vec::new(), 1, 1, 0, Config::default()),
        Err(Error::InvalidFrameCount)
    ));
}

/// 和集合が上限を超えたら量子化して据える
///
/// 箱の境界はビンの境界にしか置けないため、色は6-6-6のビンごとに散らす。
#[test]
fn more_than_256_colors_go_through_quantization() {
    let data: Vec<u8> = (0..257).flat_map(distinct_bin).collect();
    // 1つの箱へ最大3つのビンがまとまり、両端の色は平均から実値4だけ離れる
    let (_, report) = round_trip_within(257, 1, ColorType::Rgba8, &[data], 4);

    // 257個のビンを255の箱へ割るので、隣り合う2組だけが1つの箱へまとまる
    assert_eq!(report.palette, PaletteKind::Quantized { colors: 255 });
    assert!(
        report.approximated_pixels > 0,
        "量子化したのに近似した画素が数えられていない"
    );
}

/// 色豊かな背景の上を1画素が動くフレーム列
///
/// 背景の512色で和集合が上限を超えるため、量子化の経路に乗る。
fn quantized_frames(width: u32, height: u32, count: usize) -> Vec<Vec<u8>> {
    let background: Vec<u8> = (0..width * height)
        .flat_map(|i| [(i % width * 4) as u8, (i / width * 32) as u8, 0])
        .collect();

    (0..count)
        .map(|index| {
            let mut frame = background.clone();
            set_pixel(
                &mut frame,
                width,
                ColorType::Rgb8,
                index as u32 % width,
                index as u32 % height,
                &[0xFF, 0x7F, 0xFF],
            );
            frame
        })
        .collect()
}

/// 量子化の経路でも、フレームごとの合成が入力に十分近い
#[test]
fn a_quantized_animation_survives_both_decoders() {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 8;

    let frames = quantized_frames(WIDTH, HEIGHT, 6);
    // 512個のビンを255の箱へ割るので、1つの箱に最大3つのビンが入る
    let (_, report) = round_trip_within(WIDTH, HEIGHT, ColorType::Rgb8, &frames, 6);

    assert!(
        matches!(report.palette, PaletteKind::Quantized { .. }),
        "{:?}",
        report.palette
    );
    assert!(
        report.approximated_pixels > 0,
        "量子化したのに近似した画素が数えられていない"
    );
}

/// 入力が変わらない画素は、量子化の経路でも画面上の色が変わらない
///
/// 変わっていない画素は写し直さず前の描画後の色を持ち越すため、パレットの
/// 誤差が時間方向に揺れとして現れない。
#[test]
fn unchanged_input_keeps_the_color_on_screen() {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 8;

    let frames = quantized_frames(WIDTH, HEIGHT, 6);
    let (bytes, _) = encode(WIDTH, HEIGHT, ColorType::Rgb8, &frames, 0).unwrap();
    let screens = compose(&decode_with_gif(&bytes));

    for (index, inputs) in frames.windows(2).enumerate() {
        let (before, after) = (&screens[index], &screens[index + 1]);
        for at in 0..(WIDTH * HEIGHT) as usize {
            if inputs[0][at * 3..at * 3 + 3] != inputs[1][at * 3..at * 3 + 3] {
                continue;
            }
            assert_eq!(
                before[at * 4..at * 4 + 4],
                after[at * 4..at * 4 + 4],
                "{index} 番目から {at} 画素目の色が揺れた"
            );
        }
    }
}

/// 量子化の経路でも、矩形の中の未変更画素は透過インデックスに潰れる
///
/// キャンバスが持つのは写した後の色なので、入力の画素と比べても一致しない。
/// 潰す相手を取り違えると絵は変わらないまま、LZWのランだけが伸びなくなる。
#[test]
fn unchanged_pixels_inside_a_quantized_rect_are_written_as_transparent() {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 8;
    let color = ColorType::Rgb8;

    let first: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [(i % WIDTH * 4) as u8, (i / WIDTH * 32) as u8, 0])
        .collect();
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 2, 0, &[0, 0, 200]);
    set_pixel(&mut second, WIDTH, color, 60, 0, &[8, 0, 200]);

    let (bytes, report) = encode(WIDTH, HEIGHT, color, &[first, second], 0).unwrap();
    assert!(
        matches!(report.palette, PaletteKind::Quantized { .. }),
        "{:?}",
        report.palette
    );

    let decoded = decode_with_gif(&bytes);
    let frame = &decoded.frames[1];
    assert_eq!(frame.rect(), (2, 0, 59, 1));

    let transparent: Vec<usize> = frame
        .rgba
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[3] == 0)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        transparent,
        (1..58).collect::<Vec<usize>>(),
        "変わっていない画素が潰れていない"
    );
}

/// 量子化で同じ色へ落ちた画素は、入力が変わっていても差分矩形に入らない
///
/// 同じ6-6-6のビンに入る2色は必ず同じ箱へ落ちるため、その間の書き換えは
/// 描画後の色を変えない。矩形を入力の画素で求めると、この画素まで含んでしまう。
#[test]
fn a_pixel_that_quantizes_to_the_same_color_stays_out_of_the_rect() {
    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 8;
    let color = ColorType::Rgb8;

    // 512色を6-6-6のビンへ1つずつ散らす
    let first: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [(i % WIDTH * 4) as u8, (i / WIDTH * 32) as u8, 0])
        .collect();

    let mut second = first.clone();
    // 実値を2だけ動かす。ビンは変わらないので同じ箱へ落ちる
    set_pixel(&mut second, WIDTH, color, 8, 2, &[8 * 4 + 2, 2 * 32, 0]);
    // 離れたビンへ動かす。こちらは描画後の色が変わる
    set_pixel(&mut second, WIDTH, color, 40, 6, &[40 * 4, 6 * 32, 128]);

    let (bytes, report) = encode(WIDTH, HEIGHT, color, &[first, second], 0).unwrap();
    assert!(
        matches!(report.palette, PaletteKind::Quantized { .. }),
        "{:?}",
        report.palette
    );

    let decoded = decode_with_gif(&bytes);
    assert_eq!(decoded.frames[1].rect(), (40, 6, 1, 1));
}

/// 和集合は全フレームで数えるため、後のフレームが上限を超えさせる
#[test]
fn colors_accumulated_across_frames_go_through_quantization() {
    let width = 200u32;
    let frames: Vec<Vec<u8>> = (0..2)
        .map(|index| (0..width).flat_map(|i| [i as u8, index as u8, 0]).collect())
        .collect();
    let (_, report) = round_trip_within(width, 1, ColorType::Rgb8, &frames, 4);

    assert!(
        matches!(report.palette, PaletteKind::Quantized { .. }),
        "{:?}",
        report.palette
    );
}

/// 256色の非透過色に透過画素が加わると和集合が上限を超える
///
/// 量子化の経路は非透過色を255色までに抑えるため、透過スロットが必ず取れる。
#[test]
fn a_transparent_pixel_beyond_256_opaque_colors_takes_the_transparent_slot() {
    let mut data: Vec<u8> = (0..256).flat_map(distinct_bin).collect();
    data.extend_from_slice(&[0, 0, 0, 0]);
    let (bytes, report) = round_trip_within(257, 1, ColorType::Rgba8, &[data], 4);

    assert_eq!(report.palette, PaletteKind::Quantized { colors: 255 });
    let decoded = decode_with_gif(&bytes);
    assert!(
        decoded.frames[0].transparent.is_some(),
        "透過インデックスが載っていない"
    );
}

#[test]
fn a_frame_of_the_wrong_length_is_rejected() {
    let config = Config::default();
    let mut encoder = Encoder::new(Vec::new(), 4, 4, 1, config).unwrap();
    assert!(matches!(
        encoder.add_frame(&[0; 47], delay_of(0)),
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
    encoder.add_frame(&[1, 2, 3], delay_of(0)).unwrap();
    assert!(matches!(
        encoder.add_frame(&[1, 2, 3], delay_of(1)),
        Err(Error::FrameCountMismatch {
            expected: 1,
            actual: 2
        })
    ));
}
