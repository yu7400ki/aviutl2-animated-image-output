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

    /// 画面へ出す項目名
    pub fn label(self) -> &'static str {
        match self {
            ColorFormat::Rgb24 => "透過無し",
            ColorFormat::Rgba32 => "透過付き",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// iniへ書くインデックスは、そのまま読み戻せる
    #[test]
    fn the_index_written_to_the_ini_reads_back() {
        for format in [ColorFormat::Rgb24, ColorFormat::Rgba32] {
            let written = format.to_index().to_string();
            assert!(written.parse::<ColorFormat>() == Ok(format), "{written}");
        }
    }

    /// 画面へ出す項目名は、iniの値と別のアルファベットを持つ
    #[test]
    fn the_label_is_not_an_ini_value() {
        for format in [ColorFormat::Rgb24, ColorFormat::Rgba32] {
            let label = format.label();
            assert!(label.parse::<ColorFormat>().is_err(), "{label}");
        }
    }
}
