pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};
use jxl_encoder::{EFFORT_RANGE, QUALITY_RANGE};
use std::thread::available_parallelism;

/// 設定が採れるループ回数の上限
///
/// ダイアログの数値欄が扱える上限。
pub const MAX_NUM_PLAYS: u32 = i32::MAX as u32;

/// 設定が採れるスレッド数の上限
///
/// この機械の論理CPU数。読めなければ1を返す。
pub fn available_threads() -> u32 {
    available_parallelism().map_or(1, |p| p.get() as u32)
}

#[derive(Clone)]
pub struct Config {
    pub num_plays: u32,
    pub color_format: ColorFormat,
    pub lossless: bool,
    pub quality: f32,
    pub effort: u8,
    pub max_threads: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num_plays: 0,
            color_format: ColorFormat::default(),
            lossless: false,
            quality: 90.0,
            effort: 7,
            max_threads: available_threads(),
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let num_plays = section
            .and_then(|s| s.get("num_plays"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(default.num_plays)
            .min(MAX_NUM_PLAYS);

        let color_format = section
            .and_then(|s| s.get("color_type"))
            .and_then(|s| s.parse::<ColorFormat>().ok())
            .unwrap_or_default();

        let lossless = section
            .and_then(|s| s.get("lossless"))
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(default.lossless);

        let quality = section
            .and_then(|s| s.get("quality"))
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(default.quality)
            .clamp(*QUALITY_RANGE.start(), *QUALITY_RANGE.end());

        let effort = section
            .and_then(|s| s.get("effort"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.effort)
            .clamp(*EFFORT_RANGE.start(), *EFFORT_RANGE.end());

        let threads = section
            .and_then(|s| s.get("max_threads"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(default.max_threads)
            .clamp(1, available_threads());

        Self {
            num_plays,
            color_format,
            lossless,
            quality,
            effort,
            max_threads: threads,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("num_plays", self.num_plays.to_string())
            .set("color_type", self.color_format.to_index().to_string())
            .set("lossless", self.lossless.to_string())
            .set("quality", self.quality.to_string())
            .set("effort", self.effort.to_string())
            .set("max_threads", self.max_threads.to_string());
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

        assert_eq!(config.num_plays, default.num_plays);
        assert!(config.color_format == default.color_format);
        assert_eq!(config.lossless, default.lossless);
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.effort, default.effort);
        assert_eq!(config.max_threads, default.max_threads);
    }

    /// 保存と読み戻しは、どの項目も既定と違う値で突き合わせる
    ///
    /// 既定と同じ値を通すと、その項目の書き出しが丸ごと落ちても読み戻しが揃う。
    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let saved = Config {
            num_plays: 3,
            color_format: ColorFormat::Rgba32,
            lossless: true,
            quality: 100.0,
            effort: 9,
            // 既定は論理CPU数そのものなので、1つ下を採る
            max_threads: available_threads().saturating_sub(1).max(1),
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.num_plays, saved.num_plays);
        assert!(loaded.color_format == saved.color_format);
        assert_eq!(loaded.lossless, saved.lossless);
        assert_eq!(loaded.quality, saved.quality);
        assert_eq!(loaded.effort, saved.effort);
        assert_eq!(loaded.max_threads, saved.max_threads);
    }

    /// 設定ファイルの中身をそのまま読み、セクション名と項目名まで含めて確かめる
    #[test]
    fn a_config_file_written_before_still_loads() {
        let text = "\
[Config]
num_plays=5
color_type=1
lossless=true
quality=80
effort=3
";
        let ini = Ini::load_from_str(text).unwrap();
        let config = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(config.num_plays, 5);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert!(config.lossless);
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
        assert_eq!(
            load(&[("num_plays", "3000000000")]).num_plays,
            MAX_NUM_PLAYS
        );
        assert_eq!(load(&[("num_plays", "3")]).num_plays, 3);
    }

    /// 値域の外のスレッド数は、走らせる機械の並列度の内側へ収まる
    ///
    /// 別の機械で書いた ini をそのまま読んでも、この機械で意味のある数になる。
    #[test]
    fn out_of_range_threads_are_clamped() {
        let over = (available_threads() + 1).to_string();

        assert_eq!(load(&[("max_threads", "0")]).max_threads, 1);
        assert_eq!(
            load(&[("max_threads", &over)]).max_threads,
            available_threads()
        );
    }

    /// 読めない値の項目だけが既定値へ落ちる
    #[test]
    fn an_unreadable_value_falls_back_on_its_own() {
        let config = load(&[("num_plays", "many"), ("effort", "3")]);
        let default = Config::default();

        assert_eq!(config.num_plays, default.num_plays);
        assert_eq!(config.effort, 3);
    }

    /// 認識しないキーだけのセクションは既定値になる
    ///
    /// 他のプラグインの設定を写した ini でも、こちらの既定は動かない。
    #[test]
    fn unknown_keys_are_ignored() {
        let config = load(&[("method", "6"), ("speed", "3")]);
        let default = Config::default();

        assert_eq!(config.num_plays, default.num_plays);
        assert_eq!(config.lossless, default.lossless);
        assert_eq!(config.quality, default.quality);
        assert_eq!(config.effort, default.effort);
    }
}
