pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};

#[derive(Clone)]
pub struct Config {
    pub repeat: i32,
    pub color_format: ColorFormat,
    pub lossless: bool,
    pub quality: f32,
    pub method: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            lossless: false,
            quality: 75.0,
            method: 4,
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = section
            .and_then(|s| s.get("repeat"))
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let lossless = section
            .and_then(|s| s.get("lossless"))
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(default.lossless);

        let quality = section
            .and_then(|s| s.get("quality"))
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(default.quality)
            .clamp(0.0, 100.0);

        let method = section
            .and_then(|s| s.get("method"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.method)
            .clamp(0, 6);

        Self {
            repeat,
            color_format,
            lossless,
            quality,
            method,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("lossless", self.lossless.to_string())
            .set("quality", self.quality.to_string())
            .set("method", self.method.to_string());
    }
}
