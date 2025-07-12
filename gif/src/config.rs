#[derive(Debug, Clone, PartialEq)]
pub enum ColorFormat {
    Rgb24,
    Rgba32,
}

impl Default for ColorFormat {
    fn default() -> Self {
        ColorFormat::Rgba32
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub repeat: u16,
    pub color_format: ColorFormat,
}

impl Config {
    pub const fn default() -> Self {
        Config {
            repeat: 0,
            color_format: ColorFormat::Rgb24,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default()
    }
}
