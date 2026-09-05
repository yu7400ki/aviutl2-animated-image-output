//! 符号化した .jxl を読み直す道具と、投入するフレームの素材
//!
//! 公開APIだけで組み、`tests/` の検証と crate の内側の検証が同じものを引く。

#![allow(dead_code)]

use jxl::api::{self, states::Initialized};
use jxl::bit_reader::BitReader;
use jxl::headers::encodings::UnconditionalCoder;
use jxl::headers::frame_header::{FrameHeader, FrameType};
use jxl::headers::toc::{Toc, TocNonserialized};
use jxl::headers::{FileHeader, JxlHeader};
use jxl_encoder::{ColorType, Config, Encoder, QUALITY_RANGE};
use std::io::Write;

pub const WIDTH: u32 = 48;
pub const HEIGHT: u32 = 32;

/// 1秒あたりのtick数。分子と分母の取り違えが値に出るよう互いに離す
pub const TPS_NUMERATOR: u32 = 30000;
pub const TPS_DENOMINATOR: u32 = 1001;

/// アニメーションの再生回数。±1の混入が値に出るよう1から離す
pub const NUM_PLAYS: u32 = 5;

/// 各フレームの表示時間。並びの取り違えが出るよう互いに違える
pub const DURATIONS: [u32; 3] = [3, 5, 7];

/// 出力が排水の受け皿を超える大きさのキャンバスの一辺
pub const LARGE_SIDE: u32 = 256;

/// 位置から決まる雑音。圧縮が効かないので出力が大きくなる
pub fn noise_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut state = 0x1234_5678u32;
    (0..(width as usize * height as usize * 4))
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

/// 横方向と縦方向で滑らかに変わる不透明なRGB。`phase` は絵をずらす
pub fn gradient_rgb(phase: u32) -> Vec<u8> {
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

/// `gradient_rgb` に、左から右へ薄れるαを足したもの
pub fn gradient_rgba(phase: u32) -> Vec<u8> {
    gradient_rgb(phase)
        .chunks_exact(3)
        .enumerate()
        .flat_map(|(index, pixel)| {
            let x = (index as u32) % WIDTH;
            [pixel[0], pixel[1], pixel[2], (x * 255 / WIDTH) as u8]
        })
        .collect()
}

/// `color_type` の画素を `phase` ぶんずらした1フレーム
pub fn frame(color_type: ColorType, phase: u32) -> Vec<u8> {
    match color_type {
        ColorType::Rgb8 => gradient_rgb(phase),
        ColorType::Rgba8 => gradient_rgba(phase),
    }
}

pub fn config(color_type: ColorType) -> Config {
    Config {
        color_type,
        quality: *QUALITY_RANGE.end(),
        effort: 3,
        num_plays: NUM_PLAYS,
        tps_numerator: TPS_NUMERATOR,
        tps_denominator: TPS_DENOMINATOR,
        max_threads: 2,
    }
}

/// `width` x `height` の `frames` を `writer` へ符号化する
pub fn encode_into<W: Write>(
    writer: W,
    config: Config,
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    durations: &[u32],
) -> W {
    let mut encoder = Encoder::new(writer, width, height, frames.len() as u32, config).unwrap();
    for (frame, duration) in frames.iter().zip(durations) {
        encoder.add_frame(frame, *duration).unwrap();
    }
    encoder.finish().unwrap()
}

/// `width` x `height` の `frames` を符号化する
pub fn encode_frames(
    config: Config,
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    durations: &[u32],
) -> Vec<u8> {
    encode_into(Vec::new(), config, width, height, frames, durations)
}

/// `durations` と同じ数の勾配のフレームを符号化する
pub fn encode(config: Config, durations: &[u32]) -> Vec<u8> {
    let frames: Vec<Vec<u8>> = (0..durations.len())
        .map(|index| frame(config.color_type, index as u32 * 5))
        .collect();
    encode_frames(config, WIDTH, HEIGHT, &frames, durations)
}

/// 読み戻した画像の全体
pub struct Decoded {
    pub info: api::JxlBasicInfo,
    pub profile: api::JxlColorProfile,
    pub frames: Vec<api::VisibleFrameInfo>,
    /// 書かれた順の通常フレームのヘッダ。表示時間0の副フレームを含む
    pub headers: Vec<FrameHeader>,
    /// 合成後の各フレームのキャンバス全面
    pub pixels: Vec<Vec<u8>>,
}

/// 表示時間を持つヘッダ
///
/// 表示時間0の副フレームは次の表示フレームへ畳まれるので、単独では表示されない。
/// 静止画は表示時間の欄を持たないので、ストリームを閉じる1枚がそのまま表示される。
pub fn displayed(headers: &[FrameHeader]) -> Vec<&FrameHeader> {
    headers
        .iter()
        .filter(|header| header.duration != 0 || header.is_last)
        .collect()
}

/// コードストリームを順に歩き、通常フレームのヘッダを読み出す
///
/// 表示時間0の副フレームは `scanned_frames` に現れないので、記録された位置からは
/// 届かない。libjxlがpatchのために書く参照フレームは、こちらが並べたものでは
/// ないので落とす。読めた並びが実体であることを、表示フレームと重なる欄で検める。
pub fn frame_headers(encoded: &[u8], frames: &[api::VisibleFrameInfo]) -> Vec<FrameHeader> {
    let mut reader = BitReader::new(encoded);
    let file_header = FileHeader::read(&mut reader).expect("ファイルヘッダの読み出し");
    let nonserialized = file_header.frame_header_nonserialized();

    let mut headers: Vec<FrameHeader> = Vec::new();
    loop {
        reader
            .jump_to_byte_boundary()
            .expect("フレームの先頭への整列");
        let header = FrameHeader::read_unconditional(&(), &mut reader, &nonserialized)
            .unwrap_or_else(|error| {
                panic!(
                    "{} 枚目のフレームヘッダの読み出し: {error}",
                    headers.len() + 1
                )
            });
        let toc = Toc::read_unconditional(
            &(),
            &mut reader,
            &TocNonserialized {
                num_entries: header.num_toc_entries() as u32,
            },
        )
        .unwrap_or_else(|error| panic!("{} 枚目のTOCの読み出し: {error}", headers.len() + 1));
        reader.jump_to_byte_boundary().expect("節の先頭への整列");
        let section_bytes: u32 = toc.entries.iter().sum();

        let is_last = header.is_last;
        if header.frame_type == FrameType::RegularFrame {
            headers.push(header);
        }
        if is_last {
            break;
        }
        reader
            .skip_bits(section_bytes as usize * 8)
            .expect("次のフレームへの読み飛ばし");
    }

    let display = displayed(&headers);
    assert_eq!(
        display.len(),
        frames.len(),
        "表示フレームの数が読み戻した可視フレームと違う"
    );
    for (header, frame) in display.iter().zip(frames) {
        assert_eq!(
            header.duration,
            frame.duration_ticks,
            "{} 枚目のヘッダが読み戻した可視フレームと違う",
            frame.index + 1
        );
        assert_eq!(
            header.is_last,
            frame.is_last,
            "{} 枚目のヘッダが読み戻した可視フレームと違う",
            frame.index + 1
        );
    }
    headers
}

/// 段を1つ進める。入力を使い切らずに止まったら符号化が不完全
pub fn complete<T, U>(result: api::ProcessingResult<T, U>, what: &str) -> T {
    match result {
        api::ProcessingResult::Complete { result } => result,
        api::ProcessingResult::NeedsMoreInput { size_hint, .. } => {
            panic!("{what} が入力不足で止まった (あと {size_hint} バイト)")
        }
    }
}

/// 全フレームの画素とメタデータを読み出す
pub fn decode(encoded: &[u8], color_type: ColorType) -> Decoded {
    let mut input: &[u8] = encoded;
    let decoder = api::JxlDecoder::<Initialized>::new(api::JxlDecoderOptions::default());
    let mut decoder = complete(
        decoder.process(&mut input, None).unwrap(),
        "画像情報の読み出し",
    );

    let info = decoder.basic_info().clone();
    let profile = decoder.embedded_color_profile().clone();
    let (width, height) = info.size;
    decoder.set_pixel_format(match color_type {
        ColorType::Rgb8 => api::JxlPixelFormat::rgb8(info.extra_channels.len()),
        ColorType::Rgba8 => api::JxlPixelFormat::rgba8(info.extra_channels.len()),
    });

    let bytes_per_row = width * color_type.bytes_per_pixel();
    let mut pixels = Vec::new();
    while decoder.has_more_frames() {
        let with_frame = complete(
            decoder.process(&mut input, None).unwrap(),
            "フレーム情報の読み出し",
        );
        let mut frame = vec![0u8; bytes_per_row * height];
        decoder = {
            let mut buffers = [api::JxlOutputBuffer::new(&mut frame, height, bytes_per_row)];
            complete(
                with_frame.process(&mut input, &mut buffers, None).unwrap(),
                "画素の読み出し",
            )
        };
        pixels.push(frame);
    }

    let frames = decoder.scanned_frames().to_vec();
    Decoded {
        info,
        profile,
        headers: frame_headers(encoded, &frames),
        frames,
        pixels,
    }
}

/// インターリーブされた画素からαだけを取り出す
pub fn alpha_channel(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).map(|pixel| pixel[3]).collect()
}

/// インターリーブされた画素から色だけを取り出す
pub fn color_channels(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

/// キャンバス全体を指す矩形 (x, y, 幅, 高さ)
pub const WHOLE: (u32, u32, u32, u32) = (0, 0, WIDTH, HEIGHT);

/// フレームごとに書き加えるブロック (x, y, 幅, 高さ)
///
/// xとy、幅と高さの取り違えが値に出るよう互いに違え、重なりを持たせない。
/// 取り違えた矩形もキャンバスに収まるので、値を見なければ食い違いが残る。
pub const BLOCKS: [(u32, u32, u32, u32); 3] = [(3, 7, 5, 11), (20, 4, 13, 9), (9, 18, 6, 12)];

/// `BLOCKS` を書き加えていくフレーム列の表示時間
pub const BLOCK_DURATIONS: [u32; BLOCKS.len() + 1] = [3, 5, 7, 11];

/// `block` の範囲を全チャネル反転した値で埋める
///
/// 反転した値は元の値と必ず違うので、範囲がそのまま差分の外接矩形になる。
pub fn invert_block(frame: &mut [u8], color_type: ColorType, block: (u32, u32, u32, u32)) {
    let (x, y, width, height) = block;
    let bytes_per_pixel = color_type.bytes_per_pixel();
    for row in 0..height as usize {
        let start = ((y as usize + row) * WIDTH as usize + x as usize) * bytes_per_pixel;
        for byte in &mut frame[start..start + width as usize * bytes_per_pixel] {
            *byte = !*byte;
        }
    }
}

/// 先頭が勾配で、以降は `BLOCKS` を1つずつ書き加えたフレーム列
///
/// 隣り合うフレームはブロック1つ分しか違わないので、差分矩形は画面の一部になる。
pub fn block_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let mut frames = vec![frame(color_type, 0)];
    for block in BLOCKS {
        let mut next = frames.last().expect("先頭フレームが無い").clone();
        invert_block(&mut next, color_type, block);
        frames.push(next);
    }
    frames
}

/// `block_frames` の各フレームが書き直す矩形
pub fn block_rects() -> Vec<(u32, u32, u32, u32)> {
    std::iter::once(WHOLE).chain(BLOCKS).collect()
}

/// `block_frames` を符号化する
pub fn encode_blocks(config: Config) -> Vec<u8> {
    encode_frames(
        config,
        WIDTH,
        HEIGHT,
        &block_frames(config.color_type),
        &BLOCK_DURATIONS,
    )
}

/// 各フレームのヘッダが示す矩形 (x, y, 幅, 高さ)
pub fn rects(headers: &[FrameHeader]) -> Vec<(u32, u32, u32, u32)> {
    headers
        .iter()
        .map(|header| {
            let (width, height) = header.size();
            (
                header.x0 as u32,
                header.y0 as u32,
                width as u32,
                height as u32,
            )
        })
        .collect()
}

/// 各フレームのtick数
pub fn ticks(decoded: &Decoded) -> Vec<u32> {
    decoded
        .frames
        .iter()
        .map(|frame| frame.duration_ticks)
        .collect()
}

/// 変化のあと1枚だけ書き加える点 (x, y, 幅, 高さ)
///
/// 割れた表示フレームを最終フレームから離し、置き先の欄が書かれる位置へ置く。
pub const TRAILING_DOT: (u32, u32, u32, u32) = (1, 1, 1, 1);

/// 離れた2箇所の変化 (x, y, 幅, 高さ)
///
/// 外接矩形は 38x24 で、割ると 840 画素を書かずに済む。
pub const DISTANT: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (34, 20, 6, 6)];

/// 近い2箇所の変化 (x, y, 幅, 高さ)
///
/// 外接矩形は 14x20 で、割っても 40 画素しか減らない。
pub const NEARBY: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 20), (10, 2, 6, 20)];

/// `NEARBY` の外接矩形
pub const NEARBY_BOUNDS: (u32, u32, u32, u32) = (2, 2, 14, 20);

/// `scattered_frames` の各フレームの表示時間
pub const SCATTERED_DURATIONS: [u32; 3] = [3, 5, 7];

/// 勾配、`blocks` を反転したもの、さらに `TRAILING_DOT` を反転したものの3枚
pub fn scattered_frames(color_type: ColorType, blocks: &[(u32, u32, u32, u32)]) -> Vec<Vec<u8>> {
    let base = frame(color_type, 0);
    let mut scattered = base.clone();
    for block in blocks {
        invert_block(&mut scattered, color_type, *block);
    }
    let mut tail = scattered.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base, scattered, tail]
}

/// `scattered_frames` を符号化する
pub fn encode_scattered(config: Config, blocks: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    encode_frames(
        config,
        WIDTH,
        HEIGHT,
        &scattered_frames(config.color_type, blocks),
        &SCATTERED_DURATIONS,
    )
}

/// 一過性の重なり (x, y, 幅, 高さ)
///
/// 消えたあとのフレームは、重なる前のキャンバスを土台にすれば書き直す画素が無くなる。
pub const POPUP: (u32, u32, u32, u32) = (10, 6, 24, 18);

/// 差分が空になったフレームが書き直す矩形 (x, y, 幅, 高さ)
pub const UNCHANGED: (u32, u32, u32, u32) = (0, 0, 1, 1);

/// 勾配、`POPUP` を反転したもの、勾配へ戻したもの、`TRAILING_DOT` を反転したものの4枚
pub fn popup_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let base = frame(color_type, 0);
    let mut covered = base.clone();
    invert_block(&mut covered, color_type, POPUP);
    let mut tail = base.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base.clone(), covered, base, tail]
}

/// `popup_frames` を符号化する
pub fn encode_popup(config: Config) -> Vec<u8> {
    encode_frames(
        config,
        WIDTH,
        HEIGHT,
        &popup_frames(config.color_type),
        &BLOCK_DURATIONS,
    )
}

/// 各フレームが合成後のキャンバスを置く参照スロット
pub fn saved_slots(headers: &[FrameHeader]) -> Vec<u32> {
    headers
        .iter()
        .map(|header| header.save_as_reference)
        .collect()
}

/// 空の枠と同じ透明な黒のキャンバスに、`block` だけを反転して置いたフレーム
pub fn lone_block(color_type: ColorType, block: (u32, u32, u32, u32)) -> Vec<u8> {
    let mut frame = vec![0u8; (WIDTH * HEIGHT) as usize * color_type.bytes_per_pixel()];
    invert_block(&mut frame, color_type, block);
    frame
}

/// 空の枠を土台にすれば1画素で書けるフレーム列
///
/// 空の枠は透明な黒として読まれるので、2枚目は自分の置き先を土台にすると
/// 書き直す画素がほとんど無くなる。
pub fn lone_block_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let second = lone_block(color_type, LONE_BLOCKS[1]);
    let mut tail = second.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![lone_block(color_type, LONE_BLOCKS[0]), second, tail]
}

/// 透明な黒の上を動く塊 (x, y, 幅, 高さ)
pub const LONE_BLOCKS: [(u32, u32, u32, u32); 2] = [(2, 2, 6, 6), (10, 2, 6, 6)];

/// `LONE_BLOCKS` の外接矩形
pub const LONE_BOUNDS: (u32, u32, u32, u32) = (2, 2, 14, 6);

/// 勾配、全面を反転したもの、勾配へ `DISTANT` を書き加えたもの、`TRAILING_DOT` を
/// さらに反転したものの4枚
///
/// 3枚目は直前のフレームとは全面で食い違い、2つ前のキャンバスとは離れた2箇所しか
/// 違わない。
pub fn flash_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let base = frame(color_type, 0);
    let mut flashed = base.clone();
    invert_block(&mut flashed, color_type, WHOLE);
    let mut scattered = base.clone();
    for block in DISTANT {
        invert_block(&mut scattered, color_type, block);
    }
    let mut tail = scattered.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base, flashed, scattered, tail]
}

/// 層の取り出しにかけるフレーム列
pub struct Sequence {
    pub name: &'static str,
    pub frames: Vec<Vec<u8>>,
    pub durations: &'static [u32],
    /// 書かれる矩形 (x, y, 幅, 高さ)
    pub rects: Vec<(u32, u32, u32, u32)>,
}

/// 全面・矩形1枚・矩形2枚の3つの書き方を、直前と2つ前の2通りの土台で踏む列
pub fn sequences(color_type: ColorType) -> Vec<Sequence> {
    vec![
        // 2枚目は直前を、3枚目は2つ前を土台にした矩形1枚
        Sequence {
            name: "popup",
            frames: popup_frames(color_type),
            durations: &BLOCK_DURATIONS,
            rects: vec![WHOLE, POPUP, UNCHANGED, TRAILING_DOT],
        },
        // 2枚目は直前を土台に矩形2枚へ割れる
        Sequence {
            name: "scattered",
            frames: scattered_frames(color_type, &DISTANT),
            durations: &SCATTERED_DURATIONS,
            rects: vec![WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT],
        },
        // 3枚目は2つ前を土台に矩形2枚へ割れる
        Sequence {
            name: "flash",
            frames: flash_frames(color_type),
            durations: &BLOCK_DURATIONS,
            rects: vec![WHOLE, WHOLE, DISTANT[0], DISTANT[1], TRAILING_DOT],
        },
        // 粗い品質でも3枚目が2つ前を土台にする
        Sequence {
            name: "restored",
            frames: restored_frames(color_type),
            durations: &BLOCK_DURATIONS,
            rects: vec![WHOLE, POPUP, UNCHANGED, TRAILING_DOT],
        },
    ]
}

/// `sequence` のフレームを符号化する
pub fn encode_sequence(config: Config, sequence: &Sequence) -> Vec<u8> {
    encode_frames(config, WIDTH, HEIGHT, &sequence.frames, sequence.durations)
}

/// プラグインの既定に当たる品質
pub const QUALITY: f32 = 90.0;

/// 量子化の誤差が画面に出る粗い品質
pub const COARSE_QUALITY: f32 = 40.0;

/// 一様な背景の値。値域の端なので、平坦な面は入力どおりに復号される
pub const FLAT: u8 = 0;

/// 背景から大きく離れた値
pub const PAINT: u8 = 200;

pub fn lossy(color_type: ColorType, quality: f32) -> Config {
    Config {
        quality,
        ..config(color_type)
    }
}

/// 一様な色で埋めた不透明なフレーム
pub fn flat(color_type: ColorType, value: u8) -> Vec<u8> {
    let mut frame = vec![value; (WIDTH * HEIGHT) as usize * color_type.bytes_per_pixel()];
    if color_type == ColorType::Rgba8 {
        for pixel in frame.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
    }
    frame
}

/// `block` の範囲の色を `value` で塗る
pub fn paint(frame: &mut [u8], color_type: ColorType, block: (u32, u32, u32, u32), value: u8) {
    let (x, y, width, height) = block;
    let bytes_per_pixel = color_type.bytes_per_pixel();
    for row in 0..height as usize {
        let start = ((y as usize + row) * WIDTH as usize + x as usize) * bytes_per_pixel;
        for pixel in
            frame[start..start + width as usize * bytes_per_pixel].chunks_exact_mut(bytes_per_pixel)
        {
            pixel[..3].fill(value);
        }
    }
}

/// `base` の上に `block` を `value` で塗ったフレーム
pub fn painted(
    base: &[u8],
    color_type: ColorType,
    block: (u32, u32, u32, u32),
    value: u8,
) -> Vec<u8> {
    let mut frame = base.to_vec();
    paint(&mut frame, color_type, block, value);
    frame
}

/// 2つの面で最も離れた画素の隔たり
pub fn max_gap(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

/// `block` の範囲を、位置から決まる値で塗る
///
/// 隣り合う画素が離れているので、非可逆では層の色が入力から動く。値は背景から
/// 遠いところだけを通る。
pub fn speckle(frame: &mut [u8], color_type: ColorType, block: (u32, u32, u32, u32)) {
    let (x, y, width, height) = block;
    let bytes_per_pixel = color_type.bytes_per_pixel();
    for row in 0..height {
        for column in 0..width {
            let at = ((y + row) * WIDTH + x + column) as usize * bytes_per_pixel;
            frame[at..at + 3].fill(PAINT + (row * 7 + column * 29) as u8 % 50);
        }
    }
}

/// 一様な背景、`POPUP` を斑に塗ったもの、背景へ戻したもの、`TRAILING_DOT` を
/// 反転したものの4枚
///
/// 背景が平坦で復号しても動かないので、粗い品質でも3枚目が2つ前のキャンバスを
/// 土台にする。
pub fn restored_frames(color_type: ColorType) -> Vec<Vec<u8>> {
    let base = flat(color_type, FLAT);
    let mut covered = base.clone();
    speckle(&mut covered, color_type, POPUP);
    let mut tail = base.clone();
    invert_block(&mut tail, color_type, TRAILING_DOT);
    vec![base.clone(), covered, base, tail]
}
