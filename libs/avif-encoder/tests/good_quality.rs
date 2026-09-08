//! 良質側の動作点を1スレッドで通す
//!
//! この組み合わせでだけ働く符号化器の内部の見積りは、1フレーム分の統計が
//! 溜まってから初めて使われる。キャンバスを縮めると標本が足りず、経路ごと踏まなくなる。

use avif_encoder::{ColorType, Config, Encoder, Usage, YuvFormat};

const WIDTH: u32 = 416;
const HEIGHT: u32 = 240;
const FRAMES: u32 = 24;

/// 明るい地に暗い帯を並べ、横へ流す。境界の立った画面の絵を模す
fn scrolling_text(phase: u32) -> Vec<u8> {
    let mut rgb = vec![0xf2u8; (WIDTH * HEIGHT * 3) as usize];
    for y in 0..HEIGHT {
        let row = y / 16;
        if y % 16 >= 11 {
            continue;
        }
        // 行ごとに違う周期と長さの帯を置く
        let period = 7 + (row * 5) % 23;
        let width = 3 + (row * 3) % 9;
        for x in 0..WIDTH {
            let u = (x + phase * (1 + row % 3)) % period;
            if u < width {
                let index = ((y * WIDTH + x) * 3) as usize;
                let shade = (0x20 + (row * 17) % 0x40) as u8;
                rgb[index] = shade;
                rgb[index + 1] = shade;
                rgb[index + 2] = shade.saturating_add(0x18);
            }
        }
    }
    rgb
}

/// `scrolling_text` に、横へ流れる帯状の α を足したもの
fn scrolling_text_rgba(phase: u32) -> Vec<u8> {
    scrolling_text(phase)
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .flat_map(|(index, pixel)| {
            let x = (index as u32) % WIDTH;
            let alpha = (((x + phase * 5) % 64) * 4) as u8;
            [pixel[0], pixel[1], pixel[2], alpha]
        })
        .collect()
}

/// speed 6 / 1スレッドで `frame` の返す列を流し切る
fn encode(color_type: ColorType, frame: impl Fn(u32) -> Vec<u8>) {
    let config = Config {
        color_type,
        quality: 75,
        speed: 6,
        yuv_format: YuvFormat::Yuv420,
        num_plays: 0,
        timescale: 30,
        max_threads: 1,
    };
    assert_eq!(config.operating_point(false).usage, Usage::GoodQuality);

    let mut encoder = Encoder::new(Vec::new(), WIDTH, HEIGHT, FRAMES, config).unwrap();
    for phase in 0..FRAMES {
        encoder.add_frame(&frame(phase), 1).unwrap();
    }
    assert!(!encoder.finish().unwrap().is_empty());
}

#[test]
fn good_quality_single_thread_survives_many_rgb_frames() {
    encode(ColorType::Rgb8, scrolling_text);
}

#[test]
fn good_quality_single_thread_survives_many_rgba_frames() {
    encode(ColorType::Rgba8, scrolling_text_rgba);
}
