//! JPEG XLエンコーダ

// `tests/support` は公開APIの名前で書かれており、crateの内側からも同じ名前で引く
#[cfg(test)]
extern crate self as jxl_encoder;

mod delta;
mod encoder;
mod error;
mod split;
#[cfg(test)]
mod tests;

pub use anim_core::{ColorType, InputError};
pub use encoder::Encoder;
pub use error::{EncodingError, Error, ErrorCode};

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
    /// 品質 ([`QUALITY_RANGE`] の範囲)
    ///
    /// 上限で可逆になる。αは常に可逆。
    pub quality: f32,
    /// 速度と圧縮率の均衡 ([`EFFORT_RANGE`] の範囲)
    ///
    /// 大きいほど時間がかかる。縮む量は素材によって変わる。
    pub effort: u8,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
    /// 1秒あたりのtick数の分子
    ///
    /// 分母との比は最大公約数で約されてから書かれる。約した分子は1以上
    /// 1073741824以下、分母は1以上1024以下に収まる必要がある。
    pub tps_numerator: u32,
    /// 1秒あたりのtick数の分母
    pub tps_denominator: u32,
    /// 符号化に使うスレッド数
    pub max_threads: u32,
}
