pub use aviutl2::ColorFormat;
use aviutl2::ini::{Ini, Properties};
use aviutl2::{IniConfig, read};

#[derive(Clone, Default)]
pub struct Config {
    pub repeat: u16,
    pub color_format: ColorFormat,
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = read(section, "repeat", u32::from(default.repeat));
        let repeat = u16::try_from(repeat).unwrap_or(u16::MAX);
        let color_format = read(section, "color_format", default.color_format);

        Self {
            repeat,
            color_format,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string());
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

        let config = Config::load_from(None);
        assert_eq!(config.repeat, default.repeat);
        assert!(config.color_format == default.color_format);
    }

    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let saved = Config {
            repeat: 3,
            color_format: ColorFormat::Rgba32,
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert!(loaded.color_format == saved.color_format);
    }

    /// 書き出した設定ファイルにエンコード速度の項目は残らない
    #[test]
    fn a_saved_config_has_no_speed_key() {
        let mut ini = Ini::new();
        Config::default().save_to(&mut ini);

        let section = ini.section(Some(Config::SECTION)).unwrap();
        assert!(section.get("speed").is_none());
        assert!(section.get("repeat").is_some());
        assert!(section.get("color_format").is_some());
    }

    /// 以前のバージョンが書いたエンコード速度は、他の項目を妨げずに無視される
    #[test]
    fn a_leftover_speed_key_is_ignored() {
        let config = load(&[("repeat", "5"), ("color_format", "1"), ("speed", "30")]);

        assert_eq!(config.repeat, 5);
        assert!(config.color_format == ColorFormat::Rgba32);
    }

    /// 読めないループ回数は既定値になる
    #[test]
    fn an_unreadable_repeat_falls_back_to_default() {
        let default = Config::default().repeat;
        for value in ["-1", "many", ""] {
            assert_eq!(load(&[("repeat", value)]).repeat, default);
        }
    }

    /// NETSCAPE拡張が持てる回数を超えたループ回数は、上限へ収まる
    ///
    /// 既定は0で、GIFでは0が無限ループを指す。超過値をそこへ落とすと意味が反転する。
    #[test]
    fn out_of_range_repeat_is_clamped() {
        assert_eq!(load(&[("repeat", "70000")]).repeat, u16::MAX);
        assert_eq!(load(&[("repeat", "65536")]).repeat, u16::MAX);
        assert_eq!(load(&[("repeat", "65535")]).repeat, u16::MAX);
        assert_eq!(load(&[("repeat", "3")]).repeat, 3);
    }

    /// 認識しないキーだけのセクションは既定値になる
    #[test]
    fn unknown_keys_are_ignored() {
        let config = load(&[("speed", "10"), ("dither", "true")]);
        let default = Config::default();
        assert_eq!(config.repeat, default.repeat);
        assert!(config.color_format == default.color_format);
    }
}
