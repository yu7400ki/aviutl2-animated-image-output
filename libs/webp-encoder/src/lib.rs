//! WebPエンコーダ

mod codec;
mod delay;
mod encoder;
mod error;
mod frame;
mod normalize;
mod picture;
mod pipeline;
mod riff;

pub use anim_core::{ColorType, FrameDelay, InputError};
pub use encoder::Encoder;
pub use error::{EncodingError, Error};

use std::ops::RangeInclusive;

/// [`Config::quality`] が採れる値
pub const QUALITY_RANGE: RangeInclusive<f32> = 0.0..=100.0;

/// [`Config::method`] が採れる値
pub const METHOD_RANGE: RangeInclusive<u8> = 0..=6;

/// [`Config::num_plays`] が書ける上限 (ANIMのループ数欄に収まる回数)
pub const MAX_NUM_PLAYS: u32 = u16::MAX as u32;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。
    pub color_type: ColorType,
    /// 可逆で符号化するか
    pub lossless: bool,
    /// 品質 ([`QUALITY_RANGE`] の範囲)
    ///
    /// 非可逆では画質を決める。可逆では画素を動かさず、圧縮率と速度を決める。
    pub quality: f32,
    /// 圧縮率と速度の均衡 ([`METHOD_RANGE`] の範囲)
    pub method: u8,
    /// アニメーションの再生回数 (0で無限ループ)
    ///
    /// [`MAX_NUM_PLAYS`] を超える回数は上限に丸めて書く。
    pub num_plays: u32,
}

/// 符号化の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// 前フレームと同一で、表示時間の延長に併合したフレーム数
    pub merged_frames: u32,
    /// 遅延を下限で切り上げたか
    pub delay_clamped: bool,
}
