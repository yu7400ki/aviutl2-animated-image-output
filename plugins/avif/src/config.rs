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
            .unwrap_or(default.threads)
            .max(1);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn load(entries: &[(&str, &str)]) -> Config {
        let mut ini = Ini::new();
        let mut section = ini.with_section(Some(Config::SECTION));
        for (key, value) in entries {
            section.set(*key, *value);
        }
        Config::load_from(ini.section(Some(Config::SECTION)))
    }

    #[test]
    fn missing_section_falls_back_to_default() {
        let config = Config::load_from(None);
        let default = Config::default();

        assert_eq!(config.repeat, default.repeat);
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.speed, default.speed);
        assert!(config.color_format == default.color_format);
        assert!(config.yuv_format == default.yuv_format);
        assert_eq!(config.threads, default.threads);
    }

    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let saved = Config {
            repeat: 3,
            quality: 90,
            speed: 6,
            color_format: ColorFormat::Rgba32,
            yuv_format: YuvFormat::Yuv444,
            threads: 4,
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert_eq!(loaded.quality, saved.quality);
        assert_eq!(loaded.speed, saved.speed);
        assert!(loaded.color_format == saved.color_format);
        assert!(loaded.yuv_format == saved.yuv_format);
        assert_eq!(loaded.threads, saved.threads);
    }

    /// 値域の外の品質と速度は、エンコーダが受け取れる範囲へ収まる
    #[test]
    fn out_of_range_quality_and_speed_are_clamped() {
        let config = load(&[("quality", "200"), ("speed", "99")]);

        assert_eq!(config.quality, 100);
        assert_eq!(config.speed, 10);
    }

    /// 0や未検査の値のスレッド数は、1以上へ寄る
    ///
    /// 符号化器はスレッド数を並列化の可否にしか使わず、0は無検査で
    /// 渡すと並列化が効かないだけだが、意味のある下限として1を保つ。
    #[test]
    fn a_zero_thread_count_is_clamped_to_at_least_one() {
        assert_eq!(load(&[("threads", "0")]).threads, 1);
    }

    /// 読めない値の項目だけが既定値へ落ちる
    #[test]
    fn an_unreadable_value_falls_back_on_its_own() {
        let config = load(&[("repeat", "many"), ("speed", "3")]);
        let default = Config::default();

        assert_eq!(config.repeat, default.repeat);
        assert_eq!(config.speed, 3);
    }
}
