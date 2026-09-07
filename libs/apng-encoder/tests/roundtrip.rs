//! 出力したAPNGを`png`クレートでデコードし、入力フレームと一致することを確認する

use anim_core::InputError;
use apng_encoder::{ColorType, Config, Encoder, Error, FrameDelay};
use std::io::{self, Cursor, Write};
use std::num::NonZeroUsize;

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
        encoder.add_frame(data.clone(), delay).unwrap();
    }
    encoder.finish().unwrap().into_inner()
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
        .add_frame(input[0].clone(), FrameDelay::new(1, 30).unwrap())
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
            .add_frame(input[0].clone(), FrameDelay::new(1001, 120000).unwrap())
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
            .add_frame(data.clone(), FrameDelay::new(1, 30).unwrap())
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
/// 先頭フレームは決定と書き出しの列を抜けるまで書き出されないので、失敗が伝わるのは
/// 先に投入したフレームのものになる。
#[test]
fn a_failed_write_poisons_the_encoder() {
    /// 書き出しの失敗と、その次の投入まで届くフレーム数
    ///
    /// 列を抜けるのに要る数より余裕を持たせている。ワーカー数は列の深さを決めるので
    /// 1つに固定する。
    const COUNT: u32 = 8;

    let input = frames(8, 8, ColorType::Rgba8, COUNT);
    let delay = FrameDelay::new(1, 30).unwrap();
    // シグネチャ・IHDR・acTL・fcTLは通り、IDATの途中で失敗する長さ
    let writer = FailingWriter { remaining: 100 };
    let mut encoder = Encoder::with_workers(
        writer,
        8,
        8,
        COUNT,
        config(ColorType::Rgba8),
        NonZeroUsize::MIN,
    )
    .unwrap();

    let outcomes: Vec<Result<(), Error>> = input
        .iter()
        .map(|frame| encoder.add_frame(frame.clone(), delay))
        .collect();
    let failed = outcomes
        .iter()
        .position(|outcome| outcome.is_err())
        .expect("列を抜けたフレームの書き出しが失敗する");

    assert!(matches!(outcomes[failed], Err(Error::Io(_))));
    assert!(
        matches!(outcomes[failed + 1], Err(Error::Poisoned)),
        "失敗した後も投入を受け付けている"
    );
    assert!(matches!(encoder.finish(), Err(Error::Poisoned)));
}

#[test]
fn frame_of_the_wrong_size_is_rejected() {
    let mut encoder =
        Encoder::new(Cursor::new(Vec::new()), 4, 4, 1, config(ColorType::Rgba8)).unwrap();
    assert!(matches!(
        encoder.add_frame(vec![0u8; 63], FrameDelay::new(1, 30).unwrap()),
        Err(Error::Input(InputError::FrameSizeMismatch {
            expected: 64,
            actual: 63
        }))
    ));
}

#[test]
fn extra_frame_is_rejected() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let delay = FrameDelay::new(1, 30).unwrap();
    let mut encoder =
        Encoder::new(Cursor::new(Vec::new()), 2, 2, 1, config(ColorType::Rgba8)).unwrap();
    encoder.add_frame(input[0].clone(), delay).unwrap();

    assert!(matches!(
        encoder.add_frame(input[0].clone(), delay),
        Err(Error::Input(InputError::FrameCountMismatch {
            expected: 1,
            actual: 2
        }))
    ));
}

#[test]
fn missing_frame_is_rejected_on_finish() {
    let input = frames(2, 2, ColorType::Rgba8, 1);
    let mut encoder =
        Encoder::new(Cursor::new(Vec::new()), 2, 2, 3, config(ColorType::Rgba8)).unwrap();
    encoder
        .add_frame(input[0].clone(), FrameDelay::new(1, 30).unwrap())
        .unwrap();

    assert!(matches!(
        encoder.finish(),
        Err(Error::Input(InputError::FrameCountMismatch {
            expected: 3,
            actual: 1
        }))
    ));
}

#[test]
fn invalid_parameters_are_rejected() {
    let rgba = config(ColorType::Rgba8);
    assert!(matches!(
        Encoder::new(Cursor::new(Vec::new()), 0, 4, 1, rgba),
        Err(Error::Input(InputError::InvalidDimensions {
            width: 0,
            height: 4
        }))
    ));
    assert!(matches!(
        Encoder::new(Cursor::new(Vec::new()), 4, 4, 0, rgba),
        Err(Error::Input(InputError::InvalidFrameCount))
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

/// どちらの戦略が選ばれても可逆であること
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
