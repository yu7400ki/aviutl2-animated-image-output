//! APNG (Animated PNG) エンコーダ

mod chunk;
mod delay;
mod encoder;
mod error;
mod filter;
mod zlib;

pub use delay::FrameDelay;
pub use encoder::{ColorType, Config, Encoder};
pub use error::Error;
