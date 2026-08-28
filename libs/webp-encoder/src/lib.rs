//! WebPエンコーダ

mod codec;
mod delay;
mod encoder;
mod error;
mod frame;
mod layout;
mod normalize;
mod picture;
mod pipeline;
mod riff;

pub use anim_core::FrameDelay;
pub use encoder::Encoder;
pub use error::{EncodingError, Error};
pub use layout::ColorType;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。
    pub color_type: ColorType,
    /// 可逆で符号化するか
    pub lossless: bool,
    /// 品質 0.0..=100.0 (非可逆では画質、可逆では圧縮の努力)
    pub quality: f32,
    /// 速度と圧縮率の均衡 0..=6
    pub method: u8,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
}

/// 符号化の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// 前フレームと同一で、表示時間の延長に併合したフレーム数
    pub merged_frames: u32,
    /// 遅延を下限で切り上げたか
    pub delay_clamped: bool,
    /// 投入したフレームのいずれかに透過画素があったか
    pub has_alpha: bool,
}
