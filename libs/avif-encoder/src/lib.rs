//! AVIFエンコーダ

mod encoder;
mod error;
mod image;
mod layout;

pub use anim_core::ColorType;
pub use encoder::{Encoder, OperatingPoint, Usage};
pub use error::{EncodingError, Error};
pub use image::YuvFormat;

use std::ops::RangeInclusive;

/// [`Config::quality`] が採れる値
pub const QUALITY_RANGE: RangeInclusive<u8> = 0..=100;

/// [`Config::speed`] が採れる値
pub const SPEED_RANGE: RangeInclusive<u8> = 0..=10;

/// エンコード設定
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 入力フレームの色種別
    ///
    /// [`Encoder::add_frame`] に渡すバイト列の解釈を決める。αを持つ色種別を
    /// 選ぶと、シーケンスにはαのストリームが付く。
    pub color_type: ColorType,
    /// 品質 ([`QUALITY_RANGE`] の範囲)
    ///
    /// 色とαの両方に使う。
    pub quality: u8,
    /// 速度と圧縮率の均衡 ([`SPEED_RANGE`] の範囲)
    ///
    /// 解決される動作点は [`Config::operating_point`] が返す。
    pub speed: u8,
    /// クロマサブサンプリング
    pub yuv_format: YuvFormat,
    /// アニメーションの再生回数 (0で無限ループ)
    pub num_plays: u32,
    /// 1秒あたりの時間刻み数
    pub timescale: u32,
    /// 符号化に使うスレッド数の上限
    pub max_threads: u32,
}
