//! 出力したAPNGを`png`クレートでデコードし、入力フレームと一致することを確認する

use apng_encoder::{ColorType, Config, Encoder, Error, FrameDelay};
use std::io::{self, Cursor, Seek, SeekFrom, Write};

/// 決定的な擬似乱数でフレームの内容を作る
fn frame_data(len: usize, seed: u32) -> Vec<u8> {
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

fn frames(width: u32, height: u32, color_type: ColorType, count: u32) -> Vec<Vec<u8>> {
    let len = width as usize * height as usize * color_type.bytes_per_pixel();
    (0..count).map(|i| frame_data(len, i + 1)).collect()
}

fn config(color_type: ColorType) -> Config {
    Config {
        color_type,
        compression_level: 6,
        num_plays: 0,
        ..Config::default()
    }
}

fn encode(width: u32, height: u32, color_type: ColorType, input: &[Vec<u8>]) -> Vec<u8> {
    let delay = FrameDelay::new(1001, 30000).unwrap();
    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        input.len() as u32,
        config(color_type),
    )
    .unwrap();
    for data in input {
        encoder.add_frame(data, delay).unwrap();
    }
    encoder.finish().unwrap().into_inner()
}

struct DecodedFrame {
    data: Vec<u8>,
    control: png::FrameControl,
}

/// PLTEとtRNSから組み立てた、添字を画素へ戻す表
///
/// tRNSを持つパレットはRGBA8へ、持たないパレットはRGB8へ展開する。
struct Expansion {
    /// 添字順に並べた画素
    entries: Vec<u8>,
    /// 展開後の1画素あたりのバイト数
    bytes_per_pixel: usize,
}

impl Expansion {
    /// 出力がパレット参照でなければ `None`
    fn of(info: &png::Info) -> Option<Self> {
        if info.color_type != png::ColorType::Indexed {
            return None;
        }

        let plte = info.palette.as_deref().expect("PLTEが必要");
        let trns = info.trns.as_deref().unwrap_or(&[]);
        assert_eq!(plte.len() % 3, 0, "PLTEは3バイトずつ");
        assert!(trns.len() <= plte.len() / 3, "tRNSはPLTEより多くならない");

        let bytes_per_pixel = if trns.is_empty() { 3 } else { 4 };
        let mut entries = Vec::with_capacity(plte.len() / 3 * bytes_per_pixel);
        for (index, color) in plte.chunks_exact(3).enumerate() {
            entries.extend_from_slice(color);
            if bytes_per_pixel == 4 {
                // tRNSに無いエントリは不透明とみなす
                entries.push(trns.get(index).copied().unwrap_or(0xFF));
            }
        }

        Some(Expansion {
            entries,
            bytes_per_pixel,
        })
    }

    /// 添字の並びを画素へ展開する
    fn apply(&self, indices: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(indices.len() * self.bytes_per_pixel);
        for &index in indices {
            let at = index as usize * self.bytes_per_pixel;
            out.extend_from_slice(&self.entries[at..at + self.bytes_per_pixel]);
        }
        out
    }
}

/// APNGを読み出し、acTLの再生回数と全フレームを返す
///
/// パレット参照の出力はPLTEとtRNSを引いて画素へ展開する。
fn decode(bytes: &[u8]) -> (u32, Vec<DecodedFrame>) {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let animation = *reader.info().animation_control().expect("acTLが必要");
    let expansion = Expansion::of(reader.info());

    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let mut decoded = Vec::new();
    for _ in 0..animation.num_frames {
        let info = reader.next_frame(&mut buf).unwrap();
        let raw = &buf[..info.buffer_size()];
        decoded.push(DecodedFrame {
            data: match &expansion {
                Some(expansion) => expansion.apply(raw),
                None => raw.to_vec(),
            },
            control: *reader.info().frame_control().expect("fcTLが必要"),
        });
    }

    (animation.num_plays, decoded)
}

/// チャンクの型を出現順に並べる
fn chunk_types(bytes: &[u8]) -> Vec<[u8; 4]> {
    let mut types = Vec::new();
    let mut offset = 8;

    while offset + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        types.push(bytes[offset + 4..offset + 8].try_into().unwrap());
        offset += 12 + len;
    }

    types
}

/// フレームを描いた後、次のフレームを描く前にキャンバスへ施す後始末
struct Disposal {
    /// 戻す行の、キャンバス上の先頭バイト位置と内容
    rows: Vec<(usize, Vec<u8>)>,
}

impl Disposal {
    /// キャンバスをフレームを描く前の内容へ戻す
    fn apply(&self, canvas: &mut [u8]) {
        for (head, row) in &self.rows {
            canvas[*head..*head + row.len()].copy_from_slice(row);
        }
    }
}

/// APNGのblend_op=OVERが定めるアルファ合成
///
/// 完全に透明な前景はキャンバスを残し、不透明な前景と完全に透明なキャンバスは
/// 前景を残す。それ以外は両者を混ぜる。
fn blend_over(foreground: [u8; 4], background: [u8; 4]) -> [u8; 4] {
    let (front_alpha, back_alpha) = (foreground[3] as u32, background[3] as u32);
    if front_alpha == 0 {
        return background;
    }
    if front_alpha == u8::MAX as u32 || back_alpha == 0 {
        return foreground;
    }

    let carried = back_alpha * (u8::MAX as u32 - front_alpha) / u8::MAX as u32;
    let alpha = front_alpha + carried;
    let mut out = [0u8; 4];
    for (channel, out) in out[..3].iter_mut().enumerate() {
        let sum = foreground[channel] as u32 * front_alpha + background[channel] as u32 * carried;
        *out = (sum / alpha) as u8;
    }
    out[3] = alpha as u8;
    out
}

/// フレームの矩形をキャンバスの該当位置へ合成し、dispose_opの後始末を返す
fn composite(
    canvas: &mut [u8],
    frame: &DecodedFrame,
    width: u32,
    color_type: ColorType,
) -> Disposal {
    let bpp = color_type.bytes_per_pixel();
    let stride = width as usize * bpp;
    let row_len = frame.control.width as usize * bpp;
    let head = frame.control.y_offset as usize * stride + frame.control.x_offset as usize * bpp;

    let rows = match frame.control.dispose_op {
        png::DisposeOp::None => Vec::new(),
        png::DisposeOp::Previous => (0..frame.control.height as usize)
            .map(|y| {
                let dst = head + y * stride;
                (dst, canvas[dst..dst + row_len].to_vec())
            })
            .collect(),
        png::DisposeOp::Background => panic!("BACKGROUNDは書き出さない"),
    };

    for y in 0..frame.control.height as usize {
        let dst = head + y * stride;
        let src = y * row_len;
        let row = &frame.data[src..src + row_len];
        match frame.control.blend_op {
            png::BlendOp::Source => canvas[dst..dst + row_len].copy_from_slice(row),
            png::BlendOp::Over => {
                assert_eq!(color_type, ColorType::Rgba8, "OVERはアルファを要する");
                for (x, foreground) in row.chunks_exact(bpp).enumerate() {
                    let at = dst + x * bpp;
                    let blended = blend_over(
                        foreground.try_into().unwrap(),
                        canvas[at..at + bpp].try_into().unwrap(),
                    );
                    canvas[at..at + bpp].copy_from_slice(&blended);
                }
            }
        }
    }

    Disposal { rows }
}

fn assert_roundtrip(width: u32, height: u32, color_type: ColorType, count: u32) {
    let input = frames(width, height, color_type, count);
    let bytes = encode(width, height, color_type, &input);
    let (num_plays, decoded) = decode(&bytes);

    assert_eq!(num_plays, 0);
    assert_eq!(decoded.len(), input.len());

    let mut canvas = vec![0u8; input[0].len()];
    for (index, (frame, expected)) in decoded.iter().zip(&input).enumerate() {
        let disposal = composite(&mut canvas, frame, width, color_type);
        assert_eq!(&canvas, expected, "フレーム {index}");
        disposal.apply(&mut canvas);

        // fcTLとfdATが共有する連番: 先頭フレームはfdATを持たない
        let expected_sequence = if index == 0 { 0 } else { index as u32 * 2 - 1 };
        assert_eq!(frame.control.sequence_number, expected_sequence);
        assert_eq!(
            (frame.control.delay_num, frame.control.delay_den),
            (1001, 30000)
        );
    }
}

#[test]
fn single_pixel_rgba() {
    assert_roundtrip(1, 1, ColorType::Rgba8, 1);
}

#[test]
fn single_pixel_rgb_multiple_frames() {
    assert_roundtrip(1, 1, ColorType::Rgb8, 5);
}

#[test]
fn odd_size_rgb() {
    assert_roundtrip(7, 5, ColorType::Rgb8, 1);
}

#[test]
fn odd_size_rgba_multiple_frames() {
    assert_roundtrip(7, 5, ColorType::Rgba8, 5);
}

#[test]
fn odd_size_rgb_multiple_frames() {
    assert_roundtrip(7, 5, ColorType::Rgb8, 5);
}

#[test]
fn wide_frame_rgba() {
    assert_roundtrip(129, 3, ColorType::Rgba8, 2);
}

#[test]
fn num_plays_reaches_the_animation_control() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        2,
        2,
        1,
        Config {
            num_plays: 7,
            ..config(ColorType::Rgba8)
        },
    )
    .unwrap();
    encoder
        .add_frame(&input[0], FrameDelay::new(1, 30).unwrap())
        .unwrap();
    let bytes = encoder.finish().unwrap().into_inner();

    assert_eq!(decode(&bytes).0, 7);
}

/// u16に収まらないフレームレートも近似されてfcTLに載る
#[test]
fn out_of_range_delay_is_approximated() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let bytes = {
        let mut encoder =
            Encoder::new(Cursor::new(Vec::new()), 2, 2, 1, config(ColorType::Rgba8)).unwrap();
        encoder
            .add_frame(&input[0], FrameDelay::new(1001, 120000).unwrap())
            .unwrap();
        encoder.finish().unwrap().into_inner()
    };

    let (_, decoded) = decode(&bytes);
    let control = decoded[0].control;
    assert_eq!((control.delay_num, control.delay_den), (342, 40999));
}

/// 1フレームのバイト数が`usize`で表現できない大きさは`new`の時点で弾く
#[test]
fn oversized_image_is_rejected() {
    assert!(matches!(
        Encoder::new(
            Cursor::new(Vec::new()),
            u32::MAX,
            u32::MAX,
            1,
            config(ColorType::Rgba8)
        ),
        Err(Error::ImageTooLarge {
            width: u32::MAX,
            height: u32::MAX
        })
    ));
}

/// 設定した圧縮レベルがdeflateまで届いていること
#[test]
fn compression_level_changes_the_output_size() {
    let (width, height) = (128, 128);
    let len = width as usize * height as usize * ColorType::Rgba8.bytes_per_pixel();
    // 圧縮しやすい階調でレベル差を出す
    let data: Vec<u8> = (0..len).map(|i| (i / 7) as u8).collect();

    let size = |compression_level| {
        let mut encoder = Encoder::new(
            Cursor::new(Vec::new()),
            width,
            height,
            1,
            Config {
                compression_level,
                ..config(ColorType::Rgba8)
            },
        )
        .unwrap();
        encoder
            .add_frame(&data, FrameDelay::new(1, 30).unwrap())
            .unwrap();
        encoder.finish().unwrap().into_inner().len()
    };

    let (low, middle, high) = (size(1), size(6), size(9));
    assert!(
        high < middle,
        "レベル9 ({high}) はレベル6 ({middle}) より小さいこと"
    );
    assert!(
        middle < low,
        "レベル6 ({middle}) はレベル1 ({low}) より小さいこと"
    );
}

/// 一定バイト数まで受け付け、それ以降は必ず失敗する書き出し先
struct FailingWriter {
    remaining: usize,
    position: u64,
}

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.remaining {
            self.remaining = 0;
            return Err(io::Error::other("書き出し失敗"));
        }
        self.remaining -= buf.len();
        self.position += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 書き出した位置だけを追う。戻った先を書き換えても内容は残らない
impl Seek for FailingWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.position = match pos {
            SeekFrom::Start(at) => at,
            SeekFrom::Current(offset) | SeekFrom::End(offset) => self
                .position
                .checked_add_signed(offset)
                .ok_or_else(|| io::Error::other("位置が範囲外"))?,
        };
        Ok(self.position)
    }
}

/// fcTLだけが書かれた状態で再開すると不正なAPNGになるため、失敗後は受け付けない
///
/// 先頭フレームはdispose_opが決まる2フレーム目の投入まで書き出されない。
#[test]
fn a_failed_write_poisons_the_encoder() {
    let input = frames(8, 8, ColorType::Rgba8, 3);
    let delay = FrameDelay::new(1, 30).unwrap();
    // シグネチャ・IHDR・acTL・fcTLは通り、IDATの途中で失敗する長さ
    let writer = FailingWriter {
        remaining: 100,
        position: 0,
    };
    let mut encoder = Encoder::new(writer, 8, 8, 3, config(ColorType::Rgba8)).unwrap();

    encoder.add_frame(&input[0], delay).unwrap();
    assert!(matches!(
        encoder.add_frame(&input[1], delay),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        encoder.add_frame(&input[2], delay),
        Err(Error::Poisoned)
    ));
    assert!(matches!(encoder.finish(), Err(Error::Poisoned)));
}

#[test]
fn frame_of_the_wrong_size_is_rejected() {
    let mut encoder =
        Encoder::new(Cursor::new(Vec::new()), 4, 4, 1, config(ColorType::Rgba8)).unwrap();
    assert!(matches!(
        encoder.add_frame(&[0u8; 63], FrameDelay::new(1, 30).unwrap()),
        Err(Error::FrameSizeMismatch {
            expected: 64,
            actual: 63
        })
    ));
}

#[test]
fn extra_frame_is_rejected() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let delay = FrameDelay::new(1, 30).unwrap();
    let mut encoder =
        Encoder::new(Cursor::new(Vec::new()), 2, 2, 1, config(ColorType::Rgba8)).unwrap();
    encoder.add_frame(&input[0], delay).unwrap();

    assert!(matches!(
        encoder.add_frame(&input[0], delay),
        Err(Error::FrameCountMismatch {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn missing_frame_is_rejected_on_finish() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let mut encoder =
        Encoder::new(Cursor::new(Vec::new()), 2, 2, 3, config(ColorType::Rgba8)).unwrap();
    encoder
        .add_frame(&input[0], FrameDelay::new(1, 30).unwrap())
        .unwrap();

    assert!(matches!(
        encoder.finish(),
        Err(Error::FrameCountMismatch {
            expected: 3,
            actual: 1
        })
    ));
}

#[test]
fn invalid_parameters_are_rejected() {
    let rgba = config(ColorType::Rgba8);
    assert!(matches!(
        Encoder::new(Cursor::new(Vec::new()), 0, 4, 1, rgba),
        Err(Error::InvalidDimensions {
            width: 0,
            height: 4
        })
    ));
    assert!(matches!(
        Encoder::new(Cursor::new(Vec::new()), 4, 4, 0, rgba),
        Err(Error::InvalidFrameCount)
    ));
    assert!(matches!(
        Encoder::new(
            Cursor::new(Vec::new()),
            4,
            4,
            1,
            Config {
                compression_level: 10,
                ..rgba
            }
        ),
        Err(Error::InvalidCompressionLevel(10))
    ));
}

/// 差分クロップの検証に使うキャンバスの大きさ
const CROP_WIDTH: u32 = 8;
const CROP_HEIGHT: u32 = 6;

/// 一様な背景のフレームを作る
fn solid(color_type: ColorType, value: u8) -> Vec<u8> {
    let len = CROP_WIDTH as usize * CROP_HEIGHT as usize * color_type.bytes_per_pixel();
    vec![value; len]
}

/// 指定した画素の全チャンネルを書き換える
fn set_pixel(frame: &mut [u8], color_type: ColorType, x: u32, y: u32, value: u8) {
    let bpp = color_type.bytes_per_pixel();
    let start = (y as usize * CROP_WIDTH as usize + x as usize) * bpp;
    frame[start..start + bpp].fill(value);
}

/// 各フレームのfcTLが示す矩形 (x, y, 幅, 高さ)
fn rects(decoded: &[DecodedFrame]) -> Vec<(u32, u32, u32, u32)> {
    decoded
        .iter()
        .map(|frame| {
            let c = frame.control;
            (c.x_offset, c.y_offset, c.width, c.height)
        })
        .collect()
}

/// フレーム列を符号化し、fcTLの矩形と合成結果の両方を確かめる
fn assert_crop(color_type: ColorType, input: &[Vec<u8>], expected: &[(u32, u32, u32, u32)]) {
    let bytes = encode(CROP_WIDTH, CROP_HEIGHT, color_type, input);
    let (_, decoded) = decode(&bytes);

    assert_eq!(rects(&decoded), expected, "{color_type:?}");

    let mut canvas = vec![0u8; input[0].len()];
    for (index, (frame, source)) in decoded.iter().zip(input).enumerate() {
        let disposal = composite(&mut canvas, frame, CROP_WIDTH, color_type);
        assert_eq!(&canvas, source, "{color_type:?} フレーム {index}");
        disposal.apply(&mut canvas);
    }
}

const WHOLE: (u32, u32, u32, u32) = (0, 0, CROP_WIDTH, CROP_HEIGHT);

/// 差分の無いフレームは1x1の矩形になり、フレーム数はそのまま保たれる
#[test]
fn identical_frames_are_written_as_a_unit_rect() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let input = vec![solid(color_type, 0x40); 3];
        assert_crop(color_type, &input, &[WHOLE, (0, 0, 1, 1), (0, 0, 1, 1)]);
    }
}

#[test]
fn a_single_pixel_change_is_cropped() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = solid(color_type, 0x40);
        let mut changed = base.clone();
        set_pixel(&mut changed, color_type, 3, 2, 0xFF);

        assert_crop(color_type, &[base, changed], &[WHOLE, (3, 2, 1, 1)]);
    }
}

#[test]
fn corner_changes_are_cropped() {
    let corners = [
        (0, 0),
        (CROP_WIDTH - 1, 0),
        (0, CROP_HEIGHT - 1),
        (CROP_WIDTH - 1, CROP_HEIGHT - 1),
    ];
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let mut input = vec![solid(color_type, 0x40)];
        let mut expected = vec![WHOLE];
        for (x, y) in corners {
            let mut frame = input.last().unwrap().clone();
            set_pixel(&mut frame, color_type, x, y, 0xFF);
            input.push(frame);
            expected.push((x, y, 1, 1));
        }

        assert_crop(color_type, &input, &expected);
    }
}

#[test]
fn an_edge_row_change_is_cropped() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = solid(color_type, 0x40);
        let mut changed = base.clone();
        for x in 0..CROP_WIDTH {
            set_pixel(&mut changed, color_type, x, CROP_HEIGHT - 1, 0xFF);
        }

        assert_crop(
            color_type,
            &[base, changed],
            &[WHOLE, (0, CROP_HEIGHT - 1, CROP_WIDTH, 1)],
        );
    }
}

#[test]
fn an_edge_column_change_is_cropped() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = solid(color_type, 0x40);
        let mut changed = base.clone();
        for y in 0..CROP_HEIGHT {
            set_pixel(&mut changed, color_type, CROP_WIDTH - 1, y, 0xFF);
        }

        assert_crop(
            color_type,
            &[base, changed],
            &[WHOLE, (CROP_WIDTH - 1, 0, 1, CROP_HEIGHT)],
        );
    }
}

#[test]
fn a_full_change_covers_the_canvas() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let input = [solid(color_type, 0x40), solid(color_type, 0x80)];
        assert_crop(color_type, &input, &[WHOLE, WHOLE]);
    }
}

/// 離れた2画素の外接矩形は、変更されていない画素も含めて書き直す
#[test]
fn disjoint_changes_span_a_bounding_rect() {
    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = solid(color_type, 0x40);
        let mut changed = base.clone();
        set_pixel(&mut changed, color_type, 1, 1, 0xFF);
        set_pixel(&mut changed, color_type, 6, 4, 0xFF);

        assert_crop(color_type, &[base, changed], &[WHOLE, (1, 1, 6, 4)]);
    }
}

/// 部分矩形が連続するフレーム列
///
/// 毎フレーム別の位置へ擬似乱数で埋めた3x3のブロックを書き加える。矩形の
/// 位置・内容がフレームごとに変わるため、行オフセットと切り出しバッファの
/// 両方が正しくないと合成結果が一致しない。
#[test]
fn consecutive_partial_rects_carry_their_own_content() {
    const BLOCK: u32 = 3;
    const POSITIONS: [(u32, u32); 4] = [(0, 0), (5, 0), (0, 3), (5, 3)];

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let bpp = color_type.bytes_per_pixel();
        let mut input = vec![solid(color_type, 0x40)];
        let mut expected = vec![WHOLE];

        for (index, &(bx, by)) in POSITIONS.iter().enumerate() {
            let mut frame = input.last().unwrap().clone();
            let values = frame_data((BLOCK * BLOCK) as usize * bpp, index as u32 + 1);
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let src = (y * BLOCK + x) as usize * bpp;
                    let dst = ((by + y) as usize * CROP_WIDTH as usize + (bx + x) as usize) * bpp;
                    for ch in 0..bpp {
                        // 背景と必ず異なる値にして、矩形が縮まないようにする
                        frame[dst + ch] = 0x80 | (values[src + ch] & 0x3F);
                    }
                }
            }
            input.push(frame);
            expected.push((bx, by, BLOCK, BLOCK));
        }

        assert_crop(color_type, &input, &expected);
    }
}

/// 各フレームのfcTLが示すdispose_op
fn dispose_ops(decoded: &[DecodedFrame]) -> Vec<png::DisposeOp> {
    decoded
        .iter()
        .map(|frame| frame.control.dispose_op)
        .collect()
}

/// `value` で埋めた `block` の領域を持つフレームを作る
fn with_block(color_type: ColorType, block: (u32, u32, u32, u32), value: u8) -> Vec<u8> {
    let mut frame = solid(color_type, 0x40);
    for y in 0..block.3 {
        for x in 0..block.2 {
            set_pixel(&mut frame, color_type, block.0 + x, block.1 + y, value);
        }
    }
    frame
}

/// 1フレームだけ現れる領域はPREVIOUSで捨て、次のフレームで塗り戻さない
///
/// 捨てたフレームの次はそれを描く前のキャンバスとの差分になるため、内容が戻った
/// フレームは差分を持たない。
#[test]
fn a_transient_region_is_disposed_to_previous() {
    const BLOCK: (u32, u32, u32, u32) = (2, 1, 3, 2);
    const UNIT: (u32, u32, u32, u32) = (0, 0, 1, 1);

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let base = solid(color_type, 0x40);
        let marked = with_block(color_type, BLOCK, 0xFF);
        let input = vec![base.clone(), marked.clone(), base.clone(), marked, base];

        let bytes = encode(CROP_WIDTH, CROP_HEIGHT, color_type, &input);
        let (_, decoded) = decode(&bytes);

        // 最終フレームは戻す先が無いため捨てない
        assert_eq!(
            dispose_ops(&decoded),
            [
                png::DisposeOp::None,
                png::DisposeOp::Previous,
                png::DisposeOp::None,
                png::DisposeOp::Previous,
                png::DisposeOp::None
            ],
            "{color_type:?}"
        );
        assert_eq!(
            rects(&decoded),
            [WHOLE, BLOCK, UNIT, BLOCK, UNIT],
            "{color_type:?}"
        );
        assert_composites_to(&bytes, CROP_WIDTH, color_type, &input);
    }
}

/// 変更が積み上がる列では、捨てると矩形が広がるため捨てない
#[test]
fn cumulative_changes_are_never_disposed() {
    const POSITIONS: [(u32, u32); 3] = [(1, 1), (6, 4), (3, 2)];

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        let mut input = vec![solid(color_type, 0x40)];
        for (index, &(x, y)) in POSITIONS.iter().enumerate() {
            let mut frame = input.last().unwrap().clone();
            set_pixel(&mut frame, color_type, x, y, 0x80 + index as u8);
            input.push(frame);
        }

        let bytes = encode(CROP_WIDTH, CROP_HEIGHT, color_type, &input);
        let (_, decoded) = decode(&bytes);

        assert!(
            dispose_ops(&decoded)
                .iter()
                .all(|op| matches!(op, png::DisposeOp::None)),
            "{color_type:?}"
        );
        assert_composites_to(&bytes, CROP_WIDTH, color_type, &input);
    }
}

/// 一様で広い領域
const WIDE: (u32, u32, u32, u32) = (0, 0, CROP_WIDTH, 3);
/// 擬似乱数で埋めた狭い領域
const NARROW: (u32, u32, u32, u32) = (2, 4, 4, 2);

/// 圧縮後の大きさが面積と逆に並ぶ3フレームを作る
///
/// 最後のフレームは [`NARROW`] だけが擬似乱数で、残りは一様。1つ目は [`WIDE`] が、
/// 2つ目は [`NARROW`] が最後のフレームと違う。最後のフレームから切り出すと、
/// 広い [`WIDE`] の方が狭い [`NARROW`] より小さく圧縮される。
fn rects_ordered_against_their_size() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let color_type = ColorType::Rgb8;
    let bpp = color_type.bytes_per_pixel();
    let noise = frame_data(NARROW.2 as usize * NARROW.3 as usize * bpp, 1);

    let mut last = solid(color_type, 0x40);
    for y in 0..NARROW.3 {
        for x in 0..NARROW.2 {
            let src = ((y * NARROW.2 + x) as usize) * bpp;
            let dst = (((NARROW.1 + y) * CROP_WIDTH + NARROW.0 + x) as usize) * bpp;
            last[dst..dst + bpp].copy_from_slice(&noise[src..src + bpp]);
        }
    }

    let mut wide_differs = last.clone();
    for y in 0..WIDE.3 {
        for x in 0..WIDE.2 {
            set_pixel(&mut wide_differs, color_type, x, y, 0x80);
        }
    }

    let mut narrow_differs = last.clone();
    for y in 0..NARROW.3 {
        for x in 0..NARROW.2 {
            set_pixel(
                &mut narrow_differs,
                color_type,
                NARROW.0 + x,
                NARROW.1 + y,
                0x40,
            );
        }
    }

    (wide_differs, narrow_differs, last)
}

/// 捨てた場合の矩形が広いときは、圧縮して比べる前に候補から外す
///
/// 広い方が小さく圧縮できる場合でも、面積で先に落とす。
#[test]
fn a_wider_restored_rect_is_not_tried() {
    let (wide_differs, narrow_differs, last) = rects_ordered_against_their_size();
    let input = vec![wide_differs, narrow_differs, last];

    let bytes = encode(CROP_WIDTH, CROP_HEIGHT, ColorType::Rgb8, &input);
    let (_, decoded) = decode(&bytes);

    assert!(
        dispose_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::DisposeOp::None))
    );
    assert_eq!(rects(&decoded)[2], NARROW);
    assert_composites_to(&bytes, CROP_WIDTH, ColorType::Rgb8, &input);
}

/// 捨てた場合の矩形が狭くても、圧縮後に大きくなるなら捨てない
#[test]
fn a_restored_rect_that_compresses_larger_is_not_taken() {
    let (wide_differs, narrow_differs, last) = rects_ordered_against_their_size();
    let input = vec![narrow_differs, wide_differs, last];

    let bytes = encode(CROP_WIDTH, CROP_HEIGHT, ColorType::Rgb8, &input);
    let (_, decoded) = decode(&bytes);

    assert!(
        dispose_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::DisposeOp::None))
    );
    assert_eq!(rects(&decoded)[2], WIDE);
    assert_composites_to(&bytes, CROP_WIDTH, ColorType::Rgb8, &input);
}

/// 先頭フレームのdispose_opにPREVIOUSを選ばない
///
/// 先頭のfcTLのPREVIOUSはBACKGROUNDとして扱われ、キャンバスは復元されない。
/// 2フレーム目を全面で異なる擬似乱数にすると、捨てた場合の候補が1画素に縮んで
/// 圧縮後の大きさで必ず勝つ。
#[test]
fn the_first_frame_is_never_disposed_to_previous() {
    let color_type = ColorType::Rgb8;
    let len = CROP_WIDTH as usize * CROP_HEIGHT as usize * color_type.bytes_per_pixel();
    // 一様な背景と必ず異なるよう、全バイトを奇数にする
    let noisy: Vec<u8> = frame_data(len, 1).iter().map(|b| b | 1).collect();
    let input = vec![solid(color_type, 0x40), noisy.clone(), noisy];

    let bytes = encode(CROP_WIDTH, CROP_HEIGHT, color_type, &input);
    let (_, decoded) = decode(&bytes);

    assert_eq!(dispose_ops(&decoded)[0], png::DisposeOp::None);
    assert_eq!(rects(&decoded)[1], WHOLE);
    assert_composites_to(&bytes, CROP_WIDTH, color_type, &input);
}

/// 圧縮後の大きさが同じなら、保留中のフレームを捨てない
#[test]
fn a_tie_keeps_the_pending_frame() {
    const ROWS: u32 = 5;

    let color_type = ColorType::Rgb8;
    let last = solid(color_type, 0x40);
    let middle = solid(color_type, 0x80);
    let mut first = last.clone();
    for y in 0..ROWS {
        for x in 0..CROP_WIDTH {
            set_pixel(&mut first, color_type, x, y, 0x80);
        }
    }

    let input = vec![first, middle, last];
    let bytes = encode(CROP_WIDTH, CROP_HEIGHT, color_type, &input);
    let (_, decoded) = decode(&bytes);

    assert!(
        dispose_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::DisposeOp::None))
    );
    // 捨てれば矩形は上から ROWS 行に縮むが、一様なので圧縮後は全面と同じ大きさになる
    assert_eq!(rects(&decoded)[2], WHOLE);
    assert_composites_to(&bytes, CROP_WIDTH, color_type, &input);
}

/// blend_opの検証に使うキャンバスの大きさ
///
/// 潰した画素の並びが圧縮に効くだけの画素を取る。
const BLEND_WIDTH: u32 = 24;
const BLEND_HEIGHT: u32 = 16;
/// 1フレームの画素数
const BLEND_PIXELS: usize = BLEND_WIDTH as usize * BLEND_HEIGHT as usize;

/// 画素ごとに違う不透明な色を敷いたRGBA8のフレーム
///
/// 隣り合う画素が揃わないため、そのまま書くとほとんど縮まない。
fn blend_frame(seed: u32) -> Vec<u8> {
    frame_data(BLEND_PIXELS * 3, seed)
        .chunks_exact(3)
        .flat_map(|color| [color[0], color[1], color[2], 0xFF])
        .collect()
}

/// RGBA8の1画素を書き換える
fn set_rgba(frame: &mut [u8], x: u32, y: u32, color: [u8; 4]) {
    let at = (y as usize * BLEND_WIDTH as usize + x as usize) * 4;
    frame[at..at + 4].copy_from_slice(&color);
}

/// パレットに収まるフレームが使う色数
///
/// 塗り替える帯と合わせるとパレットに収まらない色数へ届く。
const PALETTE_BLEND_COLORS: u8 = 128;

/// [`PALETTE_BLEND_COLORS`] 色から画素ごとに選んで敷いた不透明なRGBA8のフレーム
///
/// 隣り合う画素が揃わないのは [`blend_frame`] と同じで、色数はパレットに収まる。
fn palette_blend_frame(seed: u32) -> Vec<u8> {
    frame_data(BLEND_PIXELS, seed)
        .iter()
        .flat_map(|&byte| {
            let index = byte % PALETTE_BLEND_COLORS;
            [index, index.wrapping_mul(3), index.wrapping_mul(7), 0xFF]
        })
        .collect()
}

/// 上端の帯を、パレットに収まらない色数になるまで塗り替えたフレーム
///
/// 帯の中は1画素残らず変わるため、矩形はその帯に一致してOVERの候補が立たない。
fn with_color_burst(base: &[u8]) -> Vec<u8> {
    /// 塗り替える行数
    const ROWS: u32 = 8;

    let mut frame = base.to_vec();
    for y in 0..ROWS {
        for x in 0..BLEND_WIDTH {
            let color = MAX_PALETTE_COLORS + (y * BLEND_WIDTH + x) as usize;
            set_rgba(
                &mut frame,
                x,
                y,
                [color as u8, (color >> 8) as u8, (color >> 16) as u8, 0xFF],
            );
        }
    }
    frame
}

/// 矩形の対角にある2画素だけを不透明な色へ書き換えたフレーム
///
/// 矩形はキャンバスのほぼ全体に広がり、その中のほとんどの画素が変化しない。
fn with_opaque_corners(base: &[u8]) -> Vec<u8> {
    let mut frame = base.to_vec();
    set_rgba(&mut frame, 1, 1, [0xFF, 0x00, 0x00, 0xFF]);
    set_rgba(
        &mut frame,
        BLEND_WIDTH - 2,
        BLEND_HEIGHT - 2,
        [0x00, 0xFF, 0x00, 0xFF],
    );
    frame
}

/// 各フレームのfcTLが示すblend_op
fn blend_ops(decoded: &[DecodedFrame]) -> Vec<png::BlendOp> {
    decoded.iter().map(|frame| frame.control.blend_op).collect()
}

/// RGBA8のフレーム列を符号化し、blend_opの並びと合成結果の両方を確かめる
fn assert_blend(input: &[Vec<u8>], expected: &[png::BlendOp]) {
    let bytes = encode(BLEND_WIDTH, BLEND_HEIGHT, ColorType::Rgba8, input);
    let (_, decoded) = decode(&bytes);

    assert_eq!(blend_ops(&decoded), expected);
    assert_composites_to(&bytes, BLEND_WIDTH, ColorType::Rgba8, input);
}

/// 変化した画素がわずかな矩形は、潰した候補の方が小さくOVERで書かれる
#[test]
fn a_mostly_unchanged_rect_is_written_with_over() {
    let base = blend_frame(1);
    let changed = with_opaque_corners(&base);

    assert_blend(
        &[base, changed],
        &[png::BlendOp::Source, png::BlendOp::Over],
    );
}

/// OVERで書いた矩形では、変化していない画素が完全な透明に潰れている
#[test]
fn unchanged_pixels_inside_an_over_rect_are_transparent() {
    let base = blend_frame(1);
    let changed = with_opaque_corners(&base);
    let bytes = encode(
        BLEND_WIDTH,
        BLEND_HEIGHT,
        ColorType::Rgba8,
        &[base, changed],
    );
    let (_, decoded) = decode(&bytes);

    assert_eq!(blend_ops(&decoded)[1], png::BlendOp::Over);
    // 矩形は (1, 1) から対角の画素までで、その両端だけが変化している
    let region = &decoded[1].data;
    assert_eq!(
        region.len(),
        (BLEND_WIDTH as usize - 2) * (BLEND_HEIGHT as usize - 2) * 4
    );
    assert_eq!(&region[..4], &[0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(&region[region.len() - 4..], &[0x00, 0xFF, 0x00, 0xFF]);
    assert!(region[4..region.len() - 4].iter().all(|&byte| byte == 0));
}

/// 変化していない画素は、アルファがいくつでもキャンバスから復元される
///
/// 完全に透明な画素も半透明の画素も潰す先は同じで、キャンバスの側が残る。
#[test]
fn unchanged_pixels_of_any_alpha_survive_over() {
    let mut base = blend_frame(1);
    set_rgba(&mut base, 4, 4, [0x00, 0x00, 0x00, 0x00]);
    set_rgba(&mut base, 5, 4, [0x11, 0x22, 0x33, 0x00]);
    set_rgba(&mut base, 6, 4, [0x44, 0x55, 0x66, 0x40]);
    let changed = with_opaque_corners(&base);

    assert_blend(
        &[base, changed],
        &[png::BlendOp::Source, png::BlendOp::Over],
    );
}

/// 完全に透明へ変わった画素があればSOURCEで書く
#[test]
fn a_transparent_change_falls_back_to_source() {
    let base = blend_frame(1);
    let mut changed = with_opaque_corners(&base);
    set_rgba(&mut changed, 8, 8, [0x00, 0x00, 0x00, 0x00]);

    assert_blend(
        &[base, changed],
        &[png::BlendOp::Source, png::BlendOp::Source],
    );
}

/// 完全に透明でRGBが残る画素へ変わってもSOURCEで書く
#[test]
fn a_transparent_change_that_keeps_its_color_falls_back_to_source() {
    let base = blend_frame(1);
    let mut changed = with_opaque_corners(&base);
    set_rgba(&mut changed, 8, 8, [0x77, 0x88, 0x99, 0x00]);

    assert_blend(
        &[base, changed],
        &[png::BlendOp::Source, png::BlendOp::Source],
    );
}

/// 矩形の中に半透明へ変わった画素が1つでもあればSOURCEで書く
#[test]
fn a_semi_transparent_change_falls_back_to_source() {
    let base = blend_frame(1);
    let mut changed = with_opaque_corners(&base);
    set_rgba(&mut changed, 8, 8, [0x77, 0x88, 0x99, 0x80]);

    assert_blend(
        &[base, changed],
        &[png::BlendOp::Source, png::BlendOp::Source],
    );
}

/// 変化していない画素が1つも無い矩形はSOURCEで書く
#[test]
fn a_rect_without_an_unchanged_pixel_stays_on_source() {
    let base = blend_frame(1);
    let changed: Vec<u8> = base
        .chunks_exact(4)
        .flat_map(|p| [p[0] ^ 0xFF, p[1], p[2], 0xFF])
        .collect();

    assert_blend(
        &[base, changed],
        &[png::BlendOp::Source, png::BlendOp::Source],
    );
}

/// 潰しても切り出した内容が変わらない矩形はSOURCEで書く
///
/// 変化していない画素が元から完全な透明なら、潰す前と後で1バイトも変わらず、
/// 圧縮後の大きさも並ぶ。
#[test]
fn a_tie_keeps_the_source_blend() {
    let clear = vec![0u8; BLEND_PIXELS * 4];
    let changed = with_opaque_corners(&clear);

    assert_blend(
        &[clear, changed],
        &[png::BlendOp::Source, png::BlendOp::Source],
    );
}

/// 潰した画素が並びを乱す矩形は、そのまま書いた方が小さくSOURCEで書く
///
/// 変化していない画素が散らばっていると、潰した跡が周期的な穴になる。
#[test]
fn an_over_candidate_that_compresses_larger_is_not_taken() {
    const STEP: usize = 7;

    let uniform = [0x30u8, 0x40, 0x50, 0xFF].repeat(BLEND_PIXELS);
    let mut speckled = uniform.clone();
    for pixel in (0..BLEND_PIXELS).step_by(STEP) {
        speckled[pixel * 4..pixel * 4 + 4].copy_from_slice(&[0xC0, 0xB0, 0xA0, 0xFF]);
    }

    assert_blend(
        &[speckled, uniform],
        &[png::BlendOp::Source, png::BlendOp::Source],
    );
}

/// 捨てたフレームの次は、復元されたキャンバスとの差分をOVERで書く
///
/// 重ねる先を取り違えると、捨てたフレームと同じ内容の画素が潰れてしまい、
/// 復元されたキャンバスの側が残って元の値に戻らない。
#[test]
fn an_over_rect_after_a_disposal_is_layered_on_the_restored_canvas() {
    const MARK: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
    const SPOT: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
    /// 1フレームだけ現れる帯の上端
    const BAND: u32 = 9;

    let base = blend_frame(1);
    // 離れた2画素だけを書き換えたフレーム。矩形はその外接矩形に広がる
    let mut restored = base.clone();
    set_rgba(&mut restored, 2, 2, MARK);
    set_rgba(&mut restored, 7, 7, SPOT);
    // 同じ2画素に加えて、より広い帯を書き換えたフレーム
    let mut transient = restored.clone();
    let overlay = blend_frame(2);
    for y in BAND..BLEND_HEIGHT {
        for x in 0..BLEND_WIDTH {
            let at = (y as usize * BLEND_WIDTH as usize + x as usize) * 4;
            let color = [overlay[at], overlay[at + 1], overlay[at + 2], 0xFF];
            set_rgba(&mut transient, x, y, color);
        }
    }

    let input = vec![base, transient, restored];
    let bytes = encode(BLEND_WIDTH, BLEND_HEIGHT, ColorType::Rgba8, &input);
    let (_, decoded) = decode(&bytes);

    assert_eq!(decoded[1].control.dispose_op, png::DisposeOp::Previous);
    assert_eq!(blend_ops(&decoded)[2], png::BlendOp::Over);
    assert_composites_to(&bytes, BLEND_WIDTH, ColorType::Rgba8, &input);
}

/// パレット参照の出力でも、変化した画素がわずかな矩形はOVERで書く
///
/// 潰した画素はアルファが0の色の添字で書く。その色が載っていることが条件になる。
#[test]
fn a_palette_with_a_transparent_color_is_written_with_over() {
    let mut base = palette_blend_frame(1);
    set_rgba(&mut base, 0, 0, [0x00, 0x00, 0x00, 0x00]);
    let changed = with_opaque_corners(&base);

    let input = vec![base, changed];
    let bytes = encode_with(
        BLEND_WIDTH,
        BLEND_HEIGHT,
        reduce_config(ColorType::Rgba8),
        &input,
    );

    assert_eq!(output_color_type(&bytes), png::ColorType::Indexed);
    let (_, decoded) = decode(&bytes);
    assert_eq!(
        blend_ops(&decoded),
        [png::BlendOp::Source, png::BlendOp::Over]
    );
    assert_composites_to(&bytes, BLEND_WIDTH, ColorType::Rgba8, &input);
}

/// アルファが0の色が載っていないパレットは、潰した画素を書けないのでOVERを使わない
#[test]
fn a_palette_without_a_transparent_color_stays_on_source() {
    let base = palette_blend_frame(1);
    let changed = with_opaque_corners(&base);

    let input = vec![base, changed];
    let bytes = encode_with(
        BLEND_WIDTH,
        BLEND_HEIGHT,
        reduce_config(ColorType::Rgba8),
        &input,
    );

    assert_eq!(output_color_type(&bytes), png::ColorType::Indexed);
    let (_, decoded) = decode(&bytes);
    assert!(
        blend_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::BlendOp::Source))
    );
    assert_composites_to(&bytes, BLEND_WIDTH, ColorType::Rgba8, &input);
}

/// パレット参照の出力でも、1フレームだけ現れる領域はPREVIOUSで捨てる
#[test]
fn a_palette_is_disposed_to_previous() {
    const BLOCK: (u32, u32, u32, u32) = (2, 1, 3, 2);
    const TRIGGER: (u32, u32) = (20, 13);

    // 塗り替える2色を足してもパレットに収まる色数
    let base = palette_frame(MAX_PALETTE_COLORS - 8, 0);
    let mut marked = base.clone();
    for y in 0..BLOCK.3 as usize {
        for x in 0..BLOCK.2 as usize {
            let start = ((BLOCK.1 as usize + y) * REDUCE_WIDTH as usize + BLOCK.0 as usize + x) * 4;
            marked[start..start + 3].fill(0x10);
        }
    }
    let mut restored = base.clone();
    let trigger = (TRIGGER.1 as usize * REDUCE_WIDTH as usize + TRIGGER.0 as usize) * 4;
    restored[trigger..trigger + 3].fill(0x20);

    let input = vec![base, marked, restored];
    let bytes = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8),
        &input,
    );

    let (_, decoded) = decode(&bytes);
    assert_eq!(
        dispose_ops(&decoded),
        [
            png::DisposeOp::None,
            png::DisposeOp::Previous,
            png::DisposeOp::None
        ]
    );
    assert_eq!(
        rects(&decoded),
        [
            (0, 0, REDUCE_WIDTH, REDUCE_HEIGHT),
            BLOCK,
            (TRIGGER.0, TRIGGER.1, 1, 1)
        ]
    );
    assert_reduced_roundtrip(&bytes, &input);
}

/// 捨てたフレームにしか無い内容は、復元されたキャンバスとの差分として残る
///
/// 重ねる先を捨てたフレームにすると、そこと一致する画素が潰れて矩形から消える。
#[test]
fn an_over_rect_after_a_disposal_keeps_what_the_restored_canvas_lacks() {
    const MARK: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
    const SPOT: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
    /// 1フレームだけ現れる帯の上端
    const BAND: u32 = 9;

    let base = blend_frame(3);
    let mut restored = base.clone();
    set_rgba(&mut restored, 3, 2, MARK);
    set_rgba(&mut restored, 9, 6, SPOT);
    let mut transient = restored.clone();
    let overlay = blend_frame(4);
    for y in BAND..BLEND_HEIGHT {
        for x in 0..BLEND_WIDTH {
            let at = (y as usize * BLEND_WIDTH as usize + x as usize) * 4;
            let color = [overlay[at], overlay[at + 1], overlay[at + 2], 0xFF];
            set_rgba(&mut transient, x, y, color);
        }
    }

    let input = vec![base, transient, restored];
    let bytes = encode(BLEND_WIDTH, BLEND_HEIGHT, ColorType::Rgba8, &input);
    let (_, decoded) = decode(&bytes);

    assert_eq!(decoded[1].control.dispose_op, png::DisposeOp::Previous);
    assert_eq!(blend_ops(&decoded)[2], png::BlendOp::Over);
    // 矩形は2画素の外接矩形で、その両端だけが復元されたキャンバスと違う
    let control = decoded[2].control;
    assert_eq!((control.x_offset, control.y_offset), (3, 2));
    assert_eq!((control.width, control.height), (7, 5));
    let region = &decoded[2].data;
    assert_eq!(region.len(), 7 * 5 * 4);
    assert_eq!(&region[..4], &MARK);
    assert_eq!(&region[region.len() - 4..], &SPOT);
    assert!(region[4..region.len() - 4].iter().all(|&byte| byte == 0));
}

/// アルファを持たない出力にはOVERを使わない
#[test]
fn an_output_without_alpha_is_never_written_with_over() {
    let base: Vec<u8> = blend_frame(1)
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut changed = base.clone();
    let at = (BLEND_WIDTH as usize + 1) * 3;
    changed[at..at + 3].copy_from_slice(&[0xFF, 0x00, 0x00]);

    let input = vec![base, changed];
    let bytes = encode(BLEND_WIDTH, BLEND_HEIGHT, ColorType::Rgb8, &input);
    let (_, decoded) = decode(&bytes);

    assert!(
        blend_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::BlendOp::Source))
    );
    assert_composites_to(&bytes, BLEND_WIDTH, ColorType::Rgb8, &input);
}

/// 色種別を落とす検証に使うキャンバスの大きさ
///
/// 1フレームでパレットに収まる色数を超えられるだけの画素を取る。
const REDUCE_WIDTH: u32 = 24;
const REDUCE_HEIGHT: u32 = 16;
/// 1フレームの画素数
const REDUCE_PIXELS: usize = REDUCE_WIDTH as usize * REDUCE_HEIGHT as usize;
/// 1フレームぶんの入力バイト数 (RGBA8)
const REDUCE_FRAME_LEN: usize = REDUCE_PIXELS * 4;

/// パレットに収まる色数の上限
const MAX_PALETTE_COLORS: usize = 256;

/// 画素ごとに違う色を置いたフレーム列を作る
///
/// 1フレームだけでパレットに収まる色数を超え、フレームごとに色をずらすため
/// すべてのフレームが全画面の差分になる。アルファは全画素255。
fn distinct_frames(color_type: ColorType, count: u32) -> Vec<Vec<u8>> {
    let bpp = color_type.bytes_per_pixel();
    (0..count)
        .map(|index| {
            let mut frame = Vec::with_capacity(REDUCE_PIXELS * bpp);
            for pixel in 0..REDUCE_PIXELS {
                let color = pixel + index as usize * REDUCE_PIXELS;
                frame.extend_from_slice(&[color as u8, (color >> 8) as u8, (color >> 16) as u8]);
                if bpp == 4 {
                    frame.push(0xFF);
                }
            }
            frame
        })
        .collect()
}

/// `colors` 種類の色を敷き詰めた不透明なRGBA8のフレームを作る
///
/// `offset` をずらすと、色の集合が重ならないフレームになる。
fn palette_frame(colors: usize, offset: usize) -> Vec<u8> {
    (0..REDUCE_PIXELS)
        .flat_map(|pixel| {
            let color = offset + pixel % colors;
            [color as u8, (color >> 8) as u8, (color >> 16) as u8, 0xFF]
        })
        .collect()
}

/// 色種別を落とす設定
fn reduce_config(color_type: ColorType) -> Config {
    Config {
        reduce_color: true,
        ..config(color_type)
    }
}

/// 符号化し、失敗したらそのエラーを返す
fn try_encode_with(
    width: u32,
    height: u32,
    config: Config,
    input: &[Vec<u8>],
) -> Result<Vec<u8>, Error> {
    let delay = FrameDelay::new(1001, 30000).unwrap();
    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        input.len() as u32,
        config,
    )?;
    for data in input {
        encoder.add_frame(data, delay)?;
    }
    Ok(encoder.finish()?.into_inner())
}

/// 符号化した結果を返す
fn encode_with(width: u32, height: u32, config: Config, input: &[Vec<u8>]) -> Vec<u8> {
    try_encode_with(width, height, config, input).expect("符号化に失敗した")
}

/// IHDRが示す出力の色種別
fn output_color_type(bytes: &[u8]) -> png::ColorType {
    png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .unwrap()
        .info()
        .color_type
}

/// 合成に使う画素表現
///
/// パレット参照の出力は [`Expansion`] が展開した後の表現になる。
fn composite_color_type(bytes: &[u8]) -> ColorType {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let info = reader.info();
    match info.color_type {
        png::ColorType::Rgb => ColorType::Rgb8,
        png::ColorType::Rgba => ColorType::Rgba8,
        png::ColorType::Indexed if info.trns.is_none() => ColorType::Rgb8,
        png::ColorType::Indexed => ColorType::Rgba8,
        other => panic!("扱わない色種別: {other:?}"),
    }
}

/// PLTEに並ぶ色
fn plte(bytes: &[u8]) -> Vec<u8> {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    reader
        .info()
        .palette
        .as_deref()
        .expect("PLTEが必要")
        .to_vec()
}

/// tRNSに並ぶアルファ
fn trns(bytes: &[u8]) -> Vec<u8> {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    reader.info().trns.as_deref().unwrap_or(&[]).to_vec()
}

/// RGBA8のフレームからアルファを落とす
fn without_alpha(frame: &[u8]) -> Vec<u8> {
    frame
        .chunks_exact(4)
        .flat_map(|p| p[..3].to_vec())
        .collect()
}

/// パレット参照で出た出力を合成し、RGBA8の入力と一致することを確かめる
///
/// アルファの落ちた出力は、入力からアルファを落としたものと比べる。
fn assert_reduced_roundtrip(bytes: &[u8], input: &[Vec<u8>]) {
    assert_eq!(output_color_type(bytes), png::ColorType::Indexed);

    let color_type = composite_color_type(bytes);
    let (_, decoded) = decode(bytes);
    assert_eq!(decoded.len(), input.len());

    let mut canvas = vec![0u8; REDUCE_FRAME_LEN / 4 * color_type.bytes_per_pixel()];
    for (index, (frame, source)) in decoded.iter().zip(input).enumerate() {
        let disposal = composite(&mut canvas, frame, REDUCE_WIDTH, color_type);

        let expected_frame = match color_type {
            ColorType::Rgb8 => without_alpha(source),
            ColorType::Rgba8 => source.clone(),
        };
        assert_eq!(canvas, expected_frame, "フレーム {index}");
        disposal.apply(&mut canvas);

        let expected_sequence = if index == 0 { 0 } else { index as u32 * 2 - 1 };
        assert_eq!(frame.control.sequence_number, expected_sequence);
    }
}

/// パレットで出るフレームも、矩形と遅延を保ったまま順に書き出される
#[test]
fn palette_frames_keep_their_rects_and_order() {
    let mut input = vec![vec![0xFFu8; REDUCE_FRAME_LEN]];
    let positions = [(1u32, 1u32), (6, 4), (3, 2)];
    for (index, &(x, y)) in positions.iter().enumerate() {
        let mut frame = input.last().unwrap().clone();
        let start = (y as usize * REDUCE_WIDTH as usize + x as usize) * 4;
        frame[start..start + 3].fill(0x10 + index as u8);
        input.push(frame);
    }

    let bytes = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8),
        &input,
    );

    // 4色しか無いためパレットで出る
    assert_reduced_roundtrip(&bytes, &input);

    let (_, decoded) = decode(&bytes);
    let expected: Vec<(u32, u32, u32, u32)> = std::iter::once((0, 0, REDUCE_WIDTH, REDUCE_HEIGHT))
        .chain(positions.iter().map(|&(x, y)| (x, y, 1, 1)))
        .collect();
    assert_eq!(rects(&decoded), expected);
}

/// 1フレームだけの入力も、そのフレームで載るか失敗するかが決まる
#[test]
fn a_single_frame_input_is_settled_on_that_frame() {
    let within = vec![palette_frame(MAX_PALETTE_COLORS, 0)];
    let bytes = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8),
        &within,
    );
    assert_reduced_roundtrip(&bytes, &within);

    let beyond = distinct_frames(ColorType::Rgba8, 1);
    assert!(matches!(
        try_encode_with(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            reduce_config(ColorType::Rgba8),
            &beyond
        ),
        Err(Error::ColorLimitExceeded { frame: 0 })
    ));
}

/// 透過する色と不透明な色を混ぜたRGBA8のフレームを作る
///
/// 色は8種で、そのうち3種のアルファが255未満になる。
fn mixed_alpha_frame() -> Vec<u8> {
    (0..REDUCE_PIXELS)
        .flat_map(|pixel| {
            let color = (pixel % 8) as u8;
            let alpha = if color < 3 { color * 0x40 } else { 0xFF };
            [color, 0x40, 0x80, alpha]
        })
        .collect()
}

/// 色種別を落とす設定で符号化する
fn encode_reduced(color_type: ColorType, input: &[Vec<u8>]) -> Vec<u8> {
    encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(color_type),
        input,
    )
}

/// 色数が上限に収まる入力は、1色でも上限ちょうどでもパレットで出る
#[test]
fn an_input_within_the_color_limit_is_written_as_indexed_color() {
    for colors in [1, 2, MAX_PALETTE_COLORS - 1, MAX_PALETTE_COLORS] {
        let input = vec![palette_frame(colors, 0), palette_frame(colors, 0)];
        let bytes = encode_reduced(ColorType::Rgba8, &input);

        assert_reduced_roundtrip(&bytes, &input);
    }
}

/// PLTEとtRNSは載せた色数に関わらず上限いっぱいのエントリ数で書く
///
/// 添字を振り終える前にフレームを書き出すため、色数が決まる前に場所を確保する。
/// 載せた色の後ろは、PLTEが0でtRNSが不透明の詰め物になる。
#[test]
fn the_palette_chunks_are_padded_to_a_fixed_number_of_entries() {
    const COLORS: usize = 8;

    let input = vec![mixed_alpha_frame()];
    let bytes = encode_reduced(ColorType::Rgba8, &input);

    let plte = plte(&bytes);
    assert_eq!(plte.len() / 3, MAX_PALETTE_COLORS);
    assert!(plte[COLORS * 3..].iter().all(|&byte| byte == 0));

    let trns = trns(&bytes);
    assert_eq!(trns.len(), MAX_PALETTE_COLORS);
    assert!(trns[COLORS..].iter().all(|&alpha| alpha == u8::MAX));

    assert_reduced_roundtrip(&bytes, &input);
}

/// 見つけた順が値の昇順とも降順とも食い違う色を敷いたRGBA8のフレーム
///
/// 先頭の画素から順に現れる色の並びが、どちらの向きの並べ替えとも一致しない。
fn unordered_palette_frame() -> Vec<u8> {
    const REDS: [u8; 4] = [0x30, 0x10, 0x40, 0x20];

    (0..REDUCE_PIXELS)
        .flat_map(|pixel| [REDS[pixel % REDS.len()], 0x00, 0x00, 0xFF])
        .collect()
}

/// 添字は色を見つけた順に振る
///
/// 全フレームを見終わる前に添字を焼くため、明るさなどでは並べ替えられない。
#[test]
fn the_palette_follows_the_order_the_colors_were_found() {
    let input = vec![unordered_palette_frame()];
    let bytes = encode_reduced(ColorType::Rgba8, &input);

    let plte = plte(&bytes);
    let reds: Vec<u8> = plte[..12].iter().step_by(3).copied().collect();
    assert_eq!(reds, [0x30, 0x10, 0x40, 0x20]);
    assert_reduced_roundtrip(&bytes, &input);
}

/// 上限ちょうどの色は載り、1つ超えると先頭フレームを指して失敗する
#[test]
fn one_color_over_the_limit_fails_on_the_first_frame() {
    let cases = [
        (
            ColorType::Rgb8,
            palette_frame_rgb as fn(usize, usize) -> Vec<u8>,
        ),
        (ColorType::Rgba8, palette_frame),
    ];

    for (color_type, frame) in cases {
        let within = vec![frame(MAX_PALETTE_COLORS, 0)];
        let bytes = encode_reduced(color_type, &within);
        assert_eq!(
            output_color_type(&bytes),
            png::ColorType::Indexed,
            "{color_type:?}"
        );
        assert_composites_to(&bytes, REDUCE_WIDTH, color_type, &within);

        let beyond = vec![frame(MAX_PALETTE_COLORS + 1, 0)];
        assert!(
            matches!(
                try_encode_with(
                    REDUCE_WIDTH,
                    REDUCE_HEIGHT,
                    reduce_config(color_type),
                    &beyond
                ),
                Err(Error::ColorLimitExceeded { frame: 0 })
            ),
            "{color_type:?}"
        );
    }
}

/// 色数が溢れたら、それを持ち込んだフレームを指して失敗する
///
/// 1フレームずつなら収まる色数でも、和集合がパレットを超えれば溢れる。
#[test]
fn a_color_beyond_the_palette_fails_on_the_frame_that_brings_it() {
    const PER_FRAME: usize = 200;

    let single = vec![palette_frame(PER_FRAME, 0)];
    let bytes = encode_reduced(ColorType::Rgba8, &single);
    assert_reduced_roundtrip(&bytes, &single);

    let union = vec![
        palette_frame(PER_FRAME, 0),
        palette_frame(PER_FRAME, 0),
        palette_frame(PER_FRAME, PER_FRAME),
    ];
    assert!(matches!(
        try_encode_with(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            reduce_config(ColorType::Rgba8),
            &union
        ),
        Err(Error::ColorLimitExceeded { frame: 2 })
    ));
}

/// 色数が溢れた後のエンコーダは使用不能になる
#[test]
fn a_color_beyond_the_palette_poisons_the_encoder() {
    let base = palette_blend_frame(1);
    let burst = with_color_burst(&base);
    let delay = FrameDelay::new(1001, 30000).unwrap();

    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        BLEND_WIDTH,
        BLEND_HEIGHT,
        3,
        reduce_config(ColorType::Rgba8),
    )
    .unwrap();
    encoder.add_frame(&base, delay).unwrap();
    assert!(matches!(
        encoder.add_frame(&burst, delay),
        Err(Error::ColorLimitExceeded { frame: 1 })
    ));
    assert!(matches!(
        encoder.add_frame(&base, delay),
        Err(Error::Poisoned)
    ));
    assert!(matches!(encoder.finish(), Err(Error::Poisoned)));
}

/// 型で見つけた最初のチャンクのデータ部
///
/// 終端していないバイト列も読めるよう、デコーダを通さずに走査する。
fn raw_chunk(bytes: &[u8], kind: &[u8; 4]) -> Vec<u8> {
    let mut offset = 8;
    while offset + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if &bytes[offset + 4..offset + 8] == kind {
            return bytes[offset + 8..offset + 8 + len].to_vec();
        }
        offset += 12 + len;
    }
    panic!("チャンクが必要: {}", String::from_utf8_lossy(kind));
}

/// 書き戻す前のPLTEに並ぶマゼンタ
fn magenta_entries() -> Vec<u8> {
    (0..MAX_PALETTE_COLORS)
        .flat_map(|_| [u8::MAX, 0x00, u8::MAX])
        .collect()
}

/// 色数が溢れた書き出しは、書き戻す前のマゼンタのパレットを残す
///
/// 場所を確保したPLTEは [`Encoder::finish`] まで書き戻されない。詰め物が黒だと
/// 寛容なデコーダが先頭フレームを真っ黒な絵として描いてしまう。
#[test]
fn a_color_beyond_the_palette_leaves_a_magenta_palette() {
    let base = palette_blend_frame(1);
    let burst = with_color_burst(&base);
    let delay = FrameDelay::new(1001, 30000).unwrap();
    let mut writer = Cursor::new(Vec::new());

    let mut encoder = Encoder::new(
        &mut writer,
        BLEND_WIDTH,
        BLEND_HEIGHT,
        3,
        reduce_config(ColorType::Rgba8),
    )
    .unwrap();
    encoder.add_frame(&base, delay).unwrap();
    assert!(matches!(
        encoder.add_frame(&burst, delay),
        Err(Error::ColorLimitExceeded { frame: 1 })
    ));
    drop(encoder);

    let bytes = writer.into_inner();
    assert_eq!(raw_chunk(&bytes, b"PLTE"), magenta_entries());
    assert!(
        raw_chunk(&bytes, b"tRNS")
            .iter()
            .all(|&alpha| alpha == u8::MAX)
    );
    assert!(!chunk_types(&bytes).iter().any(|kind| kind == b"IEND"));
}

/// 先頭からの絶対位置へのシークだけが失敗する書き出し先
///
/// その向きのシークはPLTEとtRNSの書き戻しでしか起きないため、全フレームを
/// 書き終えた後の書き戻しで失敗する。
struct RewriteFails(Cursor<Vec<u8>>);

impl Write for RewriteFails {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Seek for RewriteFails {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::Start(_) => Err(io::Error::other("書き戻し失敗")),
            pos => self.0.seek(pos),
        }
    }
}

/// パレットの書き戻しに失敗したら、終端せずにエラーを返す
///
/// 全フレームを書き終えた後の失敗なので、黙って成功すると詰め物のパレットを
/// 引く、IENDの無いファイルが残る。tRNSを持たないRGB8の入力ではPLTEが唯一の
/// 書き戻しになる。
#[test]
fn a_failure_while_settling_the_palette_is_reported() {
    let cases = [
        (
            ColorType::Rgba8,
            palette_frame as fn(usize, usize) -> Vec<u8>,
        ),
        (ColorType::Rgb8, palette_frame_rgb),
    ];

    for (color_type, frame) in cases {
        let input = vec![frame(8, 0), frame(8, 1)];
        let delay = FrameDelay::new(1001, 30000).unwrap();
        let mut writer = RewriteFails(Cursor::new(Vec::new()));

        let mut encoder = Encoder::new(
            &mut writer,
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            input.len() as u32,
            reduce_config(color_type),
        )
        .unwrap();
        for data in &input {
            encoder.add_frame(data, delay).unwrap();
        }
        assert!(
            matches!(encoder.finish(), Err(Error::Io(_))),
            "{color_type:?}"
        );

        let bytes = writer.0.into_inner();
        assert_eq!(
            raw_chunk(&bytes, b"PLTE"),
            magenta_entries(),
            "{color_type:?}"
        );
        assert!(
            !chunk_types(&bytes).iter().any(|kind| kind == b"IEND"),
            "{color_type:?}"
        );
    }
}

/// アルファを持つ色はtRNSに載り、画素は元のまま戻る
#[test]
fn a_palette_keeps_the_alpha_of_every_color() {
    let input = vec![mixed_alpha_frame()];
    let bytes = encode_reduced(ColorType::Rgba8, &input);

    assert_reduced_roundtrip(&bytes, &input);
    assert_eq!(trns(&bytes)[..3], [0x00, 0x40, 0x80]);
}

/// アルファを持つ入力のパレットは、透過が現れなくてもtRNSを持つ
///
/// tRNSを書くかどうかは入力の色種別で決まり、画素のアルファは見ない。
#[test]
fn a_palette_from_an_alpha_input_always_carries_a_trns() {
    let input = vec![palette_frame(4, 0)];
    let bytes = encode_reduced(ColorType::Rgba8, &input);

    assert_reduced_roundtrip(&bytes, &input);
    assert!(trns(&bytes).iter().all(|&alpha| alpha == u8::MAX));
}

/// PLTEとtRNSは画素データより前に置く
#[test]
fn the_palette_chunks_come_before_the_pixel_data() {
    let input = vec![mixed_alpha_frame()];
    let bytes = encode_reduced(ColorType::Rgba8, &input);

    let types = chunk_types(&bytes);
    let at = |kind: &[u8; 4]| {
        types
            .iter()
            .position(|t| t == kind)
            .expect("チャンクが必要")
    };

    assert_eq!(at(b"IHDR"), 0);
    assert!(at(b"PLTE") < at(b"tRNS"));
    assert!(at(b"tRNS") < at(b"fcTL"));
    assert!(at(b"acTL") < at(b"IDAT"));
    assert!(at(b"fcTL") < at(b"IDAT"));
}

/// RGB8の入力もパレットへ落とす。アルファが現れ得ないためtRNSは書かない
#[test]
fn rgb_input_within_the_color_limit_is_written_as_indexed_color() {
    let frame: Vec<u8> = palette_frame(16, 0)
        .chunks_exact(4)
        .flat_map(|pixel| pixel[..3].to_vec())
        .collect();
    let input = vec![frame.clone(), frame];
    let bytes = encode_reduced(ColorType::Rgb8, &input);

    assert_eq!(output_color_type(&bytes), png::ColorType::Indexed);
    assert!(chunk_types(&bytes).iter().all(|kind| kind != b"tRNS"));
    assert_composites_to(&bytes, REDUCE_WIDTH, ColorType::Rgb8, &input);
}

/// [`palette_frame`] と同じ色をRGB8で敷き詰めたフレームを作る
fn palette_frame_rgb(colors: usize, offset: usize) -> Vec<u8> {
    palette_frame(colors, offset)
        .chunks_exact(4)
        .flat_map(|pixel| pixel[..3].to_vec())
        .collect()
}

/// 全画面が交互に入れ替わる、色数が上限に収まるRGB8のフレーム列
///
/// どのフレームも直前と全画素が違い、1つ飛ばしのフレームは内容が戻る。
fn alternating_palette_frames(count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| palette_frame_rgb(4, index % 2 * 8))
        .collect()
}

/// 先頭フレームの後に現れた色も、パレットに収まる限り同じ表へ載る
#[test]
fn colors_found_after_the_first_frame_join_the_palette() {
    let input = alternating_palette_frames(5);
    let bytes = encode_reduced(ColorType::Rgb8, &input);

    assert_eq!(output_color_type(&bytes), png::ColorType::Indexed);
    assert_composites_to(&bytes, REDUCE_WIDTH, ColorType::Rgb8, &input);

    let (_, decoded) = decode(&bytes);
    assert!(dispose_ops(&decoded).contains(&png::DisposeOp::Previous));
}

/// パレットで出る素材は、部分矩形とdispose_opをまたいでも可逆であること
#[test]
fn a_palette_survives_partial_rects() {
    let input = transient_palette_frames();
    let bytes = encode_reduced(ColorType::Rgba8, &input);

    assert_reduced_roundtrip(&bytes, &input);
    assert_eq!(
        rects(&decode(&bytes).1)[1],
        (2, 1, 3, 2),
        "変わった領域だけを書く"
    );
}

/// 1フレームだけ現れる領域を持つ、色数が上限に収まるRGBA8のフレーム列
fn transient_palette_frames() -> Vec<Vec<u8>> {
    let base = mixed_alpha_frame();
    let mut marked = base.clone();
    for y in 1..3usize {
        for x in 2..5usize {
            let start = (y * REDUCE_WIDTH as usize + x) * 4;
            marked[start..start + 4].copy_from_slice(&[0xF0, 0xF1, 0xF2, 0x20]);
        }
    }

    vec![base.clone(), marked.clone(), base.clone(), marked, base]
}

/// フィルタ戦略の検証に使うキャンバスの大きさ
const FILTER_WIDTH: u32 = 40;
const FILTER_HEIGHT: u32 = 24;

/// 行の中で同じバイト列が繰り返すフレーム
///
/// フィルタを掛けずに渡した方が小さくなる。
fn flat_frame(color_type: ColorType, seed: u32) -> Vec<u8> {
    const PALETTE: [[u8; 3]; 4] = [
        [0x1E, 0x1E, 0x28],
        [0xD0, 0xD0, 0xC8],
        [0x40, 0x80, 0xC0],
        [0xC0, 0x40, 0x60],
    ];

    let blocks = frame_data((FILTER_WIDTH * FILTER_HEIGHT) as usize, seed + 1);
    let mut frame = Vec::new();
    for y in 0..FILTER_HEIGHT as usize {
        for x in 0..FILTER_WIDTH as usize {
            let block = x / 7 + y / 5 * 9;
            let index = (blocks[block % blocks.len()] as usize + seed as usize) % PALETTE.len();
            frame.extend_from_slice(&PALETTE[index]);
            if color_type == ColorType::Rgba8 {
                frame.push(0xFF);
            }
        }
    }
    frame
}

/// なだらかな階調に微小なノイズを載せたフレーム
///
/// 隣接画素の差が小さく、行ごとの適応フィルタが効く。
fn detailed_frame(color_type: ColorType, seed: u32) -> Vec<u8> {
    let bpp = color_type.bytes_per_pixel();
    let grain = frame_data((FILTER_WIDTH * FILTER_HEIGHT) as usize * 3, seed + 1);
    let mut frame = Vec::new();
    for y in 0..FILTER_HEIGHT as usize {
        for x in 0..FILTER_WIDTH as usize {
            for channel in 0..3 {
                let base = (x * 3 + y * 5 + channel * 17 + seed as usize * 2) as u8;
                let index = (y * FILTER_WIDTH as usize + x) * 3 + channel;
                frame.push(base.wrapping_add(grain[index] & 7));
            }
            if bpp == 4 {
                frame.push(0xFF);
            }
        }
    }
    frame
}

/// [`gradient_frame`] が敷く色数
///
/// キャンバスがこの全部を覆う値域になっている。
const GRADIENT_COLORS: usize = 128;

/// 添字がなだらかな勾配になるフレーム
///
/// 色を見つける順が値の昇順と一致するため、パレット参照の添字も勾配になる。
/// 隣接画素の添字の差が一定で、行ごとの適応フィルタが効く。
fn gradient_frame(color_type: ColorType, seed: u32) -> Vec<u8> {
    let mut frame = Vec::new();
    for y in 0..FILTER_HEIGHT as usize {
        for x in 0..FILTER_WIDTH as usize {
            let color = (x + y * 5 + seed as usize * 2) % GRADIENT_COLORS;
            frame.extend_from_slice(&[color as u8, 0x40, 0x80]);
            if color_type == ColorType::Rgba8 {
                frame.push(0xFF);
            }
        }
    }
    frame
}

/// 出力を合成し、フレームごとに `expected` と一致することを確かめる
fn assert_composites_to(bytes: &[u8], width: u32, color_type: ColorType, expected: &[Vec<u8>]) {
    let (_, decoded) = decode(bytes);
    assert_eq!(decoded.len(), expected.len());

    let mut canvas = vec![0u8; expected[0].len()];
    for (index, (frame, source)) in decoded.iter().zip(expected).enumerate() {
        let disposal = composite(&mut canvas, frame, width, color_type);
        assert_eq!(&canvas, source, "フレーム {index}");
        disposal.apply(&mut canvas);
    }
}

/// プローブの前後をまたぐ長さで、どちらの戦略に決まっても可逆であること
#[test]
fn both_filter_strategies_are_reversible() {
    let sources = [flat_frame as fn(ColorType, u32) -> Vec<u8>, detailed_frame];

    for color_type in [ColorType::Rgb8, ColorType::Rgba8] {
        for source in sources {
            for count in [1u32, 3, 5, 9] {
                let input: Vec<Vec<u8>> = (0..count).map(|seed| source(color_type, seed)).collect();
                let bytes = encode(FILTER_WIDTH, FILTER_HEIGHT, color_type, &input);

                assert_composites_to(&bytes, FILTER_WIDTH, color_type, &input);
            }
        }
    }
}

/// 色種別を落とす経路でも、どちらの戦略に決まっても可逆であること
#[test]
fn both_filter_strategies_are_reversible_while_reducing_color() {
    let sources = [flat_frame as fn(ColorType, u32) -> Vec<u8>, gradient_frame];
    let config = Config {
        reduce_color: true,
        ..config(ColorType::Rgba8)
    };

    for source in sources {
        for count in [1u32, 3, 5, 9] {
            for alpha in [0xFF, 0x80] {
                let mut input: Vec<Vec<u8>> = (0..count)
                    .map(|seed| source(ColorType::Rgba8, seed))
                    .collect();
                for frame in &mut input {
                    frame[3] = alpha;
                }

                let bytes = encode_with(FILTER_WIDTH, FILTER_HEIGHT, config, &input);
                let at = format!("{count} フレーム α={alpha:#04X}");
                assert_eq!(output_color_type(&bytes), png::ColorType::Indexed, "{at}");
                // アルファを持つ入力の出力は、パレットでもRGBA8へ展開される
                assert_eq!(composite_color_type(&bytes), ColorType::Rgba8, "{at}");
                assert_composites_to(&bytes, FILTER_WIDTH, ColorType::Rgba8, &input);
            }
        }
    }
}

/// 符号化して、パレットに載せた色数を取り出す
fn palette_colors_of(width: u32, height: u32, config: Config, input: &[Vec<u8>]) -> Option<u16> {
    let delay = FrameDelay::new(1001, 30000).unwrap();
    let mut encoder = Encoder::new(
        Cursor::new(Vec::new()),
        width,
        height,
        input.len() as u32,
        config,
    )
    .unwrap();
    for data in input {
        encoder.add_frame(data, delay).unwrap();
    }
    // 終端はエンコーダを消費するため、色数は先に読む
    let colors = encoder.palette_colors();
    encoder.finish().unwrap();
    colors
}

/// パレットで出したときは載せた色数まで分かる
#[test]
fn a_palette_is_reported_with_its_color_count() {
    for colors in [1, 200, MAX_PALETTE_COLORS] {
        let input = vec![palette_frame(colors, 0), palette_frame(colors, 0)];
        assert_eq!(
            palette_colors_of(
                REDUCE_WIDTH,
                REDUCE_HEIGHT,
                reduce_config(ColorType::Rgba8),
                &input
            ),
            Some(colors as u16),
            "{colors} 色"
        );
    }
}

/// 後のフレームが持ち込んだ色も、報告される色数に入る
#[test]
fn colors_brought_by_a_later_frame_join_the_reported_count() {
    let input = vec![palette_frame(100, 0), palette_frame(100, 100)];
    assert_eq!(
        palette_colors_of(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            reduce_config(ColorType::Rgba8),
            &input
        ),
        Some(200)
    );
}

/// 落とす設定でなければ色数も無い
#[test]
fn nothing_is_reported_without_the_setting() {
    let input = distinct_frames(ColorType::Rgba8, 4);
    assert_eq!(
        palette_colors_of(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            config(ColorType::Rgba8),
            &input
        ),
        None
    );
}
