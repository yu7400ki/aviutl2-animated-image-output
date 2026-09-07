//! アニメーション画像のエンコーダが画像フォーマットに依らず共有する部品

mod accumulator;
mod alpha;
mod color;
mod delay;
mod diff;
mod error;
mod pacing;
mod palette;
mod region;
mod rewrite;

pub use accumulator::{Accumulator, gcd};
pub use alpha::has_transparency;
pub use color::ColorType;
pub use delay::FrameDelay;
pub use diff::{Rect, dirty_rect, unchanged_run};
pub use error::{Error, InputError};
pub use pacing::Pacing;
pub use palette::{Colors, MAX_COLORS};
pub use region::crop;
pub use rewrite::{Profile, Rewrite, Span, Triggers};
