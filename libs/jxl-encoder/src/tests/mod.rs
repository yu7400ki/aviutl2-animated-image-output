//! crate の内側からしか見られないものを使う検証

mod basis;
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
