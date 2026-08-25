//! GIFエンコーダ

mod block;
mod encoder;
mod error;
mod frame;
mod layout;
mod lzw;
mod normalize;
mod spool;
mod table;

pub use anim_core::FrameDelay;
pub use encoder::{Config, Encoder, PaletteKind, Report};
pub use error::Error;
pub use layout::ColorType;

/// テストで共有する素材の生成
#[cfg(test)]
mod testing {
    /// 決定的な擬似乱数列
    pub(crate) fn noise(len: usize, seed: u32) -> Vec<u8> {
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
}
