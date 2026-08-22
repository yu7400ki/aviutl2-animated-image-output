//! 出力したAPNGを`png`クレートでデコードし、入力フレームと一致することを確認する

use apng_encoder::{ColorType, Config, Encoder, Error, FrameDelay};
use std::io::Cursor;

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

/// フレームをキャンバスへ合成する
fn composite(canvas: &mut [u8], frame: &DecodedFrame, width: u32, height: u32) {
    assert_eq!(frame.control.x_offset, 0);
    assert_eq!(frame.control.y_offset, 0);
    assert_eq!(frame.control.width, width);
    assert_eq!(frame.control.height, height);
    assert!(matches!(frame.control.dispose_op, png::DisposeOp::None));
    assert!(matches!(frame.control.blend_op, png::BlendOp::Source));

    canvas.copy_from_slice(&frame.data);
}

fn assert_roundtrip(width: u32, height: u32, color_type: ColorType, count: u32) {
    let input = frames(width, height, color_type, count);
    let bytes = encode(width, height, color_type, &input);
    let (num_plays, decoded) = decode(&bytes);

    assert_eq!(num_plays, 0);
    assert_eq!(decoded.len(), input.len());

    let mut canvas = vec![0u8; input[0].len()];
    for (index, (frame, expected)) in decoded.iter().zip(&input).enumerate() {
        composite(&mut canvas, frame, width, height);
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
