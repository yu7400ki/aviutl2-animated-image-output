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
fn compose(decoded: &Decoded) -> Vec<Vec<u8>> {
    let stride = usize::from(decoded.width) * 4;
    let mut canvas = vec![0u8; stride * usize::from(decoded.height)];
    let mut screens = Vec::new();

    for frame in &decoded.frames {
        assert_eq!(
            frame.dispose,
            gif::DisposalMethod::Keep,
            "扱えない廃棄方法が出た"
        );
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

/// 出力を2つのデコーダへ通し、フレームごとの合成結果を正規化した入力と突き合わせる
fn round_trip(
    width: u32,
    height: u32,
    color_type: ColorType,
    frames: &[Vec<u8>],
) -> (Vec<u8>, Report) {
    let (bytes, report) = encode(width, height, color_type, frames, 0).unwrap();
    let expected: Vec<Vec<u8>> = frames
        .iter()
        .map(|data| expected_rgba(data, color_type))
        .collect();

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
        let decoder = format!("`gif` クレートの {index} 番目");
        assert_transparency(actual, expected, &decoder);
        assert_eq!(actual, expected, "{decoder} の合成結果が違う");
    }

    if let Some(raw) = decode_with_ffmpeg(&bytes) {
        let frame_len = width as usize * height as usize * 4;
        assert_eq!(
            raw.len(),
            frame_len * frames.len(),
            "ffmpeg が返したフレーム数が違う"
        );
        for (index, (actual, expected)) in raw.chunks_exact(frame_len).zip(&expected).enumerate() {
            let decoder = format!("ffmpeg の {index} 番目");
            assert_transparency(actual, expected, &decoder);
            assert_eq!(actual, expected.as_slice(), "{decoder} の合成結果が違う");
        }
    }

    (bytes, report)
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

/// 不透明な画素が透過になる遷移は、まだ表現できない
///
/// 透過インデックスはキャンバスを書き換えないため、キャンバスを残す廃棄方法では
/// 抜けない。
#[test]
fn an_opaque_pixel_turning_transparent_is_rejected() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgba8;

    let first = solid(WIDTH, HEIGHT, color, &[0x20, 0x40, 0x60, 0xFF]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0, 0, 0, 0]);

    assert!(matches!(
        encode(WIDTH, HEIGHT, color, &[first, second], 0),
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

/// 先頭区間から据えたテーブルに無い色が後から現れたら弾く
#[test]
fn a_color_appearing_after_the_settlement_is_rejected() {
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
    assert!(matches!(
        encode_with(WIDTH, HEIGHT, config, &[first, second]),
        Err(Error::TooManyColors)
    ));
}

/// 先頭区間が上限いっぱいの色で、257色目が後から現れたら弾く
///
/// 溜めた区間の和集合が上限を埋めていると透過スロットが取れず、矩形の中の
/// 未変更画素も添字を引く。潰されない画素で載っていない色に当たる経路になる。
#[test]
fn a_257th_color_after_a_full_prefix_is_rejected() {
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
    assert!(matches!(
        encode_with(WIDTH, HEIGHT, config, &[first, second]),
        Err(Error::TooManyColors)
    ));
}

/// 全画素不透明の先頭区間の後に透過画素が現れたら、遷移として弾く
///
/// テーブルには透過ラン用のスロットが載っているため、弾く理由は色ではなく
/// 「不透明 → 透過」がキャンバスを残す廃棄方法で表現できないこと。
#[test]
fn a_transparent_pixel_after_an_opaque_prefix_is_a_transition() {
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
    assert!(matches!(
        encode_with(WIDTH, HEIGHT, config, &[first, second]),
        Err(Error::UnsupportedTransparency)
    ));
}

/// 溜めた区間の最後のフレームは、続きを見るまで書き出さない
///
/// 上限で決着した直後のフレームが「不透明 → 透過」の遷移を持つとき、その判定は
/// 保留したままの最後のフレームに対して行われる。区間の中で書き出してしまうと
/// 遷移を判定する相手が無くなり、表現できないことに気づけない。
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
    assert!(matches!(
        encode_with(WIDTH, HEIGHT, config, &[first, second]),
        Err(Error::UnsupportedTransparency)
    ));
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

#[test]
fn more_than_256_colors_are_rejected() {
    let data: Vec<u8> = (0..257)
        .flat_map(|i| [i as u8, (i >> 8) as u8, 0, 0xFF])
        .collect();
    assert!(matches!(
        encode(257, 1, ColorType::Rgba8, &[data], 0),
        Err(Error::TooManyColors)
    ));
}

/// 和集合は全フレームで数えるため、後のフレームが上限を超えさせる
#[test]
fn colors_accumulated_across_frames_are_rejected() {
    let width = 200u32;
    let frames: Vec<Vec<u8>> = (0..2)
        .map(|index| (0..width).flat_map(|i| [i as u8, index as u8, 0]).collect())
        .collect();
    assert!(matches!(
        encode(width, 1, ColorType::Rgb8, &frames, 0),
        Err(Error::TooManyColors)
    ));
}

/// 256色の非透過色に透過画素が加わると和集合が上限を超える
#[test]
fn a_transparent_pixel_beyond_256_opaque_colors_is_rejected() {
    let mut data: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 0xFF]).collect();
    data.extend_from_slice(&[0, 0, 0, 0]);
    assert!(matches!(
        encode(257, 1, ColorType::Rgba8, &[data], 0),
        Err(Error::TooManyColors)
    ));
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
