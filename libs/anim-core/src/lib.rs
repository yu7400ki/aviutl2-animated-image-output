//! アニメーション画像のエンコーダが画像フォーマットに依らず共有する部品

mod alpha;
mod diff;

pub use alpha::has_transparency;
pub use diff::{Rect, dirty_rect};
