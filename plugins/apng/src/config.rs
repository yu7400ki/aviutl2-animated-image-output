pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};

/// 圧縮レベルの有効範囲
pub const COMPRESSION_LEVEL_RANGE: std::ops::RangeInclusive<u32> = 1..=9;

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub compression_level: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            compression_level: 6,
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = section
            .and_then(|s| s.get("repeat"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or(default.color_format);

        let compression_level = section
            .and_then(|s| s.get("compression_level"))
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|level| COMPRESSION_LEVEL_RANGE.contains(level))
            .unwrap_or(default.compression_level);

        Self {
            repeat,
            color_format,
            compression_level,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("compression_level", self.compression_level.to_string());
    }
}
