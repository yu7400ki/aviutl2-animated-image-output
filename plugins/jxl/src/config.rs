pub use aviutl2::ColorFormat;
use aviutl2::ini::{Ini, Properties};
use aviutl2::{IniConfig, MAX_REPEAT, default_threads, max_threads, read};
use jxl_encoder::{EFFORT_RANGE, QUALITY_RANGE};

#[derive(Clone)]
pub struct Config {
    pub repeat: u32,
    pub color_format: ColorFormat,
    pub quality: f32,
    pub effort: u8,
    pub threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            quality: 90.0,
            effort: 7,
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
        let quality = read(section, "quality", default.quality)
            .clamp(*QUALITY_RANGE.start(), *QUALITY_RANGE.end());
        let effort = read(section, "effort", default.effort)
            .clamp(*EFFORT_RANGE.start(), *EFFORT_RANGE.end());
        let threads = read(section, "threads", default.threads).clamp(1, max_threads());

        Self {
            repeat,
            color_format,
            quality,
            effort,
            threads,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("quality", self.quality.to_string())
            .set("effort", self.effort.to_string())
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
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.effort, default.effort);
        assert_eq!(config.threads, default.threads);
    }

    /// 既定の均衡は、掃引で選んだ値そのもの
    ///
    /// 出力サイズも SSIM もこの値で最も良くなる。動かすには測り直しが要る。
    #[test]
    fn the_default_effort_is_the_one_the_sweep_chose() {
        assert_eq!(Config::default().effort, 7);
    }

    /// 保存と読み戻しは、どの項目も既定と違う値で突き合わせる
    ///
    /// 既定と同じ値を通すと、その項目の書き出しが丸ごと落ちても読み戻しが揃う。
    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let saved = Config {
            repeat: 3,
            color_format: ColorFormat::Rgba32,
            quality: 100.0,
            effort: 9,
            // 既定は論理CPU数の半分なので、値域の上端を採る
            threads: max_threads(),
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert!(loaded.color_format == saved.color_format);
        assert_eq!(loaded.quality, saved.quality);
        assert_eq!(loaded.effort, saved.effort);
        assert_eq!(loaded.threads, saved.threads);
    }

    /// 設定ファイルの中身をそのまま読み、セクション名と項目名まで含めて確かめる
    #[test]
    fn a_config_file_written_before_still_loads() {
        let text = "\
[Config]
repeat=5
color_format=1
quality=80
effort=3
";
        let ini = Ini::load_from_str(text).unwrap();
        let config = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(config.repeat, 5);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert_eq!(config.quality, 80.0);
        assert_eq!(config.effort, 3);
    }

    /// 値域の外の品質と均衡は、エンコーダが受け取れる範囲へ収まる
    #[test]
    fn out_of_range_quality_and_effort_are_clamped() {
        let over = load(&[("quality", "1000"), ("effort", "99")]);
        assert_eq!(over.quality, *QUALITY_RANGE.end());
        assert_eq!(over.effort, *EFFORT_RANGE.end());

        let under = load(&[("quality", "-1"), ("effort", "0")]);
        assert_eq!(under.quality, *QUALITY_RANGE.start());
        assert_eq!(under.effort, *EFFORT_RANGE.start());
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
        let config = load(&[("repeat", "many"), ("effort", "3")]);
        let default = Config::default();

        assert_eq!(config.repeat, default.repeat);
        assert_eq!(config.effort, 3);
    }

    /// 認識しないキーだけのセクションは既定値になる
    ///
    /// 他のプラグインの設定を写した ini でも、こちらの既定は動かない。
    #[test]
    fn unknown_keys_are_ignored() {
        let config = load(&[("method", "6"), ("speed", "3")]);
        let default = Config::default();

        assert_eq!(config.repeat, default.repeat);
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.effort, default.effort);
    }
}
