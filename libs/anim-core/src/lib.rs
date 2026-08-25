//! アニメーション画像のエンコーダが画像フォーマットに依らず共有する部品

mod alpha;
mod diff;
mod region;

pub use alpha::has_transparency;
pub use diff::{Rect, dirty_rect};
pub use region::{append_pixels, crop, paste};
