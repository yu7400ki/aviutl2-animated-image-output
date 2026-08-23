//! APNG (Animated PNG) エンコーダ

mod alpha;
mod chunk;
mod delay;
mod diff;
mod encoder;
mod error;
mod filter;
mod palette;
mod region;
mod spool;
mod zlib;

pub use delay::FrameDelay;
pub use encoder::{COMPRESSION_LEVELS, ColorType, Config, DEFAULT_MAX_SPOOL_BYTES, Encoder};
pub use error::Error;

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
