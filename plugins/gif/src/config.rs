pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};

#[derive(Clone)]
pub struct Config {
    pub repeat: u16,
    pub color_format: ColorFormat,
    pub speed: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            speed: 10,
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = section
            .and_then(|s| s.get("repeat"))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let speed = section
            .and_then(|s| s.get("speed"))
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(default.speed);

        Self {
            repeat,
            color_format,
            speed,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("speed", self.speed.to_string());
    }
}
