use ini::Ini;
use std::path::PathBuf;
use std::str::FromStr;

fn get_config_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(std::env::var("ProgramData")?)
        .join("aviutl2")
        .join("Plugin");

    if !path.is_dir() {}

    match path.is_dir() {
        true => Ok(path.join(concat!(env!("CARGO_PKG_NAME"), ".ini"))),
        false => Err("Config directory does not exist".into()),
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum ColorFormat {
    Rgb24,
    Rgba32,
}

impl Default for ColorFormat {
    fn default() -> Self {
        ColorFormat::Rgb24
    }
}

impl Into<png::ColorType> for ColorFormat {
    fn into(self) -> png::ColorType {
        match self {
            ColorFormat::Rgb24 => png::ColorType::Rgb,
            ColorFormat::Rgba32 => png::ColorType::Rgba,
        }
    }
}

impl Into<&'static str> for ColorFormat {
    fn into(self) -> &'static str {
        match self {
            ColorFormat::Rgb24 => "RGB 24bit",
            ColorFormat::Rgba32 => "RGBA 32bit",
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
    fn to_index(&self) -> u32 {
        match self {
            ColorFormat::Rgb24 => 0,
            ColorFormat::Rgba32 => 1,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum CompressionType {
    Default,
    Fast,
    Best,
}

impl Default for CompressionType {
    fn default() -> Self {
        CompressionType::Default
    }
}

impl Into<png::Compression> for CompressionType {
    fn into(self) -> png::Compression {
        match self {
            CompressionType::Default => png::Compression::Default,
            CompressionType::Fast => png::Compression::Fast,
            CompressionType::Best => png::Compression::Best,
        }
    }
}

impl Into<&'static str> for CompressionType {
    fn into(self) -> &'static str {
        match self {
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
    fn to_index(&self) -> u32 {
        match self {
            CompressionType::Default => 0,
            CompressionType::Fast => 1,
            CompressionType::Best => 2,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum FilterType {
    None,
    Sub,
    Up,
    Average,
    Paeth,
}

impl Default for FilterType {
    fn default() -> Self {
        FilterType::None
    }
}

impl Into<png::FilterType> for FilterType {
    fn into(self) -> png::FilterType {
        match self {
            FilterType::None => png::FilterType::NoFilter,
            FilterType::Sub => png::FilterType::Sub,
            FilterType::Up => png::FilterType::Up,
            FilterType::Average => png::FilterType::Avg,
            FilterType::Paeth => png::FilterType::Paeth,
        }
    }
}

impl Into<&'static str> for FilterType {
    fn into(self) -> &'static str {
        match self {
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
    fn to_index(&self) -> u32 {
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
}

impl Config {
    pub const fn default() -> Self {
        Config {
            repeat: 0,
            color_format: ColorFormat::Rgb24,
            compression_type: CompressionType::Default,
            filter_type: FilterType::Sub,
        }
    }

    pub fn load() -> Self {
        let config_path = get_config_file_path();

        if !config_path.is_ok() {
            return Self::default();
        }

        if let Ok(ini) = Ini::load_from_file(&config_path.unwrap()) {
            if let Some(section) = ini.section(Some("Config")) {
                let repeat = section
                    .get("repeat")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                let color_format = section
                    .get("color_format")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default();

                let compression_type = section
                    .get("compression_type")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default();

                let filter_type = section
                    .get("filter_type")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default();

                Config {
                    repeat,
                    color_format,
                    compression_type,
                    filter_type,
                }
            } else {
                Self::default()
            }
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut ini = Ini::new();

        ini.with_section(Some("Config"))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set(
                "compression_type",
                self.compression_type.to_index().to_string(),
            )
            .set("filter_type", self.filter_type.to_index().to_string());

        let config_path = get_config_file_path()?;
        ini.write_to_file(&config_path)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default()
    }
}
