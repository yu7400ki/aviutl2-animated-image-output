//! 出力プラグイン共通のピクセルフォーマット定義

use std::str::FromStr;

/// フレーム取得時のカラーフォーマット
#[derive(Copy, Clone, PartialEq, Eq, Default)]
pub enum ColorFormat {
    /// RGB 24bit (透過無し)
    #[default]
    Rgb24,
    /// RGBA 32bit (透過付き)
    Rgba32,
}

impl From<ColorFormat> for &'static str {
    fn from(value: ColorFormat) -> Self {
        match value {
            ColorFormat::Rgb24 => "透過無し",
            ColorFormat::Rgba32 => "透過付き",
        }
    }
}

impl FromStr for ColorFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(ColorFormat::Rgb24),
            Ok(1) => Ok(ColorFormat::Rgba32),
            _ => Err(()),
        }
    }
}

impl ColorFormat {
    /// ini保存やコンボボックス選択に使用するインデックス
    pub fn to_index(self) -> u32 {
        match self {
            ColorFormat::Rgb24 => 0,
            ColorFormat::Rgba32 => 1,
        }
    }
}
