#[derive(Debug, Clone, PartialEq)]
pub enum ColorFormat {
    Palette,     // 通常の256色パレット
    Transparent, // 透明色ありパレット
}

impl Default for ColorFormat {
    fn default() -> Self {
        ColorFormat::Palette
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
            color_format: ColorFormat::Palette,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default()
    }
}
