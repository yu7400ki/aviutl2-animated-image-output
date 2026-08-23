//! 出力したAPNGを`png`クレートでデコードし、入力フレームと一致することを確認する

use apng_encoder::{ColorType, Config, DEFAULT_MAX_SPOOL_BYTES, Encoder, Error, FrameDelay};
use std::io::{self, Cursor, Write};

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
        Vec::new(),
        width,
        height,
        input.len() as u32,
        config(color_type),
    )
    .unwrap();
    for data in input {
        encoder.add_frame(data, delay).unwrap();
    }
    encoder.finish().unwrap()
}

struct DecodedFrame {
    data: Vec<u8>,
    control: png::FrameControl,
}

/// APNGを読み出し、acTLの再生回数と全フレームを返す
fn decode(bytes: &[u8]) -> (u32, Vec<DecodedFrame>) {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let animation = *reader.info().animation_control().expect("acTLが必要");

    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let mut decoded = Vec::new();
    for _ in 0..animation.num_frames {
        let info = reader.next_frame(&mut buf).unwrap();
        decoded.push(DecodedFrame {
            data: buf[..info.buffer_size()].to_vec(),
            control: *reader.info().frame_control().expect("fcTLが必要"),
        });
    }

    (animation.num_plays, decoded)
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

/// フレームの矩形をキャンバスの該当位置へ書き込み、dispose_opの後始末を返す
fn composite(
    canvas: &mut [u8],
    frame: &DecodedFrame,
    width: u32,
    color_type: ColorType,
) -> Disposal {
    assert!(matches!(frame.control.blend_op, png::BlendOp::Source));

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
        canvas[dst..dst + row_len].copy_from_slice(&frame.data[src..src + row_len]);
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
        Vec::new(),
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
    let bytes = encoder.finish().unwrap();

    assert_eq!(decode(&bytes).0, 7);
}

/// u16に収まらないフレームレートも近似されてfcTLに載る
#[test]
fn out_of_range_delay_is_approximated() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let bytes = {
        let mut encoder = Encoder::new(Vec::new(), 2, 2, 1, config(ColorType::Rgba8)).unwrap();
        encoder
            .add_frame(&input[0], FrameDelay::new(1001, 120000).unwrap())
            .unwrap();
        encoder.finish().unwrap()
    };

    let (_, decoded) = decode(&bytes);
    let control = decoded[0].control;
    assert_eq!((control.delay_num, control.delay_den), (342, 40999));
}

/// 1フレームのバイト数が`usize`で表現できない大きさは`new`の時点で弾く
#[test]
fn oversized_image_is_rejected() {
    assert!(matches!(
        Encoder::new(Vec::new(), u32::MAX, u32::MAX, 1, config(ColorType::Rgba8)),
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
            Vec::new(),
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
        encoder.finish().unwrap().len()
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
}

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.remaining {
            self.remaining = 0;
            return Err(io::Error::other("書き出し失敗"));
        }
        self.remaining -= buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
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
    let writer = FailingWriter { remaining: 100 };
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
    let mut encoder = Encoder::new(Vec::new(), 4, 4, 1, config(ColorType::Rgba8)).unwrap();
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
    let mut encoder = Encoder::new(Vec::new(), 2, 2, 1, config(ColorType::Rgba8)).unwrap();
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
    let mut encoder = Encoder::new(Vec::new(), 2, 2, 3, config(ColorType::Rgba8)).unwrap();
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
        Encoder::new(Vec::new(), 0, 4, 1, rgba),
        Err(Error::InvalidDimensions {
            width: 0,
            height: 4
        })
    ));
    assert!(matches!(
        Encoder::new(Vec::new(), 4, 4, 0, rgba),
        Err(Error::InvalidFrameCount)
    ));
    assert!(matches!(
        Encoder::new(
            Vec::new(),
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

/// 色種別を落とす検証に使うキャンバスの大きさ
const REDUCE_WIDTH: u32 = 8;
const REDUCE_HEIGHT: u32 = 6;
/// 1フレームぶんの入力バイト数 (RGBA8)
const REDUCE_FRAME_LEN: usize = REDUCE_WIDTH as usize * REDUCE_HEIGHT as usize * 4;

/// 全画素が不透明なRGBA8のフレーム列を作る
///
/// `alpha_from` 以降のフレームは、先頭画素だけ不透明でなくなる。
fn reducible_frames(count: u32, alpha_from: Option<u32>) -> Vec<Vec<u8>> {
    (0..count)
        .map(|index| {
            let mut frame = frame_data(REDUCE_FRAME_LEN, index + 1);
            for pixel in frame.chunks_exact_mut(4) {
                pixel[3] = 0xFF;
            }
            if alpha_from.is_some_and(|from| index >= from) {
                frame[3] = 0x80;
            }
            frame
        })
        .collect()
}

/// 色種別を落とす設定
fn reduce_config(color_type: ColorType, max_spool_bytes: usize) -> Config {
    Config {
        reduce_color: true,
        max_spool_bytes,
        ..config(color_type)
    }
}

/// 符号化した結果と、溜めたバイト数の最大値を返す
fn encode_with(width: u32, height: u32, config: Config, input: &[Vec<u8>]) -> (Vec<u8>, usize) {
    let delay = FrameDelay::new(1001, 30000).unwrap();
    let mut encoder = Encoder::new(Vec::new(), width, height, input.len() as u32, config).unwrap();
    for data in input {
        encoder.add_frame(data, delay).unwrap();
    }
    let peak = encoder.peak_spool_bytes();
    (encoder.finish().unwrap(), peak)
}

/// IHDRが示す出力の色種別
fn output_color_type(bytes: &[u8]) -> png::ColorType {
    png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .unwrap()
        .info()
        .color_type
}

/// RGBA8のフレームからアルファを落とす
fn without_alpha(frame: &[u8]) -> Vec<u8> {
    frame
        .chunks_exact(4)
        .flat_map(|p| p[..3].to_vec())
        .collect()
}

/// 出力を合成し、RGBA8の入力と一致することを確かめる
///
/// 出力がRGBのときは、入力からアルファを落としたものと比べる。
fn assert_reduced_roundtrip(bytes: &[u8], input: &[Vec<u8>], expected: png::ColorType) {
    assert_eq!(output_color_type(bytes), expected);

    let color_type = match expected {
        png::ColorType::Rgb => ColorType::Rgb8,
        png::ColorType::Rgba => ColorType::Rgba8,
        other => panic!("扱わない色種別: {other:?}"),
    };

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

/// アルファが最初に現れる位置で出力の色種別が決まり、どの位置でも可逆であること
#[test]
fn the_output_color_type_follows_where_alpha_first_appears() {
    const COUNT: u32 = 5;

    for alpha_from in [Some(0), Some(2), Some(COUNT - 1), None] {
        let input = reducible_frames(COUNT, alpha_from);
        let (bytes, _) = encode_with(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
            &input,
        );

        let expected = match alpha_from {
            Some(_) => png::ColorType::Rgba,
            None => png::ColorType::Rgb,
        };
        assert_reduced_roundtrip(&bytes, &input, expected);
    }
}

/// アルファを見つけた時点で確定するので、そこまでの出力は保留しない場合と一致する
#[test]
fn deciding_early_produces_the_same_bytes_as_not_reducing() {
    let input = reducible_frames(4, Some(0));
    let plain = encode(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgba8, &input);
    let (reduced, _) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    assert_eq!(reduced, plain);
}

/// 落とす余地の無いRGB8の入力は、保留せずそのまま書き出す
#[test]
fn rgb_input_is_never_spooled() {
    let input = frames(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgb8, 4);
    let plain = encode(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgb8, &input);
    let (reduced, peak) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgb8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    assert_eq!(reduced, plain);
    assert_eq!(peak, 0);
}

/// 全画素が不透明なら最後のフレームまで溜めてからRGBへ落とす
#[test]
fn a_fully_opaque_input_is_written_as_rgb() {
    const COUNT: u32 = 4;
    let input = reducible_frames(COUNT, None);
    let (bytes, peak) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    assert_reduced_roundtrip(&bytes, &input, png::ColorType::Rgb);
    // 全フレームが全画面の差分になるため、画素だけで入力そのものと同じ量を抱える
    let pixels = REDUCE_FRAME_LEN * COUNT as usize;
    assert!(
        (pixels..pixels * 2).contains(&peak),
        "{pixels} バイトの画素に対して抱えたのは {peak} バイト"
    );
    assert!(bytes.len() < encode(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgba8, &input).len());
}

/// 上限を超えるフレームが来たら落とすのをやめ、入力の色種別のまま書き出す
#[test]
fn exceeding_the_spool_limit_falls_back_to_the_input_color_type() {
    let input = reducible_frames(4, None);
    let plain = encode(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgba8, &input);
    // 2フレーム目を溜められない上限
    let limit = REDUCE_FRAME_LEN + REDUCE_FRAME_LEN / 2;
    let (bytes, peak) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, limit),
        &input,
    );

    assert_eq!(bytes, plain);
    assert_reduced_roundtrip(&bytes, &input, png::ColorType::Rgba);
    assert!(peak <= limit);
}

/// 先頭フレームすら溜められない上限でも、保留せずに書き出せる
#[test]
fn a_zero_spool_limit_writes_everything_as_it_arrives() {
    let input = reducible_frames(3, None);
    let plain = encode(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgba8, &input);
    let (bytes, peak) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, 0),
        &input,
    );

    assert_eq!(bytes, plain);
    assert_eq!(peak, 0);
}

/// 溜めたフレームは矩形と遅延を保ったまま順に書き出される
#[test]
fn spooled_frames_keep_their_rects_and_order() {
    let mut input = vec![vec![0xFFu8; REDUCE_FRAME_LEN]];
    let positions = [(1u32, 1u32), (6, 4), (3, 2)];
    for (index, &(x, y)) in positions.iter().enumerate() {
        let mut frame = input.last().unwrap().clone();
        let start = (y as usize * REDUCE_WIDTH as usize + x as usize) * 4;
        frame[start..start + 3].fill(0x10 + index as u8);
        input.push(frame);
    }

    let (bytes, _) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    assert_reduced_roundtrip(&bytes, &input, png::ColorType::Rgb);

    let (_, decoded) = decode(&bytes);
    let expected: Vec<(u32, u32, u32, u32)> = std::iter::once((0, 0, REDUCE_WIDTH, REDUCE_HEIGHT))
        .chain(positions.iter().map(|&(x, y)| (x, y, 1, 1)))
        .collect();
    assert_eq!(rects(&decoded), expected);
}

/// 1フレームだけの入力も、そのフレームを見てから色種別が決まる
#[test]
fn a_single_frame_input_is_decided_on_that_frame() {
    for alpha_from in [Some(0), None] {
        let input = reducible_frames(1, alpha_from);
        let (bytes, _) = encode_with(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
            &input,
        );

        let expected = match alpha_from {
            Some(_) => png::ColorType::Rgba,
            None => png::ColorType::Rgb,
        };
        assert_reduced_roundtrip(&bytes, &input, expected);
    }
}

/// 差分矩形の内側にあるアルファは、矩形の原点から離れていても見つかる
///
/// 走査が矩形の先頭だけに縮むと、アルファが無いものとして扱われてRGBへ落ちてしまう。
#[test]
fn alpha_anywhere_inside_a_partial_rect_is_found() {
    const BLOCK: u32 = 3;
    const ORIGIN: (u32, u32) = (4, 2);

    for hole in [(0u32, 0u32), (1, 1), (BLOCK - 1, BLOCK - 1)] {
        let base = vec![0xFFu8; REDUCE_FRAME_LEN];
        let mut changed = base.clone();
        for y in 0..BLOCK {
            for x in 0..BLOCK {
                let start =
                    ((ORIGIN.1 + y) as usize * REDUCE_WIDTH as usize + (ORIGIN.0 + x) as usize) * 4;
                changed[start..start + 3].fill(0x10 + (y * BLOCK + x) as u8);
                if (x, y) == hole {
                    changed[start + 3] = 0x80;
                }
            }
        }

        let input = vec![base, changed];
        let (bytes, _) = encode_with(
            REDUCE_WIDTH,
            REDUCE_HEIGHT,
            reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
            &input,
        );

        let (_, decoded) = decode(&bytes);
        assert_eq!(
            rects(&decoded),
            [
                (0, 0, REDUCE_WIDTH, REDUCE_HEIGHT),
                (ORIGIN.0, ORIGIN.1, BLOCK, BLOCK)
            ],
            "{hole:?}"
        );
        assert_reduced_roundtrip(&bytes, &input, png::ColorType::Rgba);
    }
}

/// 1フレームだけ現れる領域を持つRGBA8のフレーム列
///
/// `transparent` が真なら、先頭フレームから不透明でない画素を含む。
fn transient_frames(transparent: bool) -> Vec<Vec<u8>> {
    let mut base = vec![0xFFu8; REDUCE_FRAME_LEN];
    if transparent {
        base[3] = 0x80;
    }

    let mut marked = base.clone();
    for y in 1..3usize {
        for x in 2..5usize {
            let start = (y * REDUCE_WIDTH as usize + x) * 4;
            marked[start..start + 3].fill(0x10);
        }
    }

    vec![base.clone(), marked.clone(), base.clone(), marked, base]
}

/// 溜めるのをやめた後のdispose_opは、最初から溜めない場合と変わらない
#[test]
fn disposal_after_the_spool_matches_not_reducing() {
    let input = transient_frames(true);
    let plain = encode(REDUCE_WIDTH, REDUCE_HEIGHT, ColorType::Rgba8, &input);
    let (reduced, _) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    assert_eq!(reduced, plain);

    let (_, decoded) = decode(&reduced);
    assert!(dispose_ops(&decoded).contains(&png::DisposeOp::Previous));
    assert_reduced_roundtrip(&reduced, &input, png::ColorType::Rgba);
}

/// 溜めるのをやめた直後のフレームには、捨てられる保留フレームが無い
///
/// そこで捨てる判断をすると、書き出し済みのフレームは戻らないのに、次のフレームは
/// 戻ったキャンバスとの差分で書かれてしまう。
#[test]
fn the_frame_after_a_commit_has_nothing_to_dispose() {
    const BLOCK: (u32, u32, u32, u32) = (2, 1, 3, 2);

    let base = vec![0xFFu8; REDUCE_FRAME_LEN];
    let mut marked = base.clone();
    for y in 0..BLOCK.3 as usize {
        for x in 0..BLOCK.2 as usize {
            let start = ((BLOCK.1 as usize + y) * REDUCE_WIDTH as usize + BLOCK.0 as usize + x) * 4;
            marked[start..start + 3].fill(0x10);
        }
    }
    // 2フレーム目で透過が見つかり、そこで色種別が確定して溜めたぶんが流れる
    let alpha = (BLOCK.1 as usize * REDUCE_WIDTH as usize + BLOCK.0 as usize) * 4 + 3;
    marked[alpha] = 0x80;

    let input = vec![base.clone(), marked, base];
    let (bytes, _) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    let (_, decoded) = decode(&bytes);
    assert!(
        dispose_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::DisposeOp::None))
    );
    assert_eq!(
        rects(&decoded),
        [(0, 0, REDUCE_WIDTH, REDUCE_HEIGHT), BLOCK, BLOCK]
    );
    assert_reduced_roundtrip(&bytes, &input, png::ColorType::Rgba);
}

/// 溜めている間は出力の色種別が決まらず候補を比べられないため、捨てない
#[test]
fn spooled_frames_are_never_disposed() {
    let input = transient_frames(false);
    let (bytes, _) = encode_with(
        REDUCE_WIDTH,
        REDUCE_HEIGHT,
        reduce_config(ColorType::Rgba8, DEFAULT_MAX_SPOOL_BYTES),
        &input,
    );

    let (_, decoded) = decode(&bytes);
    assert!(
        dispose_ops(&decoded)
            .iter()
            .all(|op| matches!(op, png::DisposeOp::None))
    );
    assert_reduced_roundtrip(&bytes, &input, png::ColorType::Rgb);
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
    let sources = [flat_frame as fn(ColorType, u32) -> Vec<u8>, detailed_frame];

    for source in sources {
        for count in [1u32, 3, 5, 9] {
            let opaque: Vec<Vec<u8>> = (0..count)
                .map(|seed| source(ColorType::Rgba8, seed))
                .collect();
            let config = Config {
                reduce_color: true,
                ..config(ColorType::Rgba8)
            };

            let (bytes, _) = encode_with(FILTER_WIDTH, FILTER_HEIGHT, config, &opaque);
            assert_eq!(output_color_type(&bytes), png::ColorType::Rgb);
            let expected: Vec<Vec<u8>> = opaque.iter().map(|f| without_alpha(f)).collect();
            assert_composites_to(&bytes, FILTER_WIDTH, ColorType::Rgb8, &expected);

            // 透過を含む入力は、溜めずにそのまま書き出す経路へ移る
            let mut transparent = opaque.clone();
            for frame in &mut transparent {
                frame[3] = 0x80;
            }
            let (bytes, _) = encode_with(FILTER_WIDTH, FILTER_HEIGHT, config, &transparent);
            assert_eq!(output_color_type(&bytes), png::ColorType::Rgba);
            assert_composites_to(&bytes, FILTER_WIDTH, ColorType::Rgba8, &transparent);
        }
    }
}
