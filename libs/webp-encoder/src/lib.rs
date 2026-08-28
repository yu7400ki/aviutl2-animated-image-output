//! WebPエンコーダ

mod codec;
mod error;
mod layout;
mod picture;

pub use codec::{Config, EncodedFrame};
pub use error::{EncodingError, Error};
pub use layout::ColorType;

use crate::codec::Codec;
use crate::layout::Layout;

/// `width` x `height` の1フレームを単葉の .webp として符号化する
///
/// `data` は [`Config::color_type`] の画素が隙間なく並んでいること。
///
/// # Errors
/// 寸法が0か16383を超えるとき [`Error::InvalidDimensions`]。`data` の長さが
/// 寸法と色種別に合わないとき [`Error::FrameSizeMismatch`]。符号化に失敗した
/// とき [`Error::Encode`]。
pub fn encode(
    width: u32,
    height: u32,
    data: &[u8],
    config: &Config,
) -> Result<EncodedFrame, Error> {
    let layout = Layout::new(width, height, config.color_type)?;
    layout.check_frame(data)?;

    let mut codec = Codec::new(config)?;
    codec.encode(data, &layout, layout.whole())
}
