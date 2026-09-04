//! crate の内側からしか見られないものを使う検証

mod closed;
mod layers;

#[path = "../../tests/support/mod.rs"]
mod support;

use crate::layers::Layers;
use crate::{ColorType, Config, Encoder};
use anim_core::Rect;
use support::*;

/// 矩形を書いた順に控えながら `parts` を流し込み、返った対を集める
fn peel<'a>(
    color_type: ColorType,
    parts: impl IntoIterator<Item = &'a [u8]>,
    written: &[(u32, u32, u32, u32)],
) -> Vec<(Rect, Vec<u8>)> {
    let mut layers = Layers::new(color_type, 2).unwrap();
    for rect in written {
        layers.wrote(rect_of(*rect));
    }

    let mut peeled = Vec::new();
    for part in parts {
        layers
            .feed(part, |rect, layer| peeled.push((rect, layer.to_vec())))
            .unwrap();
    }
    peeled
}

/// フレームを1枚ずつ投入し、表示フレームごとの画面と書き出したバイト列を集める
fn screens(config: Config, frames: &[Vec<u8>], durations: &[u32]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut encoder = Encoder::new(Vec::new(), WIDTH, HEIGHT, frames.len() as u32, config).unwrap();
    let mut screens = Vec::new();
    for (frame, duration) in frames.iter().zip(durations) {
        encoder.add_frame(frame, *duration).unwrap();
        screens.push(encoder.delta().screen().expect("復号器が無い").to_vec());
    }
    encoder.close().unwrap();
    screens.push(encoder.delta().screen().expect("復号器が無い").to_vec());

    // 先頭フレームはまだ書き出されておらず、画面に出ていない
    screens.remove(0);
    (screens, encoder.finish().unwrap())
}
