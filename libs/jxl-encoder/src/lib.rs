//! JPEG XLエンコーダ

mod encoder;
mod error;
mod layout;

pub use encoder::Encoder;
pub use error::{EncodingError, Error, ErrorCode};
pub use layout::ColorType;

use std::ops::RangeInclusive;

/// [`Config::quality`] が採れる値
pub const QUALITY_RANGE: RangeInclusive<f32> = 0.0..=100.0;

/// [`Config::effort`] が採れる値
pub const EFFORT_RANGE: RangeInclusive<u8> = 1..=10;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。αを持つ色種別を
    /// 選ぶと、画像にαのチャネルが付く。
    pub color_type: ColorType,
    /// 可逆で符号化するか
    pub lossless: bool,
    /// 品質 ([`QUALITY_RANGE`] の範囲)
    ///
    /// 上限は可逆と同義になる。αは常に可逆。
    pub quality: f32,
    /// 速度と圧縮率の均衡 ([`EFFORT_RANGE`] の範囲)
    ///
    /// 大きいほど遅く小さくなる。
    pub effort: u8,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
    /// 1秒あたりのtick数の分子
    pub tps_numerator: u32,
    /// 1秒あたりのtick数の分母
    pub tps_denominator: u32,
    /// 符号化に使うスレッド数
    pub max_threads: u32,
}
