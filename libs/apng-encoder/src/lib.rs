//! APNG (Animated PNG) エンコーダ

mod alpha;
mod chunk;
mod delay;
mod diff;
mod encoder;
mod error;
mod filter;
mod region;
mod spool;
mod zlib;

pub use delay::FrameDelay;
pub use encoder::{COMPRESSION_LEVELS, ColorType, Config, DEFAULT_MAX_SPOOL_BYTES, Encoder};
pub use error::Error;
