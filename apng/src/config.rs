#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone)]
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
}

impl Default for Config {
    fn default() -> Self {
        Self::default()
    }
}
