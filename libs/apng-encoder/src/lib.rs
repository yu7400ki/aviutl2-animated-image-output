//! APNG (Animated PNG) エンコーダ

mod chunk;
mod delay;
mod diff;
mod encoder;
mod error;
mod filter;
mod zlib;

pub use delay::FrameDelay;
pub use encoder::{COMPRESSION_LEVELS, ColorType, Config, Encoder};
pub use error::Error;
