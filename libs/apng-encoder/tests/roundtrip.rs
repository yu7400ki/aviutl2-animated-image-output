//! 出力したAPNGを`png`クレートでデコードし、入力フレームと一致することを確認する

use apng_encoder::{ColorType, Config, Encoder, Error, FrameDelay};
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
        reduce_color: false,
        max_spool_bytes: apng_encoder::DEFAULT_MAX_SPOOL_BYTES,
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

/// フレームの矩形をキャンバスの該当位置へ書き込む
fn composite(canvas: &mut [u8], frame: &DecodedFrame, width: u32, color_type: ColorType) {
    assert!(matches!(frame.control.dispose_op, png::DisposeOp::None));
    assert!(matches!(frame.control.blend_op, png::BlendOp::Source));

    let bpp = color_type.bytes_per_pixel();
    let stride = width as usize * bpp;
    let row_len = frame.control.width as usize * bpp;
    let head = frame.control.y_offset as usize * stride + frame.control.x_offset as usize * bpp;
    for y in 0..frame.control.height as usize {
        let dst = head + y * stride;
        let src = y * row_len;
        canvas[dst..dst + row_len].copy_from_slice(&frame.data[src..src + row_len]);
    }
}

fn assert_roundtrip(width: u32, height: u32, color_type: ColorType, count: u32) {
    let input = frames(width, height, color_type, count);
    let bytes = encode(width, height, color_type, &input);
    let (num_plays, decoded) = decode(&bytes);

    assert_eq!(num_plays, 0);
    assert_eq!(decoded.len(), input.len());

    let mut canvas = vec![0u8; input[0].len()];
    for (index, (frame, expected)) in decoded.iter().zip(&input).enumerate() {
        composite(&mut canvas, frame, width, color_type);
        assert_eq!(&canvas, expected, "フレーム {index}");

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
#[test]
fn a_failed_write_poisons_the_encoder() {
    let input = frames(8, 8, ColorType::Rgba8, 2);
    let delay = FrameDelay::new(1, 30).unwrap();
    // シグネチャ・IHDR・acTL・fcTLは通り、IDATの途中で失敗する長さ
    let writer = FailingWriter { remaining: 100 };
    let mut encoder = Encoder::new(writer, 8, 8, 2, config(ColorType::Rgba8)).unwrap();

    assert!(matches!(
        encoder.add_frame(&input[0], delay),
        Err(Error::Io(_))
    ));
    assert!(matches!(
        encoder.add_frame(&input[1], delay),
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
        composite(&mut canvas, frame, CROP_WIDTH, color_type);
        assert_eq!(&canvas, source, "{color_type:?} フレーム {index}");
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
