pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};
use std::str::FromStr;

#[derive(Copy, Clone, PartialEq, Default)]
pub enum CompressionType {
    #[default]
    Default,
    Fast,
    Best,
}

impl From<CompressionType> for png::Compression {
    fn from(value: CompressionType) -> Self {
        match value {
            CompressionType::Default => png::Compression::Default,
            CompressionType::Fast => png::Compression::Fast,
            CompressionType::Best => png::Compression::Best,
        }
    }
}

impl From<CompressionType> for &'static str {
    fn from(value: CompressionType) -> Self {
        match value {
            CompressionType::Default => "標準",
            CompressionType::Fast => "高速",
            CompressionType::Best => "最高",
        }
    }
}

impl FromStr for CompressionType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(CompressionType::Default),
            Ok(1) => Ok(CompressionType::Fast),
            Ok(2) => Ok(CompressionType::Best),
            _ => Err(()),
        }
    }
}

impl CompressionType {
    fn to_index(self) -> u32 {
        match self {
            CompressionType::Default => 0,
            CompressionType::Fast => 1,
            CompressionType::Best => 2,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Default)]
pub enum FilterType {
    None,
    #[default]
    Sub,
    Up,
    Average,
    Paeth,
}

impl From<FilterType> for png::FilterType {
    fn from(value: FilterType) -> Self {
        match value {
            FilterType::None => png::FilterType::NoFilter,
            FilterType::Sub => png::FilterType::Sub,
            FilterType::Up => png::FilterType::Up,
            FilterType::Average => png::FilterType::Avg,
            FilterType::Paeth => png::FilterType::Paeth,
        }
    }
}

impl From<FilterType> for &'static str {
    fn from(value: FilterType) -> Self {
        match value {
            FilterType::None => "なし",
            FilterType::Sub => "Sub",
            FilterType::Up => "Up",
            FilterType::Average => "Average",
            FilterType::Paeth => "Paeth",
        }
    }
}

impl FromStr for FilterType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(FilterType::None),
            Ok(1) => Ok(FilterType::Sub),
            Ok(2) => Ok(FilterType::Up),
            Ok(3) => Ok(FilterType::Average),
            Ok(4) => Ok(FilterType::Paeth),
            _ => Err(()),
        }
    }
}

impl FilterType {
    fn to_index(self) -> u32 {
        match self {
            FilterType::None => 0,
            FilterType::Sub => 1,
            FilterType::Up => 2,
            FilterType::Average => 3,
            FilterType::Paeth => 4,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub compression_type: CompressionType,
    pub filter_type: FilterType,
    pub adaptive_filter: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            compression_type: CompressionType::default(),
            filter_type: FilterType::default(),
            adaptive_filter: true,
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = section
            .and_then(|s| s.get("repeat"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(default.color_format);

        let compression_type = section
            .and_then(|s| s.get("compression_type"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(default.compression_type);

        let filter_type = section
            .and_then(|s| s.get("filter_type"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(default.filter_type);

        let adaptive_filter = section
            .and_then(|s| s.get("adaptive_filter"))
            .and_then(|s| s.parse::<u32>().ok())
            .map(|v| v != 0)
            .unwrap_or(default.adaptive_filter);

        Config {
            repeat,
            color_format,
            compression_type,
            filter_type,
            adaptive_filter,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set(
                "compression_type",
                self.compression_type.to_index().to_string(),
            )
            .set("filter_type", self.filter_type.to_index().to_string())
            .set("adaptive_filter", (self.adaptive_filter as u32).to_string());
    }
}
