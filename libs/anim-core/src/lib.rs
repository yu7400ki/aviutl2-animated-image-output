//! アニメーション画像のエンコーダが画像フォーマットに依らず共有する部品

mod alpha;
mod color;
mod delay;
mod diff;
mod error;
mod pacing;
mod palette;
mod region;

pub use alpha::has_transparency;
pub use color::ColorType;
pub use delay::FrameDelay;
pub use diff::{Rect, dirty_rect};
pub use error::Error;
pub use pacing::Pacing;
pub use palette::{Colors, MAX_COLORS};
pub use region::{append_pixels, crop};
