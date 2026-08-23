use apng_encoder::COMPRESSION_LEVELS;
pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub compression_level: u32,
    pub reduce_color: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            compression_level: 6,
            reduce_color: false,
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

        let color_format = section
            .and_then(|s| s.get("color_format"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or(default.color_format);

        let compression_level = section
            .and_then(|s| s.get("compression_level"))
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|level| COMPRESSION_LEVELS.contains(level))
            .unwrap_or(default.compression_level);

        let reduce_color = section
            .and_then(|s| s.get("reduce_color"))
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(default.reduce_color);

        Self {
            repeat,
            color_format,
            compression_level,
            reduce_color,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("compression_level", self.compression_level.to_string())
            .set("reduce_color", self.reduce_color.to_string());
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
        let default = Config::default();
        assert_eq!(default.repeat, 0);
        assert!(default.color_format == ColorFormat::Rgb24);
        assert_eq!(default.compression_level, 6);
        assert!(!default.reduce_color);

        let config = Config::load_from(None);
        assert_eq!(config.repeat, default.repeat);
        assert!(config.color_format == default.color_format);
        assert_eq!(config.compression_level, default.compression_level);
        assert_eq!(config.reduce_color, default.reduce_color);
    }

    #[test]
    fn saved_values_round_trip() {
        let saved = Config {
            repeat: 3,
            color_format: ColorFormat::Rgba32,
            compression_level: 9,
            reduce_color: true,
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert!(loaded.color_format == saved.color_format);
        assert_eq!(loaded.compression_level, saved.compression_level);
        assert_eq!(loaded.reduce_color, saved.reduce_color);
    }

    /// 色数の最適化を持たない設定ファイルは、最適化しない状態で読める
    #[test]
    fn a_config_without_the_reduce_color_key_falls_back_to_off() {
        let config = load(&[
            ("repeat", "3"),
            ("color_format", "1"),
            ("compression_level", "9"),
        ]);

        assert_eq!(config.repeat, 3);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert_eq!(config.compression_level, 9);
        assert!(!config.reduce_color);
    }

    /// 真偽値として読めない色数の最適化は既定値になる
    #[test]
    fn an_unparsable_reduce_color_falls_back_to_the_default() {
        for value in ["yes", "1", ""] {
            assert!(!load(&[("reduce_color", value)]).reduce_color);
        }
        assert!(load(&[("reduce_color", "true")]).reduce_color);
    }

    #[test]
    fn out_of_range_compression_level_falls_back_to_default() {
        let default = Config::default().compression_level;
        for value in ["0", "10", "-1", "high", ""] {
            assert_eq!(
                load(&[("compression_level", value)]).compression_level,
                default
            );
        }
    }

    /// 認識しないキーだけのセクションは既定値になる
    #[test]
    fn unknown_keys_are_ignored() {
        let config = load(&[("compression_type", "2"), ("filter_type", "4")]);
        let default = Config::default();
        assert_eq!(config.repeat, default.repeat);
        assert!(config.color_format == default.color_format);
        assert_eq!(config.compression_level, default.compression_level);
        assert_eq!(config.reduce_color, default.reduce_color);
    }
}
