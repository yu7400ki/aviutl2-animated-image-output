//! アニメーション画像のエンコーダが画像フォーマットに依らず共有する部品

mod alpha;
mod delay;
mod diff;
mod error;
mod region;

pub use alpha::has_transparency;
pub use delay::FrameDelay;
pub use diff::{Rect, dirty_rect};
pub use error::Error;
pub use region::{append_pixels, crop, paste};
