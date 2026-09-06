pub use aviutl2::ColorFormat;
use aviutl2::ini::{Ini, Properties};
use aviutl2::{IniConfig, MAX_REPEAT, default_threads, max_threads, read, read_flag};
use webp_encoder::{METHOD_RANGE, QUALITY_RANGE};

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub lossless: bool,
    pub quality: u8,
    pub method: u8,
    pub threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            lossless: false,
            quality: 75,
            method: 4,
            threads: default_threads(),
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = read(section, "repeat", default.repeat).min(MAX_REPEAT);
        let color_format = read(section, "color_format", default.color_format);
        let lossless = read_flag(section, "lossless", default.lossless);
        let quality = read(section, "quality", default.quality)
            .clamp(*QUALITY_RANGE.start() as u8, *QUALITY_RANGE.end() as u8);
        let method = read(section, "method", default.method)
            .clamp(*METHOD_RANGE.start(), *METHOD_RANGE.end());
        let threads = read(section, "threads", default.threads).clamp(1, max_threads());

        Self {
            repeat,
            color_format,
            lossless,
            quality,
            method,
            threads,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("lossless", u32::from(self.lossless).to_string())
            .set("quality", self.quality.to_string())
            .set("method", self.method.to_string())
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
        assert!(config.color_format == default.color_format);
        assert_eq!(config.lossless, default.lossless);
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.method, default.method);
        assert_eq!(config.threads, default.threads);
    }

    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let saved = Config {
            repeat: 3,
            color_format: ColorFormat::Rgba32,
            lossless: true,
            quality: 100,
            method: 6,
            threads: max_threads(),
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert!(loaded.color_format == saved.color_format);
        assert_eq!(loaded.lossless, saved.lossless);
        assert_eq!(loaded.quality, saved.quality);
        assert_eq!(loaded.method, saved.method);
        assert_eq!(loaded.threads, saved.threads);
    }

    /// 設定ファイルの中身をそのまま読み、セクション名と項目名まで含めて確かめる
    #[test]
    fn a_config_file_written_before_still_loads() {
        let text = "\
[Config]
repeat=5
color_format=1
lossless=1
quality=90
method=3
";
        let ini = Ini::load_from_str(text).unwrap();
        let config = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(config.repeat, 5);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert!(config.lossless);
        assert_eq!(config.quality, 90);
        assert_eq!(config.method, 3);
    }

    /// 可逆の指定は、iniでは0と1で表す
    #[test]
    fn the_lossless_flag_is_read_as_zero_or_one() {
        assert!(load(&[("lossless", "1")]).lossless);
        assert!(!load(&[("lossless", "0")]).lossless);
    }

    /// 0でも1でもない可逆の指定は既定値へ落ちる
    #[test]
    fn an_unreadable_lossless_flag_falls_back_to_default() {
        let default = Config::default().lossless;
        for value in ["true", "True", "yes", "2", "-1", ""] {
            assert_eq!(load(&[("lossless", value)]).lossless, default, "{value}");
        }
    }

    /// 書き出した可逆の指定は、他の項目と同じ0と1の表現になる
    #[test]
    fn the_lossless_flag_is_written_as_zero_or_one() {
        for (lossless, written) in [(true, "1"), (false, "0")] {
            let mut ini = Ini::new();
            Config {
                lossless,
                ..Config::default()
            }
            .save_to(&mut ini);

            let section = ini.section(Some(Config::SECTION)).unwrap();
            assert_eq!(section.get("lossless"), Some(written));
        }
    }

    /// 値域の外の品質とメソッドは、エンコーダが受け取れる範囲へ収まる
    #[test]
    fn out_of_range_quality_and_method_are_clamped() {
        let config = load(&[("quality", "200"), ("method", "99")]);

        assert_eq!(config.quality, *QUALITY_RANGE.end() as u8);
        assert_eq!(config.method, *METHOD_RANGE.end());
    }

    /// 0から100の整数として読めない品質は既定値へ落ちる
    ///
    /// ダイアログは整数しか受け取らないので、iniもそこへ揃える。
    #[test]
    fn a_quality_the_ini_cannot_read_falls_back_to_default() {
        let default = Config::default().quality;
        for value in ["nan", "inf", "87.5", "-1.5", "-1", "1000"] {
            assert_eq!(load(&[("quality", value)]).quality, default, "{value}");
        }
    }

    /// 負のループ回数は既定値へ落ちる
    #[test]
    fn a_negative_repeat_falls_back_to_default() {
        let default = Config::default();
        assert_eq!(load(&[("repeat", "-5")]).repeat, default.repeat);
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

    /// 読めない値の項目だけが既定値へ落ちる
    #[test]
    fn an_unreadable_value_falls_back_on_its_own() {
        let config = load(&[("repeat", "many"), ("lossless", "1")]);
        let default = Config::default();

        assert_eq!(config.repeat, default.repeat);
        assert!(config.lossless);
    }

    /// 認識しないキーだけのセクションは既定値になる
    ///
    /// 他のプラグインの設定を写した ini でも、こちらの既定は動かない。
    #[test]
    fn unknown_keys_are_ignored() {
        let config = load(&[("compression_level", "6"), ("reduce_color", "true")]);
        let default = Config::default();

        assert_eq!(config.repeat, default.repeat);
        assert_eq!(config.lossless, default.lossless);
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.method, default.method);
    }
}
