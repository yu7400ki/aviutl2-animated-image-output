use apng_encoder::COMPRESSION_LEVELS;
pub use aviutl2::ColorFormat;
use aviutl2::ini::{Ini, Properties};
use aviutl2::{IniConfig, MAX_REPEAT, default_threads, max_threads, read, read_clamped};

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub compression_level: u32,
    pub threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            compression_level: 6,
            threads: default_threads(),
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = read_clamped(section, "repeat", 0..=MAX_REPEAT, default.repeat);
        let color_format = read(section, "color_format", default.color_format);
        let compression_level = read_clamped(
            section,
            "compression_level",
            COMPRESSION_LEVELS,
            default.compression_level,
        );
        let threads = read_clamped(section, "threads", 1..=max_threads(), default.threads);

        Self {
            repeat,
            color_format,
            compression_level,
            threads,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("compression_level", self.compression_level.to_string())
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
        let default = Config::default();
        assert_eq!(default.repeat, 0);
        assert!(default.color_format == ColorFormat::Rgb24);
        assert_eq!(default.compression_level, 6);

        let config = Config::load_from(None);
        assert_eq!(config.repeat, default.repeat);
        assert!(config.color_format == default.color_format);
        assert_eq!(config.compression_level, default.compression_level);
        assert_eq!(config.threads, default.threads);
    }

    #[test]
    fn saved_values_round_trip() {
        let saved = Config {
            repeat: 3,
            color_format: ColorFormat::Rgba32,
            compression_level: 9,
            threads: max_threads(),
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert!(loaded.color_format == saved.color_format);
        assert_eq!(loaded.compression_level, saved.compression_level);
        assert_eq!(loaded.threads, saved.threads);
    }

    /// 値域の外の圧縮レベルは、エンコーダが受け取れる範囲へ収まる
    #[test]
    fn out_of_range_compression_level_is_clamped() {
        assert_eq!(
            load(&[("compression_level", "10")]).compression_level,
            *COMPRESSION_LEVELS.end()
        );
        for value in ["0", "-1"] {
            assert_eq!(
                load(&[("compression_level", value)]).compression_level,
                *COMPRESSION_LEVELS.start(),
                "{value}"
            );
        }
    }

    /// 読めない圧縮レベルは既定値へ落ちる
    #[test]
    fn an_unreadable_compression_level_falls_back_to_default() {
        let default = Config::default().compression_level;
        for value in ["high", "6.5", ""] {
            assert_eq!(
                load(&[("compression_level", value)]).compression_level,
                default
            );
        }
    }

    /// 入力欄が扱えないループ回数は、扱える上限へ収まる
    ///
    /// i32へ折り返す値をそのまま持つと、ダイアログの初期値が負になる。
    #[test]
    fn out_of_range_num_plays_are_clamped() {
        assert_eq!(load(&[("repeat", "3000000000")]).repeat, MAX_REPEAT);
        assert_eq!(load(&[("repeat", "3")]).repeat, 3);
    }

    /// 値域の外のスレッド数は、走らせる機械の並列度の内側へ収まる
    ///
    /// 別の機械で書いた ini をそのまま読んでも、この機械で意味のある数になる。
    #[test]
    fn out_of_range_threads_are_clamped() {
        let over = (max_threads() + 1).to_string();

        assert_eq!(load(&[("threads", "0")]).threads, 1);
        assert_eq!(load(&[("threads", &over)]).threads, max_threads());
    }
}
