use apng_encoder::COMPRESSION_LEVELS;
pub use aviutl2::ColorFormat;
use aviutl2::ini::{Ini, Properties};
use aviutl2::{IniConfig, MAX_REPEAT, read};
use std::num::NonZeroUsize;
use std::thread::available_parallelism;

/// 設定が採れるスレッド数の上限
///
/// この機械の論理CPU数。読めなければ1を返す。
pub fn max_threads() -> usize {
    available_parallelism().map_or(1, NonZeroUsize::get)
}

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
            threads: (max_threads() / 2).max(1),
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = read(section, "repeat", default.repeat).min(MAX_REPEAT);
        let color_format = read(section, "color_format", default.color_format);

        let compression_level = read(section, "compression_level", default.compression_level);
        let compression_level = if COMPRESSION_LEVELS.contains(&compression_level) {
            compression_level
        } else {
            default.compression_level
        };

        let threads = read(section, "threads", default.threads).clamp(1, max_threads());

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

    /// 既定のスレッド数は上限の内側で控えめに採る
    ///
    /// 上限をそのまま採ると、書き出しが機械を独り占めする。上限が1の機械では
    /// 1つしか採れないので、そこだけ上限と一致する。
    #[test]
    fn the_default_threads_stay_inside_the_ceiling() {
        let default = Config::default().threads;
        let ceiling = max_threads();

        assert!(default >= 1, "{default}");
        assert!(default <= ceiling, "{default} / {ceiling}");
        if ceiling >= 2 {
            assert!(
                default < ceiling,
                "上限をそのまま採っている: {default} / {ceiling}"
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
        assert_eq!(config.threads, default.threads);
    }
}
