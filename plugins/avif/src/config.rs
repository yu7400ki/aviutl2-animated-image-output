pub use aviutl2::ColorFormat;
use aviutl2::ini::{Ini, Properties};
use aviutl2::{IniConfig, MAX_REPEAT};
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::thread::available_parallelism;

/// 設定が採れるスレッド数の上限
///
/// この機械の論理CPU数。読めなければ1を返す。
pub fn max_threads() -> usize {
    available_parallelism().map_or(1, NonZeroUsize::get)
}

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

impl From<YuvFormat> for avif_encoder::YuvFormat {
    fn from(value: YuvFormat) -> Self {
        match value {
            YuvFormat::Yuv420 => avif_encoder::YuvFormat::Yuv420,
            YuvFormat::Yuv422 => avif_encoder::YuvFormat::Yuv422,
            YuvFormat::Yuv444 => avif_encoder::YuvFormat::Yuv444,
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
            speed: 6,
            color_format: ColorFormat::default(),
            yuv_format: YuvFormat::default(),
            threads: (max_threads() / 2).max(1),
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
            .unwrap_or(default.repeat)
            .min(MAX_REPEAT);

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
            .clamp(1, max_threads());

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
            speed: 9,
            color_format: ColorFormat::Rgba32,
            yuv_format: YuvFormat::Yuv444,
            // 既定は論理CPU数の半分なので、値域の上端を採る
            threads: max_threads(),
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

    /// 入力欄が扱えないループ回数は、扱える上限へ収まる
    ///
    /// i32へ折り返す値をそのまま持つと、ダイアログの初期値が負になる。
    #[test]
    fn out_of_range_num_plays_are_clamped() {
        assert_eq!(load(&[("repeat", "3000000000")]).repeat, MAX_REPEAT);
        assert_eq!(load(&[("repeat", "3")]).repeat, 3);
    }

    /// 値域の外のスレッド数は、ダイアログが扱える範囲へ収まる
    ///
    /// 下限を割ると並列化が効かず、上限を超えるとダイアログが開いたときに弾かれる。
    #[test]
    fn out_of_range_threads_are_clamped() {
        assert_eq!(load(&[("threads", "0")]).threads, 1);
        assert_eq!(
            load(&[("threads", &(max_threads() + 1).to_string())]).threads,
            max_threads()
        );
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
