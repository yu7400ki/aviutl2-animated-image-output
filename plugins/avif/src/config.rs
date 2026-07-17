pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};
use std::str::FromStr;

#[derive(Copy, Clone, PartialEq, Default)]
pub enum YuvFormat {
    #[default]
    Yuv420,
    Yuv422,
    Yuv444,
}

impl From<YuvFormat> for &'static str {
    fn from(value: YuvFormat) -> Self {
        match value {
            YuvFormat::Yuv420 => "YUV420",
            YuvFormat::Yuv422 => "YUV422",
            YuvFormat::Yuv444 => "YUV444",
        }
    }
}

impl FromStr for YuvFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<u32>() {
            Ok(0) => Ok(YuvFormat::Yuv420),
            Ok(1) => Ok(YuvFormat::Yuv422),
            Ok(2) => Ok(YuvFormat::Yuv444),
            _ => Err(()),
        }
    }
}

impl From<YuvFormat> for rustavif::PixelFormat {
    fn from(value: YuvFormat) -> Self {
        match value {
            YuvFormat::Yuv420 => rustavif::PixelFormat::Yuv420,
            YuvFormat::Yuv422 => rustavif::PixelFormat::Yuv422,
            YuvFormat::Yuv444 => rustavif::PixelFormat::Yuv444,
        }
    }
}

impl YuvFormat {
    fn to_index(self) -> u32 {
        match self {
            YuvFormat::Yuv420 => 0,
            YuvFormat::Yuv422 => 1,
            YuvFormat::Yuv444 => 2,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub quality: u8,
    pub speed: u8,
    pub color_format: ColorFormat,
    pub yuv_format: YuvFormat,
    pub threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            quality: 75,
            speed: 10,
            color_format: ColorFormat::default(),
            yuv_format: YuvFormat::default(),
            threads: std::thread::available_parallelism().map_or(1, |p| p.get()),
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

        let quality = section
            .and_then(|s| s.get("quality"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.quality)
            .clamp(0, 100);

        let speed = section
            .and_then(|s| s.get("speed"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.speed)
            .clamp(0, 10);

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let yuv_format = section
            .and_then(|s| s.get("yuv_format"))
            .and_then(|s| s.parse::<YuvFormat>().ok())
            .unwrap_or_default();

        let threads = section
            .and_then(|s| s.get("threads"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default.threads);

        Self {
            repeat,
            quality,
            speed,
            color_format,
            yuv_format,
            threads,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("quality", self.quality.to_string())
            .set("speed", self.speed.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("yuv_format", self.yuv_format.to_index().to_string())
            .set("threads", self.threads.to_string());
    }
}
