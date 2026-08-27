//! 出力したGIFをデコードし、フレームごとの合成結果が正規化した入力と画素単位で
//! 一致することを確認する
//!
//! LZWの誤りは一部のデコーダだけが読めるファイルを作るため、`gif` クレートと
//! ffmpeg の2つでデコードする。ffmpeg が見つからない環境では、そちらだけを
//! 飛ばして `gif` クレートの結果で判定する。

use gif_encoder::{ColorType, Config, Encoder, Error, FrameDelay, PaletteKind, Report};
use std::cell::RefCell;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
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
/// 廃棄はフレームを表示した後に効き、Background は矩形を透過へ抜く。
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

/// バイト列をデコーダへ渡すための一時ファイルへ書き出す
fn temp_gif(bytes: &[u8]) -> PathBuf {
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
    path
}

/// ffmpeg で合成済みのフレームへデコードした生RGBA。ffmpeg が無ければ `None`
fn decode_with_ffmpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let path = temp_gif(bytes);

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

    // 透過添字はエントリを占有しないため、全画素透過の素材でもテーブルは
    // 透過添字を持つ。写す先の黒が要るのは、非透過色を1つも持たない色表が
    // できたときだけ
    assert!(!report.black_fallback, "写す先の黒を足している");

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

/// 出力のブロックを先頭からたどって読み出したもの
struct Scanned {
    /// グローバルカラーテーブルのバイト列
    global_table: Vec<u8>,
    /// フレームごとに画像データが宣言するLZW最小符号長
    min_code_sizes: Vec<u8>,
    /// グラフィック制御拡張が宣言する透過インデックス
    transparent: Vec<Option<u8>>,
}

/// ブロックの区切りを長さの宣言だけでたどる
///
/// 画像データのバイトには画像記述子やヘッダと同じ値が現れるため、目印を探す
/// 読み方では区切りを取り違える。
fn scan(bytes: &[u8]) -> Scanned {
    let entries_of = |packed: u8| 2usize << (packed & 0x07);
    assert_eq!(&bytes[..6], b"GIF89a", "ヘッダが違う");
    let packed = bytes[10];
    assert_eq!(
        packed & 0x80,
        0x80,
        "グローバルカラーテーブルを持つ宣言が無い"
    );

    let mut at = 13;
    let global_table = bytes[at..at + entries_of(packed) * 3].to_vec();
    at += global_table.len();

    let mut min_code_sizes = Vec::new();
    let mut transparent = Vec::new();
    loop {
        match bytes[at] {
            // 終端
            0x3B => {
                assert_eq!(at + 1, bytes.len(), "終端の後にバイトが残っている");
                return Scanned {
                    global_table,
                    min_code_sizes,
                    transparent,
                };
            }
            // 拡張ブロック
            0x21 => {
                // グラフィック制御拡張は4バイトのデータ副ブロック1つを持つ
                if bytes[at + 1] == 0xF9 {
                    let declared = bytes[at + 3] & 0x01 != 0;
                    transparent.push(declared.then(|| bytes[at + 6]));
                }
                at = skip_sub_blocks(bytes, at + 2);
            }
            // 画像記述子
            0x2C => {
                let packed = bytes[at + 9];
                at += 10;
                if packed & 0x80 != 0 {
                    at += entries_of(packed) * 3;
                }
                min_code_sizes.push(bytes[at]);
                at = skip_sub_blocks(bytes, at + 1);
            }
            found => panic!("{at} バイト目に知らないブロック {found:#04X} がある"),
        }
    }
}

/// サブブロックの列を読み飛ばし、ブロック終端の次の位置を返す
fn skip_sub_blocks(bytes: &[u8], mut at: usize) -> usize {
    while bytes[at] != 0 {
        at += 1 + usize::from(bytes[at]);
    }
    at + 1
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

/// 低色数の素材でも、グローバルカラーテーブルは256エントリで書かれる
///
/// 最小符号長はそのフレームで使った最大の添字から決まるので、大きさの欄が
/// 256エントリを示していても符号は広がらない。
#[test]
fn a_low_color_frame_pads_the_global_table_without_widening_the_codes() {
    for (colors, min_code_size) in [
        (2usize, 2u8),
        (3, 2),
        (4, 2),
        (5, 3),
        (8, 3),
        (9, 4),
        (16, 4),
    ] {
        let data: Vec<u8> = (0..32 * 32)
            .flat_map(|i| {
                let value = (i % colors) as u8;
                [value, value.wrapping_mul(7), value.wrapping_mul(13)]
            })
            .collect();
        let (bytes, report) = round_trip(32, 32, ColorType::Rgb8, &[data]);
        assert_eq!(
            report.palette,
            PaletteKind::Exact {
                colors: colors as u16
            },
            "{colors}色"
        );

        let scanned = scan(&bytes);
        assert_eq!(scanned.global_table.len(), 768, "{colors}色");
        // 透過ラン用のスロットが最後の色の次に載り、その先が埋め草になる
        assert!(
            scanned.global_table[(colors + 1) * 3..]
                .iter()
                .all(|&byte| byte == 0),
            "{colors}色の埋め草が黒でない"
        );
        assert_eq!(scanned.min_code_sizes, [min_code_size], "{colors}色");
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

/// 閾値未満のアルファは完全透過へ潰れ、動いた画素数が向きごとにレポートに載る
#[test]
fn partial_alpha_is_binarized_before_encoding() {
    let data: Vec<u8> = (0..64 * 8)
        .flat_map(|i| [(i % 200) as u8, 0x10, 0x20, (i % 256) as u8])
        .collect();
    let alphas = || data.chunks_exact(4).map(|p| p[3]);
    let squashed = alphas().filter(|&a| (1..128).contains(&a)).count() as u64;
    let raised = alphas().filter(|&a| (128..255).contains(&a)).count() as u64;
    assert!(squashed > 0 && raised > 0, "両方が動く素材になっていない");

    let (_, report) = round_trip(64, 8, ColorType::Rgba8, &[data]);
    assert_eq!(report.binarized_to_transparent, squashed);
    assert_eq!(report.binarized_to_opaque, raised);
}

/// 元から2値のアルファしか無い素材は、2値化で何も動かない
#[test]
fn binary_alpha_moves_no_pixels() {
    let data: Vec<u8> = (0..64 * 8)
        .flat_map(|i| {
            let alpha = if i % 3 == 0 { 0 } else { 255 };
            [(i % 200) as u8, 0x10, 0x20, alpha]
        })
        .collect();

    let (_, report) = round_trip(64, 8, ColorType::Rgba8, &[data]);
    assert_eq!(report.binarized_to_transparent, 0);
    assert_eq!(report.binarized_to_opaque, 0);
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
        panic!("溢れずに書き終えていない: {:?}", report.palette)
    };
    assert!(colors <= 64, "和集合が {colors} 色まで広がっている");
}

/// 一度も溢れなければ、最後に見つけた順の色をテーブルへ書き戻す
///
/// 書き戻しの前は場所を確保してあるだけなので、色は最後まで決まらない。
#[test]
fn a_table_that_never_overflows_is_written_back_at_the_end() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgb8;

    // 2枚目で新しい色が現れる。書き戻した色は見つけた順に並ぶ
    let first = solid(WIDTH, HEIGHT, color, &[0x10, 0x20, 0x30]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0x40, 0x50, 0x60]);
    let mut third = second.clone();
    set_pixel(&mut third, WIDTH, color, 2, 0, &[0x70, 0x80, 0x90]);

    let (bytes, report) = round_trip(WIDTH, HEIGHT, color, &[first, second, third]);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 3 });

    let table = scan(&bytes).global_table;
    assert_eq!(table.len(), 768, "確保した大きさが変わっている");
    assert_eq!(
        table[..9],
        [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90],
        "見つけた順に色が並んでいない"
    );
    assert!(
        table[9..].iter().all(|&byte| byte == 0),
        "余りが黒で埋まっていない"
    );
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

/// 素材自身の透過画素と未変更画素のランは、そのフレームの透過添字を分け合う
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

    // 先頭フレームの矩形は論理画面全体で、素材の標識はその透過添字で書かれる
    for (index, frame) in decoded.frames.iter().enumerate() {
        assert!(
            frame.transparent.is_some(),
            "{index} 番目に透過インデックスが無い"
        );
    }
    assert_eq!(
        decoded.frames[0].rgba[3], 0,
        "素材の透過画素が透過インデックスで書かれていない"
    );

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
    assert_eq!(report.palette, PaletteKind::Exact { colors: 65 });

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

/// 抜く候補が2つとも同じ矩形になる素材
///
/// 上半分が透過、下半分が画素ごとに違う色。パネルはその境目をまたいで現れ、
/// 消えるフレームは境目の下の行を元の色へ戻したうえで、行の両端だけを塗り替える。
///
/// 矩形を丸ごと抜く候補は下の行を全部書き直し、描く直前へ戻す候補は両端の2画素
/// しか書かない。どちらの外接矩形も同じ行なので、**広さは引き分け、圧縮後の
/// 大きさだけが分かれる**。
fn a_panel_whose_candidates_tie_on_area() -> Vec<Vec<u8>> {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 4;
    let color = ColorType::Rgba8;

    let mut base = solid(WIDTH, HEIGHT, color, &[0, 0, 0, 0]);
    for y in 2..HEIGHT {
        for x in 0..WIDTH {
            set_pixel(
                &mut base,
                WIDTH,
                color,
                x,
                y,
                &[(x * 8) as u8, (y * 16) as u8, 0x40, 0xFF],
            );
        }
    }

    // パネルは透過の行と不透明な行にまたがる
    let mut panel = base.clone();
    for y in 1..3 {
        for x in 2..14 {
            set_pixel(&mut panel, WIDTH, color, x, y, &[0x90, 0x30, 0x10, 0xFF]);
        }
    }

    let mut gone = base.clone();
    set_pixel(&mut gone, WIDTH, color, 2, 2, &[0x11, 0x22, 0x33, 0xFF]);
    set_pixel(&mut gone, WIDTH, color, 13, 2, &[0x44, 0x55, 0x66, 0xFF]);

    vec![base, panel, gone]
}

/// 広さが引き分けの抜く候補は、圧縮後の小さい方が採られる
///
/// 画素数は矩形の広さの目安にしかならず、透過ランがどれだけ伸びるかを写さない。
#[test]
fn the_clearing_candidates_are_compared_after_compression() {
    let frames = a_panel_whose_candidates_tie_on_area();
    let (bytes, _) = round_trip(16, 4, ColorType::Rgba8, &frames);

    use gif::DisposalMethod::{Keep, Previous};
    assert_eq!(
        disposals(&bytes),
        [Keep, Previous, Keep],
        "圧縮後の小さい候補が採られていない"
    );
    // どちらの候補もこの矩形になる。分かれるのは中身だけ
    assert_eq!(rects(&bytes)[2], (2, 2, 12, 1));
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

/// 2色だけで1画素が動くフレーム列
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

/// 上限を超える色を持つフレームで、その場の窓から量子化へ移る
///
/// 先頭フレームが1枚で上限を超えるので、割り当て済みの色は無いまま閉じる。
/// 以降の色は最近傍へ写る。
#[test]
fn a_frame_that_overflows_the_table_moves_to_quantization() {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 9;

    let first: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(distinct_bin).collect();
    let mut second = first.clone();
    set_pixel(
        &mut second,
        WIDTH,
        ColorType::Rgba8,
        1,
        1,
        &[0xFC, 0xFC, 0, 0xFF],
    );

    let (_, report) = round_trip_within(WIDTH, HEIGHT, ColorType::Rgba8, &[first, second], 4);
    assert_eq!(report.palette, PaletteKind::Quantized { colors: 255 });
    assert!(
        report.approximated_pixels > 0,
        "上限を超えた色が最近傍へ写っていない"
    );
}

/// 割り当て済みの色が残っているまま閉じても、透過添字の余地が1つ残る
///
/// 量子化するのは空きのぶんだけで、割り当て済みの色はそのままの添字で残る。
/// 空きを数え違えると、テーブルが色で埋まって透過添字を持てなくなる。
#[test]
fn a_table_settled_after_some_colors_leaves_room_for_the_transparent_index() {
    const WIDTH: u32 = 20;
    const HEIGHT: u32 = 16;
    /// 先頭フレームで割り当てる色
    const ALLOCATED: [[u8; 3]; 4] = [
        [0x10, 0x20, 0x30],
        [0x11, 0x21, 0x31],
        [0x12, 0x22, 0x32],
        [0x13, 0x23, 0x33],
    ];
    let color = ColorType::Rgb8;

    let mut first = solid(WIDTH, HEIGHT, color, &ALLOCATED[0]);
    for (at, pixel) in ALLOCATED.iter().enumerate().skip(1) {
        set_pixel(&mut first, WIDTH, color, at as u32, 0, pixel);
    }
    // 2枚目で上限を超える色が一度に現れ、そこでテーブルが閉じる
    let second: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 4) as u8, 0x80])
        .collect();

    let (bytes, report) = encode(WIDTH, HEIGHT, color, &[first, second], 0).unwrap();
    let PaletteKind::Quantized { colors } = report.palette else {
        panic!("溢れていない: {:?}", report.palette)
    };
    assert!(
        usize::from(colors) <= 255,
        "透過添字の余地が残っていない: {colors}色"
    );
    assert!(
        usize::from(colors) > ALLOCATED.len(),
        "量子化した色が載っていない: {colors}色"
    );

    // 割り当て済みの色は添字も並びもそのまま残る
    let table = scan(&bytes).global_table;
    let head: Vec<u8> = ALLOCATED.concat();
    assert_eq!(table[..head.len()], head, "割り当て済みの色が動いている");

    let decoded = decode_with_gif(&bytes);
    for (index, frame) in decoded.frames.iter().enumerate() {
        assert!(
            frame.transparent.is_some(),
            "{index} 番目に透過インデックスが無い"
        );
    }
    assert_eq!(
        decoded.frames[1].transparent,
        Some(colors as u8),
        "透過添字が、色の載っていない最小の添字になっていない"
    );
}

/// 色で埋まったテーブルに257色目が現れたら、その色は最近傍へ写る
///
/// 先頭フレームが上限ちょうどの色を埋めるので、量子化の空きは残らない。
/// 割り当て済みの色は動かないまま、載らなかった色だけが寄る。
#[test]
fn a_257th_color_is_mapped_to_its_nearest() {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 16;
    let color = ColorType::Rgb8;

    let first: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [i as u8, 0x40, 0x80])
        .collect();
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 3, 2, &[0x00, 0x41, 0x80]);

    let (bytes, report) = encode(WIDTH, HEIGHT, color, &[first, second], 0).unwrap();
    assert_eq!(report.palette, PaletteKind::Quantized { colors: 256 });
    assert_eq!(report.approximated_pixels, 1);

    // 赤だけが候補ごとに違い、ビンの中心 (実値1.5) に最も近いのは実値1の色
    let decoded = decode_with_gif(&bytes);
    let screen = &compose(&decoded)[1];
    let at = ((2 * WIDTH + 3) * 4) as usize;
    assert_eq!(screen[at..at + 4], [0x01, 0x40, 0x80, 0xFF]);
}

/// 全画素透過の先頭フレームの後に現れた不透明な色も、そのまま載る
///
/// 透過標識は添字を占めないので、テーブルはまだどの色も割り当てていない。
/// 後から現れた色は見つけた順に添字を得る。
#[test]
fn an_opaque_pixel_after_a_fully_transparent_frame_keeps_its_color() {
    const WIDTH: u32 = 100;
    const HEIGHT: u32 = 100;
    const SIDE: u32 = 10;
    let color = ColorType::Rgba8;
    let square = [0xE0, 0x20, 0x30, 0xFF];

    let first = solid(WIDTH, HEIGHT, color, &[0, 0, 0, 0]);
    let mut second = first.clone();
    for y in 0..SIDE {
        for x in 0..SIDE {
            set_pixel(&mut second, WIDTH, color, x, y, &square);
        }
    }

    let config = Config {
        color_type: color,
        ..Config::default()
    };
    let (bytes, report) = round_trip_config(WIDTH, HEIGHT, config, &[first, second], 0);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 1 });
    assert_eq!(report.local_tables, 0, "可逆のまま色表を運んでいる");
    assert_eq!(report.substituted_pixels, 0);

    let decoded = decode_with_gif(&bytes);
    let screen = &compose(&decoded)[1];
    let at = ((WIDTH + 1) * 4) as usize;
    assert_eq!(screen[at..at + 4], square, "素材の色が残っていない");
}

/// 全画素不透明の先頭フレームの後に透過画素が現れても、廃棄方法で表現する
///
/// テーブルは透過添字を持つため、標識を書く先はある。先頭フレームの矩形は
/// 論理画面全体なので、それを抜けば遷移が表現できる。
#[test]
fn a_transparent_pixel_after_an_opaque_frame_clears_the_screen() {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;
    let color = ColorType::Rgba8;

    let first = solid(WIDTH, HEIGHT, color, &[0x20, 0x40, 0x60, 0xFF]);
    let mut second = first.clone();
    set_pixel(&mut second, WIDTH, color, 1, 1, &[0, 0, 0, 0]);

    let config = Config {
        color_type: color,
        ..Config::default()
    };
    let (bytes, report) = round_trip_config(WIDTH, HEIGHT, config, &[first, second], 0);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 1 });
    assert_eq!(
        disposals(&bytes),
        [gif::DisposalMethod::Background, gif::DisposalMethod::Keep]
    );
}

/// 色で埋まったテーブルに透過画素が現れたら、そのフレームは色表へ逃げる
///
/// 不透明な色が上限を埋めると、まだ色の割り当たっていない添字が無くなる。
/// そこへ透過画素が現れると、グローバルカラーテーブルではその位置を表現できない。
///
/// 廃棄方法は保留中のフレームと突き合わせて決まるため、透過が要るのは標識を
/// 持つフレームだけではない。矩形を抜かれる側のフレームも自分の透過添字を
/// 宣言している必要がある。
#[test]
fn a_transparent_pixel_after_a_full_opaque_table_forces_an_escape() {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 16;
    let color = ColorType::Rgba8;

    let first: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [i as u8, 0x40, 0x80, 0xFF])
        .collect();
    let mut third = first.clone();
    set_pixel(&mut third, WIDTH, color, 3, 2, &[0, 0, 0, 0]);
    let frames = vec![first.clone(), first, third];

    let config = Config {
        color_type: color,
        ..Config::default()
    };
    let (bytes, report) = encode_with(WIDTH, HEIGHT, config, &frames).unwrap();
    assert_eq!(report.palette, PaletteKind::Quantized { colors: 256 });
    assert_eq!(report.local_tables, 2, "逃げた色表を書いていない");
    // 変わった画素がどれも透過標識なので、逃げた色表は非透過色を1つも持たない。
    // 抜いた矩形で書き直す持ち越しの画素は、写す先として足した黒へ落ちる
    assert!(report.black_fallback, "写す先の黒を足していない");
    assert!(
        report.substituted_pixels > 0,
        "黒へ落ちた画素を数えていない"
    );

    // グラフィック制御拡張のバイトで見る。透過は色表ではなくここが宣言する
    let declared = scan(&bytes).transparent;
    assert_eq!(
        declared[0], None,
        "色で埋まったテーブルが透過添字を持っている"
    );
    assert!(
        declared[1].is_some(),
        "矩形を抜かれる側のフレームに透過添字が無い"
    );
    assert!(
        declared[2].is_some(),
        "透過画素を持つフレームに透過添字が無い"
    );

    let screen = &compose(&decode_with_gif(&bytes))[2];
    assert_eq!(
        screen[((2 * WIDTH + 3) * 4) as usize..][..4],
        [0, 0, 0, 0],
        "透過にした画素が不透明のまま残っている"
    );
}

/// 溢れる前に書いたフレームの色は、書き戻したテーブルでも変わらない
///
/// 割り当て済みの色は添字ごとそのまま残るため、後から書き戻すテーブルの前半は
/// 既に書いたフレームが指しているものと一致する。並べ替えると、書き終えた
/// フレームの画面が変わる。
#[test]
fn the_frames_written_before_the_overflow_keep_their_colors() {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 24;
    const OVERFLOW_AT: usize = 9;
    const FRAMES: usize = 14;
    let color = ColorType::Rgb8;

    let background = [0x10u8, 0x20, 0x30];
    let dot = [0xF0u8, 0xF0, 0xF0];
    let frames: Vec<Vec<u8>> = (0..FRAMES)
        .map(|index| {
            let mut frame = solid(WIDTH, HEIGHT, color, &background);
            set_pixel(&mut frame, WIDTH, color, index as u32 % WIDTH, 0, &dot);
            if index >= OVERFLOW_AT {
                // 上限を超える色を一度に置き、この位置でテーブルを閉じさせる
                for i in 0..300u32 {
                    let pixel = [(i % 64 * 4) as u8, (i / 64 * 4) as u8, 0x80];
                    set_pixel(&mut frame, WIDTH, color, i % WIDTH, 8 + i / WIDTH, &pixel);
                }
            }
            frame
        })
        .collect();

    let (bytes, report) = encode(WIDTH, HEIGHT, color, &frames, 0).unwrap();
    assert!(
        matches!(report.palette, PaletteKind::Quantized { .. }),
        "溢れていない: {:?}",
        report.palette
    );

    let expected: Vec<Vec<u8>> = frames
        .iter()
        .map(|data| expected_rgba(data, color))
        .collect();
    let screens = compose(&decode_with_gif(&bytes));
    for index in 0..OVERFLOW_AT {
        assert_eq!(
            screens[index], expected[index],
            "`gif` クレートの {index} 番目が書き戻しで変わった"
        );
    }

    if let Some(raw) = decode_with_ffmpeg(&bytes) {
        let frame_len = (WIDTH * HEIGHT) as usize * 4;
        for (index, actual) in raw.chunks_exact(frame_len).take(OVERFLOW_AT).enumerate() {
            assert_eq!(
                actual, expected[index],
                "ffmpeg の {index} 番目が書き戻しで変わった"
            );
        }
    }
}

/// 不透明な256色ちょうどの素材は可逆で、色が揃うまでは透過ランも使える
///
/// 透過添字はまだ色の割り当たっていない最小の添字なので、色が上限を埋めるまでの
/// フレームは未変更画素を潰せる。
#[test]
fn a_material_with_exactly_256_opaque_colors_stays_lossless() {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 16;
    const FILLS_AT: usize = 3;
    const FRAMES: usize = 6;
    let color = ColorType::Rgb8;

    // 253色を敷き、動く2色と合わせて255色。最後の1色は途中のフレームで現れる
    let base: Vec<u8> = (0..WIDTH * HEIGHT)
        .flat_map(|i| [(i % 253) as u8, 0x40, 0x80])
        .collect();
    let dots = [[9u8, 9, 9], [10, 10, 10]];
    let frames: Vec<Vec<u8>> = (0..FRAMES)
        .map(|index| {
            let mut frame = base.clone();
            // 最終行の両端を入れ替えると、その間の未変更画素が矩形の中に入る
            let turn = index % 2;
            set_pixel(&mut frame, WIDTH, color, 13, 15, &dots[turn]);
            set_pixel(&mut frame, WIDTH, color, 15, 15, &dots[1 - turn]);
            if index >= FILLS_AT {
                set_pixel(&mut frame, WIDTH, color, 0, 0, &[0xFF, 0x40, 0x80]);
            }
            frame
        })
        .collect();

    let (bytes, report) = round_trip(WIDTH, HEIGHT, color, &frames);
    assert_eq!(report.palette, PaletteKind::Exact { colors: 256 });
    assert_eq!(report.approximated_pixels, 0, "可逆で書けていない");

    let decoded = decode_with_gif(&bytes);
    let frame = &decoded.frames[1];
    assert!(
        frame.transparent.is_some(),
        "色が揃う前のフレームが透過添字を持っていない"
    );
    assert!(
        frame.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0),
        "未変更画素を透過ランへ潰していない"
    );
    assert_eq!(
        decoded.frames[FRAMES - 1].transparent,
        None,
        "色で埋まったテーブルが透過添字を持っている"
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

/// フレームごとの遅延を指定して1x1のフレーム列を符号化する
fn encode_delays(frames: &[Vec<u8>], delays: &[FrameDelay]) -> (Vec<u8>, Report) {
    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        1,
        1,
        frames.len() as u32,
        Config::default(),
    )
    .unwrap();
    for (data, delay) in frames.iter().zip(delays) {
        encoder.add_frame(data, *delay).unwrap();
    }
    let (writer, report) = encoder.finish().unwrap();
    (writer.into_inner(), report)
}

/// 色だけが違う1x1のフレームを `count` 枚
fn distinct_pixels(count: u8) -> Vec<Vec<u8>> {
    (0..count).map(|i| vec![i * 0x10, 0x20, 0x30]).collect()
}

/// 2つのデコーダが読み出したフレームごとの遅延を確かめる
fn assert_delays(bytes: &[u8], expected: &[u16], label: &str) {
    let decoded = decode_with_gif(bytes);
    assert_eq!(
        decoded.frames.iter().map(|f| f.delay).collect::<Vec<_>>(),
        expected,
        "`gif` クレートが読んだ {label} の遅延が違う"
    );
    if let Some(actual) = probe_delays(bytes) {
        assert_eq!(actual, expected, "ffprobe が読んだ {label} の遅延が違う");
    }
}

/// ffprobe が読み出したフレームごとの表示時間 (1/100秒)。ffprobe が無ければ `None`
///
/// GIF の時間の刻みは 1/100 秒なので、刻みの数がそのまま遅延になる。
fn probe_delays(bytes: &[u8]) -> Option<Vec<u16>> {
    let path = temp_gif(bytes);
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=duration",
            "-of",
            "csv=p=0",
            "-i",
        ])
        .arg(&path)
        .output();
    std::fs::remove_file(&path).unwrap();

    let output = match output {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("CI").is_none(),
                "ffprobe が見つからない。デコーダが1つでは、一部のデコーダだけが読める出力を見つけられない"
            );
            eprintln!("ffprobe が見つからないため、そちらの読み出しを飛ばす");
            return None;
        }
        Err(e) => panic!("ffprobe の起動に失敗した: {e}"),
    };
    assert!(
        output.status.success(),
        "ffprobe の読み出しに失敗した: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| line.trim().parse().unwrap())
            .collect(),
    )
}

/// 30fps は3フレームで10/100秒になる
///
/// 総和は 3.33, 6.67, 10.00, … と進むので、その丸めの差は 3, 4, 3 を繰り返す。
#[test]
fn delays_are_accumulated_across_the_frames() {
    let frames = distinct_pixels(9);
    for (numerator, denominator) in [(1, 30), (1001, 30000)] {
        let delay = FrameDelay::new(numerator, denominator).unwrap();
        let (bytes, report) = encode_delays(&frames, &vec![delay; frames.len()]);
        assert!(!report.delay_clamped, "{numerator}/{denominator}");
        assert_delays(
            &bytes,
            &[3, 4, 3, 3, 4, 3, 3, 4, 3],
            &format!("{numerator}/{denominator}"),
        );
    }
}

/// 下限に届かない遅延は切り上げ、そのぶんは累積へ戻さない
#[test]
fn a_delay_below_the_lower_bound_is_raised_without_feeding_it_back() {
    let frames = distinct_pixels(3);
    let delays = [(1, 100), (1, 10), (1, 10)]
        .map(|(numerator, denominator)| FrameDelay::new(numerator, denominator).unwrap());
    let (bytes, report) = encode_delays(&frames, &delays);
    assert!(report.delay_clamped, "切り上げがレポートに載っていない");
    assert_delays(&bytes, &[2, 10, 10], "切り上げた列");
}

#[test]
fn zero_and_oversized_dimensions_are_rejected() {
    let config = Config::default();
    for (width, height) in [(0, 1), (1, 0), (65536, 1), (1, 65536)] {
        assert!(
            matches!(
                Encoder::new(Cursor::new(Vec::new()), width, height, 1, config),
                Err(Error::InvalidDimensions { .. })
            ),
            "{width}x{height}"
        );
    }
    assert!(Encoder::new(Cursor::new(Vec::new()), 65535, 1, 1, config).is_ok());
}

#[test]
fn a_frame_count_of_zero_is_rejected() {
    assert!(matches!(
        Encoder::new(Cursor::new(Vec::new()), 1, 1, 0, Config::default()),
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
    let mut encoder = Encoder::new(Cursor::new(Vec::new()), 4, 4, 1, config).unwrap();
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
    let encoder = Encoder::new(Cursor::new(Vec::new()), 1, 1, 1, config).unwrap();
    assert!(matches!(
        encoder.finish(),
        Err(Error::FrameCountMismatch {
            expected: 1,
            actual: 0
        })
    ));

    let mut encoder = Encoder::new(Cursor::new(Vec::new()), 1, 1, 1, config).unwrap();
    encoder.add_frame(&[1, 2, 3], delay_of(0)).unwrap();
    assert!(matches!(
        encoder.add_frame(&[1, 2, 3], delay_of(1)),
        Err(Error::FrameCountMismatch {
            expected: 1,
            actual: 2
        })
    ));
}

/// 一定バイト数まで受け付け、それ以降は必ず失敗する書き出し先
///
/// 受け付けたバイトは共有した控えへ写す。失敗したエンコーダは書き出し先を
/// 返さないため、途中で切れたブロックの列はこの控えからしか見られない。
struct FailingWriter {
    remaining: usize,
    position: u64,
    written: Rc<RefCell<Vec<u8>>>,
}

impl FailingWriter {
    /// `budget` バイトまで受け付ける書き出し先と、書けたバイト列の控え
    fn new(budget: usize) -> (Self, Rc<RefCell<Vec<u8>>>) {
        let written = Rc::new(RefCell::new(Vec::new()));
        let writer = FailingWriter {
            remaining: budget,
            position: 0,
            written: Rc::clone(&written),
        };
        (writer, written)
    }
}

impl Write for FailingWriter {
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

impl Seek for FailingWriter {
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

/// ヘッダ・論理画面記述子・確保したグローバルカラーテーブルのバイト数
const HEAD_BYTES: usize = 13 + 768;

/// 場所を確保する書き出しに失敗したら、エンコーダを作れない
///
/// [`Encoder::new`] がヘッダと確保したカラーテーブルを書くため、書き出し先の
/// 失敗はここで出る。
#[test]
fn a_writer_that_cannot_take_the_head_is_rejected() {
    let (writer, _) = FailingWriter::new(4);
    assert!(matches!(
        Encoder::new(writer, 8, 8, 1, Config::default()),
        Err(Error::Io(_))
    ));
}

/// 書き戻す前に落ちたファイルには、確保しただけのカラーテーブルが残る
///
/// 黒で埋めると、色の決まっていないファイルが真っ黒なアニメーションとして
/// 黙って読めてしまう。
#[test]
fn a_table_that_was_never_written_back_stays_visible() {
    const FRAMES: u32 = 16;
    const MAGENTA: [u8; 3] = [0xFF, 0x00, 0xFF];

    let frames: Vec<Vec<u8>> = (0..FRAMES).map(|seed| noise(8 * 8 * 3, seed)).collect();
    // ループ回数まで書いたところで尽き、フレームを1枚も書けない長さ
    let (writer, written) = FailingWriter::new(HEAD_BYTES + 19);
    let mut encoder = Encoder::new(writer, 8, 8, FRAMES, Config::default()).unwrap();

    let failure = frames
        .iter()
        .enumerate()
        .find_map(|(index, frame)| encoder.add_frame(frame, delay_of(index)).err());
    assert!(matches!(failure, Some(Error::Io(_))), "{failure:?}");

    let bytes = written.borrow();
    assert_eq!(bytes.len(), HEAD_BYTES + 19, "確保した先まで書けている");
    assert!(
        bytes[13..HEAD_BYTES]
            .chunks_exact(3)
            .all(|entry| entry == MAGENTA),
        "確保しただけのエントリが黙って読める色になっている"
    );
}

/// 途中で切れたブロックの列に書き足すと読めないGIFになるため、失敗後は受け付けない
///
/// 書き出しは先読みリングが埋まるまで始まらないので、失敗するのは投入の途中に
/// なる。何枚目かは先読みの深さで決まるため、最初に失敗した投入を探す。
#[test]
fn a_failed_write_poisons_the_encoder() {
    const FRAMES: u32 = 16;
    let frames: Vec<Vec<u8>> = (0..FRAMES).map(|seed| noise(8 * 8 * 3, seed)).collect();
    // 確保したカラーテーブルまでは通り、フレームの書き出しで尽きる長さ
    let (writer, _) = FailingWriter::new(HEAD_BYTES + 19);
    let mut encoder = Encoder::new(writer, 8, 8, FRAMES, Config::default()).unwrap();

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

/// 1枚書き終えた後で尽きた書き出しは、終端の無いブロックの列を残す
///
/// 毒を仕込む理由がこの形にある。中断した列に書き足しても終端まで揃わないため、
/// 失敗した後のエンコーダは投入も終端も受け付けない。
#[test]
fn a_write_that_fails_after_a_frame_leaves_the_stream_unterminated() {
    const FRAMES: u32 = 16;
    /// GIFの終端を表すブロック
    const TRAILER: u8 = 0x3B;

    let frames: Vec<Vec<u8>> = (0..FRAMES).map(|seed| noise(8 * 8 * 3, seed)).collect();
    let config = Config::default();

    // 終端まで書けたときの出力と、1枚目を書き終えた時点のバイト数を測る
    let (writer, complete) = FailingWriter::new(usize::MAX);
    let mut encoder = Encoder::new(writer, 8, 8, FRAMES, config).unwrap();
    let mut first_frame_bytes = 0;
    for (index, frame) in frames.iter().enumerate() {
        encoder.add_frame(frame, delay_of(index)).unwrap();
        if first_frame_bytes == 0 {
            first_frame_bytes = complete.borrow().len();
        }
    }
    encoder.finish().unwrap();
    let full = complete.borrow().clone();
    assert!(first_frame_bytes > 0, "投入の途中で書き出していない");
    assert!(first_frame_bytes < full.len(), "1枚目で出力が終わっている");

    // 同じ投入を、1枚目を書き終えたところで尽きる予算で走らせる
    let (writer, interrupted) = FailingWriter::new(first_frame_bytes);
    let mut encoder = Encoder::new(writer, 8, 8, FRAMES, config).unwrap();
    let mut accepted = 0;
    let mut failure = None;
    for (index, frame) in frames.iter().enumerate() {
        match encoder.add_frame(frame, delay_of(index)) {
            Ok(()) => accepted += 1,
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    assert!(accepted > 0, "1枚も受け付けないまま失敗している");
    assert!(matches!(failure, Some(Error::Io(_))), "{failure:?}");

    assert!(matches!(
        encoder.add_frame(&frames[0], delay_of(0)),
        Err(Error::Poisoned)
    ));
    assert!(matches!(encoder.finish(), Err(Error::Poisoned)));

    // グローバルカラーテーブルは色が決まった時点で書き戻されるので、
    // 中断した出力に残るのは確保しただけのバイト列になる
    let written = interrupted.borrow();
    assert_eq!(
        &written[..13],
        &full[..13],
        "書けたバイト列がヘッダで食い違う"
    );
    assert_eq!(
        &written[HEAD_BYTES..],
        &full[HEAD_BYTES..first_frame_bytes],
        "書けたバイト列が、テーブルより後ろで終端まで書けた出力と食い違う"
    );
    assert_eq!(
        full.last(),
        Some(&TRAILER),
        "終端まで書いた出力に終端が無い"
    );
    assert_ne!(
        written.last(),
        Some(&TRAILER),
        "中断した列が終端を持っている"
    );
}

/// カラーテーブルの据え直しを見る素材の寸法
const SCENE_WIDTH: u32 = 64;
const SCENE_HEIGHT: u32 = 64;

/// テーブルを閉じさせる帯の色数
const CROWD_COLORS: u32 = 300;

/// 帯を描く先頭行
const CROWD_ROW: u32 = 56;

/// 帯が現れてテーブルが閉じるフレーム
const SETTLES_AT: usize = 9;

/// 色が入れ替わってグローバルカラーテーブルから逃げるフレーム
///
/// 閉じた時点の先読みの窓 ([`SETTLES_AT`] から8フレーム) の外に置く。窓の中に
/// 入れると、閉じたテーブルが入れ替わった後の色まで覆ってしまう。
const ESCAPES_AT: usize = SETTLES_AT + 8;

/// 上限を超える色の帯を `frame` へ描く
///
/// 1フレームでこれだけの色が現れると、その位置でテーブルが閉じて量子化へ移る。
/// `blue` を変えた帯は閉じたテーブルの色から遠いので、逃げ道へ踏み切らせる。
fn crowd(frame: &mut [u8], color_type: ColorType, blue: u8) {
    let bpp = color_type.bytes_per_pixel();
    for i in 0..CROWD_COLORS {
        let pixel = [(i % 64 * 4) as u8, (i / 64 * 4) as u8, blue, 0xFF];
        set_pixel(
            frame,
            SCENE_WIDTH,
            color_type,
            i % SCENE_WIDTH,
            CROWD_ROW + i / SCENE_WIDTH,
            &pixel[..bpp],
        );
    }
}

/// 合成結果から画素を1つ取り出す
fn pixel_at(screen: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = (y as usize * SCENE_WIDTH as usize + x as usize) * 4;
    screen[at..at + 4].try_into().unwrap()
}

/// 先頭フレーム以降まったく変わらない目印の色
const MARKER: [u8; 3] = [0xFF, 0x00, 0xFF];
/// 目印が占める行数
const MARKER_ROWS: u32 = 2;

/// 入力が変わらない画素の画面上の色は、色表から逃げても変わらない
///
/// 目印の帯は先頭フレームで割り当てた色をそのまま持ち、以降どのフレームでも
/// 入力が変わらない。逃げた先の色表は変わった画素だけから作るので目印の色を
/// 持たないが、持ち越した画素を写し直さない限り画面には残り続ける。
#[test]
fn a_pixel_that_never_changes_keeps_its_color_across_an_escape() {
    const FRAMES: usize = ESCAPES_AT + 4;
    let color = ColorType::Rgb8;

    let frames: Vec<Vec<u8>> = (0..FRAMES)
        .map(|index| {
            let mut frame = solid(SCENE_WIDTH, SCENE_HEIGHT, color, &[0x04, 0x04, 0x04]);
            for y in 0..MARKER_ROWS {
                for x in 0..SCENE_WIDTH {
                    set_pixel(&mut frame, SCENE_WIDTH, color, x, y, &MARKER);
                }
            }
            if index >= SETTLES_AT {
                crowd(&mut frame, color, 0x40);
            }
            if index >= ESCAPES_AT {
                crowd(&mut frame, color, 0xC0);
            }
            frame
        })
        .collect();

    let (bytes, report) = encode(SCENE_WIDTH, SCENE_HEIGHT, color, &frames, 0).unwrap();
    assert_eq!(
        report.local_tables, 1,
        "色表を運んだのは色の入れ替わった1フレームだけではない"
    );

    let expected = [MARKER[0], MARKER[1], MARKER[2], u8::MAX];
    for (index, screen) in compose(&decode_with_gif(&bytes)).iter().enumerate() {
        for y in 0..MARKER_ROWS {
            assert_eq!(
                pixel_at(screen, 0, y),
                expected,
                "{index} 番目で目印の色が変わっている"
            );
        }
    }
}

/// 瞬きの目印を置く位置
const BLINK_AT: (u32, u32) = (0, 0);

/// 場面転換を跨いで瞬く画素を持つ素材
///
/// 瞬く色は先頭区間で割り当てるので、閉じたテーブルにそのまま載っている。
/// 動く点が毎フレームその色を書くため、据え直しの時点では直近の出力に現れている。
fn blinking_scene(frames: usize, blink_back_at: usize) -> Vec<Vec<u8>> {
    /// 瞬く画素が戻ってくる色
    const BLINK: [u8; 3] = [0xFF, 0x00, 0xFF];
    /// 場面転換の後に瞬く画素が持つ色
    const BLINKED: [u8; 3] = [0xFF, 0xFF, 0x00];
    /// 場面転換で画面を覆う色
    const AFTER: [[u8; 3]; 4] = [
        [0x00, 0xFF, 0x00],
        [0x00, 0xF0, 0x10],
        [0x10, 0xFF, 0x00],
        [0x10, 0xF0, 0x10],
    ];
    let color = ColorType::Rgb8;

    (0..frames)
        .map(|index| {
            let changed = index >= ESCAPES_AT;
            let mut frame = Vec::with_capacity((SCENE_WIDTH * SCENE_HEIGHT) as usize * 3);
            for at in 0..(SCENE_WIDTH * SCENE_HEIGHT) as usize {
                let pixel = if changed {
                    AFTER[at % AFTER.len()]
                } else {
                    [0x20, 0x10, 0x20]
                };
                frame.extend_from_slice(&pixel);
            }
            if index >= SETTLES_AT && !changed {
                crowd(&mut frame, color, 0x40);
            }

            // 毎フレーム動く点が、瞬く色を直近の出力に残す
            let dot = index as u32 % SCENE_WIDTH;
            set_pixel(&mut frame, SCENE_WIDTH, color, dot, 4, &BLINK);
            let blink = if changed && index < blink_back_at {
                BLINKED
            } else {
                BLINK
            };
            set_pixel(
                &mut frame,
                SCENE_WIDTH,
                color,
                BLINK_AT.0,
                BLINK_AT.1,
                &blink,
            );
            frame
        })
        .collect()
}

/// 瞬き (A→B→A) で戻った色は、場面が入れ替わっても同じ色で戻る
///
/// グローバルカラーテーブルは一度振った添字を手放さないので、先頭区間で
/// 割り当てた瞬きの色は場面が入れ替わった後もそのまま引ける。
#[test]
fn a_color_that_blinks_back_survives_a_scene_change() {
    const BLINK_BACK_AT: usize = ESCAPES_AT + 8;
    const FRAMES: usize = BLINK_BACK_AT + 2;

    let frames = blinking_scene(FRAMES, BLINK_BACK_AT);
    let (bytes, report) = encode(SCENE_WIDTH, SCENE_HEIGHT, ColorType::Rgb8, &frames, 0).unwrap();
    assert!(report.local_tables > 0, "色表から逃げていない");

    let screens = compose(&decode_with_gif(&bytes));
    assert_eq!(
        pixel_at(&screens[BLINK_BACK_AT], BLINK_AT.0, BLINK_AT.1),
        pixel_at(&screens[0], BLINK_AT.0, BLINK_AT.1),
        "戻ってきた色が先頭フレームと違う"
    );
}

/// テーブルが閉じた後に現れる色を、先読みの窓のフレームへ散らした素材
///
/// 帯の現れるフレームでテーブルが閉じ、続く数フレームがブロック1つずつ色を
/// 足す。足す色は帯からも背景からも遠い。
fn colors_spread_over_the_window(frames: usize, spread: &[[u8; 3]]) -> Vec<Vec<u8>> {
    let color = ColorType::Rgb8;

    (0..frames)
        .map(|index| {
            let mut frame = solid(SCENE_WIDTH, SCENE_HEIGHT, color, &[0x80, 0x80, 0x80]);
            if index >= SETTLES_AT {
                crowd(&mut frame, color, 0x40);
            }
            // 足した色は消さずに積み上げる。1フレームで変わるのはブロック1つぶん
            for (slot, color_of) in spread.iter().enumerate() {
                if index < SETTLES_AT + slot + 1 {
                    break;
                }
                for y in 0..SPREAD_BLOCK {
                    for x in 0..SPREAD_BLOCK {
                        let at = (slot as u32 * SPREAD_BLOCK + x, y);
                        set_pixel(&mut frame, SCENE_WIDTH, color, at.0, at.1, color_of);
                    }
                }
            }
            frame
        })
        .collect()
}

/// 後から足す色が占める辺の長さ
const SPREAD_BLOCK: u32 = 4;

/// 量子化の材料は書き出し位置の1枚ではなく、先読みの窓全体から取る
///
/// 窓の中の後続フレームが足す色までグローバルカラーテーブルに載るので、その
/// フレームは完全一致を引けて色表を運ばずに済む。窓が書き出し位置の1枚だけなら、
/// これらの色はテーブルから遠く、フレームごとに逃げることになる。
#[test]
fn the_quantized_table_reaches_the_whole_lookahead_window() {
    const SPREAD: [[u8; 3]; 6] = [
        [0x00, 0xFF, 0x00],
        [0x00, 0x00, 0xFF],
        [0xFF, 0xFF, 0x00],
        [0x00, 0xFF, 0xFF],
        [0xFF, 0x00, 0xFF],
        [0xFF, 0xFF, 0xFF],
    ];
    const FRAMES: usize = SETTLES_AT + SPREAD.len() + 3;

    let frames = colors_spread_over_the_window(FRAMES, &SPREAD);
    let (bytes, report) = encode(SCENE_WIDTH, SCENE_HEIGHT, ColorType::Rgb8, &frames, 0).unwrap();
    assert_eq!(report.local_tables, 0, "窓の中で足した色から逃げている");

    let screens = compose(&decode_with_gif(&bytes));
    for (slot, expected) in SPREAD.iter().enumerate() {
        let at = slot as u32 * SPREAD_BLOCK;
        assert_eq!(
            pixel_at(&screens[SETTLES_AT + slot + 1], at, 0),
            [expected[0], expected[1], expected[2], u8::MAX],
            "{slot} 番目に足した色がテーブルに載っていない"
        );
    }
}

/// 矩形を塗る
fn fill_block(frame: &mut [u8], at: (u32, u32), size: (u32, u32), pixel: &[u8]) {
    for y in at.1..at.1 + size.1 {
        for x in at.0..at.0 + size.0 {
            set_pixel(frame, SCENE_WIDTH, ColorType::Rgba8, x, y, pixel);
        }
    }
}

/// 保留中のフレームの矩形を広げる符号化と、色表からの逃げが重なる素材
///
/// 保留中のフレームの外で不透明な物が消えるので矩形を広げることになり、同じ
/// フレームで色が入れ替わって逃げ道へ踏み切る。
fn a_widened_rect_across_an_escape(frames: usize) -> Vec<Vec<u8>> {
    /// 消える物の色
    const OBJECT: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
    /// 保留中のフレームが塗る色
    const PAINT: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];

    let transparent = solid(SCENE_WIDTH, SCENE_HEIGHT, ColorType::Rgba8, &[0, 0, 0, 0]);
    (0..frames)
        .map(|index| {
            let mut frame = transparent.clone();
            // 塗る色を先頭フレームの色へ入れておく
            fill_block(&mut frame, (48, 8), (4, 4), &PAINT);
            if index < ESCAPES_AT {
                fill_block(&mut frame, (32, 32), (8, 8), &OBJECT);
            }
            if index >= 1 {
                fill_block(&mut frame, (0, 0), (4, 4), &PAINT);
            }
            if index >= SETTLES_AT {
                crowd(&mut frame, ColorType::Rgba8, 0x40);
            }
            if index >= ESCAPES_AT {
                crowd(&mut frame, ColorType::Rgba8, 0xC0);
            }
            frame
        })
        .collect()
}

/// 広げた矩形は、保留中のフレームを符号化したテーブルで符号化し直す
///
/// 逃げた先の色表は変わった画素だけから作るので、保留中のフレームが載せた色は
/// 入っていない。現在の色表で符号化し直すと、その色が最近傍へずれて画面から
/// 消える。
#[test]
fn a_widened_rect_keeps_the_table_that_encoded_the_pending_frame() {
    const FRAMES: usize = ESCAPES_AT + 4;
    const PAINT: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];

    let frames = a_widened_rect_across_an_escape(FRAMES);
    let config = Config {
        color_type: ColorType::Rgba8,
        ..Config::default()
    };
    let (bytes, report) = encode_with(SCENE_WIDTH, SCENE_HEIGHT, config, &frames).unwrap();
    assert!(report.local_tables > 0, "色表から逃げていない");

    let widened = rects(&bytes)[ESCAPES_AT - 1];
    assert!(
        widened.2 > 4 && widened.3 > 4,
        "保留中のフレームの矩形が広がっていない: {widened:?}"
    );

    // 広げた矩形は廃棄方法が矩形を抜くフレームなので、続くフレームの画面は
    // 抜いた先の扱いがデコーダで割れる。突き合わせるのはこのフレームだけ
    let frame_len = (SCENE_WIDTH * SCENE_HEIGHT) as usize * 4;
    let at = frame_len * (ESCAPES_AT - 1);
    let mut screens = vec![compose(&decode_with_gif(&bytes))[ESCAPES_AT - 1].clone()];
    if let Some(raw) = decode_with_ffmpeg(&bytes) {
        screens.push(raw[at..at + frame_len].to_vec());
    }

    for screen in &screens {
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    pixel_at(screen, x, y),
                    PAINT,
                    "広げた矩形の中で色がずれている"
                );
            }
        }
    }
}

/// 逃げ道を見る素材の寸法
const ESCAPE_WIDTH: u32 = 32;
const ESCAPE_HEIGHT: u32 = 32;

/// 逃げ道を見る素材が先頭フレームに置く色数
///
/// 次のフレームが足す2色で上限を超える数。空きが残らないので、閉じた
/// グローバルカラーテーブルは先頭フレームの色そのままになる。
const ESCAPE_BASE_COLORS: u32 = 255;

/// グローバルカラーテーブルから逃げるフレーム
const ESCAPE_AT: usize = 9;

/// 一様なドリフトが1フレームで動く距離
///
/// 動かすのは青だけで、動かす前の色が最近傍のまま残る。どの画素の二乗距離も
/// `DRIFT_STEP * DRIFT_STEP` になる。
const DRIFT_STEP: u8 = 17;

/// 悪く写った画素を数える引き金が見ていた二乗距離の閾値
const BAD_PIXEL_TOLERANCE: u32 = 20 * 20;

/// 青が0の対角線上に [`ESCAPE_BASE_COLORS`] 色を並べたフレーム
fn escape_base(color_type: ColorType) -> Vec<u8> {
    let bpp = color_type.bytes_per_pixel();
    let mut frame = Vec::with_capacity((ESCAPE_WIDTH * ESCAPE_HEIGHT) as usize * bpp);
    for at in 0..ESCAPE_WIDTH * ESCAPE_HEIGHT {
        let value = (at % ESCAPE_BASE_COLORS) as u8;
        frame.extend_from_slice(&[value, value, 0, 0xFF][..bpp]);
    }
    frame
}

/// テーブルを閉じさせる2色を先頭の2画素へ置いたフレーム
///
/// 足りない色が2色あると上限を超え、その場で量子化へ移る。どちらも黒の近くに
/// あるので、最近傍へ写しても誤差は床に届かない。
fn escape_settled(color_type: ColorType) -> Vec<u8> {
    let bpp = color_type.bytes_per_pixel();
    let mut frame = escape_base(color_type);
    for (x, blue) in [(0u32, 1u8), (1, 2)] {
        set_pixel(
            &mut frame,
            ESCAPE_WIDTH,
            color_type,
            x,
            0,
            &[0, 0, blue, 0xFF][..bpp],
        );
    }
    frame
}

/// 先頭フレームの色を持つ画素を、青の方向へ [`DRIFT_STEP`] だけ動かしたフレーム
///
/// 動くのは3画素目からで、変わった画素の色数は [`ESCAPE_BASE_COLORS`] に収まる。
fn escape_drifted(color_type: ColorType) -> Vec<u8> {
    let bpp = color_type.bytes_per_pixel();
    let mut frame = escape_settled(color_type);
    for at in 2..(ESCAPE_WIDTH * ESCAPE_HEIGHT) as usize {
        frame[at * bpp + 2] += DRIFT_STEP;
    }
    frame
}

/// ドリフトする素材のフレーム列
fn drifting_scene(color_type: ColorType) -> Vec<Vec<u8>> {
    let mut frames = vec![escape_base(color_type)];
    frames.resize(ESCAPE_AT, escape_settled(color_type));
    frames.resize(ESCAPE_AT + 3, escape_drifted(color_type));
    frames
}

/// グローバルカラーテーブルの先頭 `colors` 色
fn global_colors(bytes: &[u8], colors: usize) -> Vec<[u8; 3]> {
    scan(bytes)
        .global_table
        .chunks_exact(3)
        .take(colors)
        .map(|entry| [entry[0], entry[1], entry[2]])
        .collect()
}

/// `frame` の各画素を `table` の最近傍へ写したときの二乗距離
fn nearest_errors(frame: &[u8], color_type: ColorType, table: &[[u8; 3]]) -> Vec<u32> {
    let bpp = color_type.bytes_per_pixel();
    frame
        .chunks_exact(bpp)
        .map(|pixel| {
            table
                .iter()
                .map(|entry| {
                    (0..3)
                        .map(|axis| {
                            let difference = i32::from(pixel[axis]) - i32::from(entry[axis]);
                            (difference * difference) as u32
                        })
                        .sum()
                })
                .min()
                .expect("色表が空")
        })
        .collect()
}

/// 一様なドリフトは逃げを立て、悪く写った画素を数える引き金では立たない
///
/// 全画素が同じだけずれるので二乗距離の平均は床を超える。どの画素の二乗距離も
/// [`BAD_PIXEL_TOLERANCE`] の内側にあるので、閾値を超えた画素を数える引き金は
/// 1画素も拾えない。
#[test]
fn a_uniform_drift_escapes_where_counting_badly_mapped_pixels_would_not() {
    let color = ColorType::Rgb8;
    let frames = drifting_scene(color);

    let (bytes, report) = encode(ESCAPE_WIDTH, ESCAPE_HEIGHT, color, &frames, 0).unwrap();
    assert_eq!(
        report.palette,
        PaletteKind::Quantized {
            colors: ESCAPE_BASE_COLORS as u16
        }
    );
    assert_eq!(report.local_tables, 1, "一様なドリフトで逃げていない");

    // 逃げたフレームの、入力が変わった画素だけを見る
    let table = global_colors(&bytes, ESCAPE_BASE_COLORS as usize);
    let errors = nearest_errors(&frames[ESCAPE_AT], color, &table);
    let step = u32::from(DRIFT_STEP) * u32::from(DRIFT_STEP);
    assert!(
        errors[2..].iter().all(|&error| error == step),
        "ドリフトの二乗距離が一様でない"
    );
    assert!(
        step <= BAD_PIXEL_TOLERANCE,
        "ドリフトが閾値を超えており、画素を数える引き金でも立つ"
    );
}

/// 変わった画素の色数が上限に収まるフレームは、可逆な色表で書かれる
#[test]
fn an_escaped_frame_whose_colors_fit_is_written_losslessly() {
    let color = ColorType::Rgb8;
    let frames = drifting_scene(color);

    let (bytes, report) = encode(ESCAPE_WIDTH, ESCAPE_HEIGHT, color, &frames, 0).unwrap();
    assert_eq!(report.local_tables, 1, "色表へ逃げていない");

    let screen = &compose(&decode_with_gif(&bytes))[ESCAPE_AT];
    for (at, pixel) in frames[ESCAPE_AT].chunks_exact(3).enumerate().skip(2) {
        assert_eq!(
            &screen[at * 4..at * 4 + 3],
            pixel,
            "{at} 番目の画素が入力と一致しない"
        );
    }
}

/// 逃げるフレームに置く、上限を超える色数
const ESCAPE_MANY_COLORS: u32 = 300;

/// グローバルカラーテーブルから遠い [`ESCAPE_MANY_COLORS`] 色で埋めたフレーム
fn escape_crowded(color_type: ColorType) -> Vec<u8> {
    let bpp = color_type.bytes_per_pixel();
    let mut frame = Vec::with_capacity((ESCAPE_WIDTH * ESCAPE_HEIGHT) as usize * bpp);
    for at in 0..ESCAPE_WIDTH * ESCAPE_HEIGHT {
        let index = at % ESCAPE_MANY_COLORS;
        let pixel = [
            (index % 60) as u8 * 4,
            0,
            0x80 + (index / 60) as u8 * 8,
            0xFF,
        ];
        frame.extend_from_slice(&pixel[..bpp]);
    }
    frame
}

/// 変わった画素の色数が上限を超えるフレームは、そのフレームだけで量子化する
///
/// 逃げた色表はグローバルカラーテーブルの色を1つも引き継がないので、置いた色の
/// 近くだけに255色を割ける。
#[test]
fn an_escaped_frame_with_too_many_colors_is_quantized_on_its_own() {
    let color = ColorType::Rgb8;
    let mut frames = vec![escape_base(color)];
    frames.resize(ESCAPE_AT, escape_settled(color));
    frames.resize(ESCAPE_AT + 3, escape_crowded(color));

    let (bytes, report) = encode(ESCAPE_WIDTH, ESCAPE_HEIGHT, color, &frames, 0).unwrap();
    assert_eq!(report.local_tables, 1, "色表へ逃げていない");

    let input = &frames[ESCAPE_AT];
    let screen = &compose(&decode_with_gif(&bytes))[ESCAPE_AT];
    let table = global_colors(&bytes, ESCAPE_BASE_COLORS as usize);
    let far = nearest_errors(input, color, &table);
    assert!(
        far.iter().all(|&error| error > BAD_PIXEL_TOLERANCE),
        "置いた色がグローバルカラーテーブルの近くにある"
    );

    let mut worst = 0;
    let mut exact = 0;
    for (at, pixel) in input.chunks_exact(3).enumerate() {
        let written = &screen[at * 4..at * 4 + 3];
        let difference = (0..3)
            .map(|axis| pixel[axis].abs_diff(written[axis]))
            .max()
            .expect("3軸");
        worst = worst.max(difference);
        exact += usize::from(difference == 0);
    }
    assert!(worst <= 8, "そのフレームの量子化にしては遠い: {worst}");
    assert!(
        exact < input.len() / 3,
        "色数が上限を超えているのに全画素が一致している"
    );
}

/// 逃げた色表は透過スロットを持ち、矩形の中の未変更画素は透過ランになる
#[test]
fn an_escaped_table_writes_the_unchanged_pixels_as_transparent() {
    /// グローバルカラーテーブルの対角線から遠い色
    const FAR: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
    let color = ColorType::Rgba8;

    let mut frames = vec![escape_base(color)];
    frames.resize(ESCAPE_AT, escape_settled(color));
    let mut far = escape_settled(color);
    for at in [(4u32, 4u32), (9, 7)] {
        set_pixel(&mut far, ESCAPE_WIDTH, color, at.0, at.1, &FAR);
    }
    frames.resize(ESCAPE_AT + 2, far);

    let (bytes, report) = encode(ESCAPE_WIDTH, ESCAPE_HEIGHT, color, &frames, 0).unwrap();
    assert_eq!(report.local_tables, 1, "色表へ逃げていない");
    assert!(
        scan(&bytes).transparent[ESCAPE_AT].is_some(),
        "逃げた色表が透過添字を宣言していない"
    );

    let frame = &decode_with_gif(&bytes).frames[ESCAPE_AT];
    assert_eq!(frame.rect(), (4, 4, 6, 4));

    // 矩形の中で透過になった画素は、変えた2画素を除いた全部
    let opaque: Vec<usize> = frame
        .rgba
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[3] != 0)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        opaque,
        [0, 3 * 6 + 5],
        "透過ランが変わった画素まで覆っている"
    );
}
