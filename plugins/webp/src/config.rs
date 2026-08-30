pub use aviutl2::ColorFormat;
use aviutl2::IniConfig;
use aviutl2::ini::{Ini, Properties};
use std::num::NonZeroUsize;
use std::thread::available_parallelism;

/// 符号化を回せるワーカー数の上限
///
/// 符号化はCPUバウンドなので、論理CPUを超えて起こしても処理量は増えず、
/// 抱える量と切り替えの手間だけが伸びる。機械の並列度を読めなければ1を返す。
pub fn max_workers() -> usize {
    available_parallelism().map_or(1, NonZeroUsize::get)
}

#[derive(Clone)]
pub struct Config {
    pub repeat: i32,
    pub color_format: ColorFormat,
    pub lossless: bool,
    pub quality: f32,
    pub method: u8,
    pub workers: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repeat: 0,
            color_format: ColorFormat::default(),
            lossless: false,
            quality: 75.0,
            method: 4,
            workers: (max_workers() / 2).max(1),
        }
    }
}

impl IniConfig for Config {
    const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");

    fn load_from(section: Option<&Properties>) -> Self {
        let default = Self::default();

        let repeat = section
            .and_then(|s| s.get("repeat"))
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(default.repeat);

        let color_format = section
            .and_then(|s| s.get("color_format"))
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
            .clamp(0.0, 100.0);

        let method = section
            .and_then(|s| s.get("method"))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(default.method)
            .clamp(0, 6);

        let workers = section
            .and_then(|s| s.get("workers"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default.workers)
            .clamp(1, max_workers());

        Self {
            repeat,
            color_format,
            lossless,
            quality,
            method,
            workers,
        }
    }

    fn save_to(&self, ini: &mut Ini) {
        ini.with_section(Some(Self::SECTION))
            .set("repeat", self.repeat.to_string())
            .set("color_format", self.color_format.to_index().to_string())
            .set("lossless", self.lossless.to_string())
            .set("quality", self.quality.to_string())
            .set("method", self.method.to_string())
            .set("workers", self.workers.to_string());
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
        assert_eq!(config.workers, default.workers);
    }

    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let saved = Config {
            repeat: 3,
            color_format: ColorFormat::Rgba32,
            lossless: true,
            quality: 100.0,
            method: 6,
            workers: max_workers(),
        };

        let mut ini = Ini::new();
        saved.save_to(&mut ini);
        let loaded = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(loaded.repeat, saved.repeat);
        assert!(loaded.color_format == saved.color_format);
        assert_eq!(loaded.lossless, saved.lossless);
        assert_eq!(loaded.quality, saved.quality);
        assert_eq!(loaded.method, saved.method);
        assert_eq!(loaded.workers, saved.workers);
    }

    /// 設定ファイルの中身をそのまま読み、セクション名と項目名まで含めて確かめる
    #[test]
    fn a_config_file_written_before_still_loads() {
        let text = "\
[Config]
repeat=5
color_format=1
lossless=true
quality=90
method=3
";
        let ini = Ini::load_from_str(text).unwrap();
        let config = Config::load_from(ini.section(Some(Config::SECTION)));

        assert_eq!(config.repeat, 5);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert!(config.lossless);
        assert_eq!(config.quality, 90.0);
        assert_eq!(config.method, 3);
    }

    /// 値域の外の品質とメソッドは、エンコーダが受け取れる範囲へ収まる
    #[test]
    fn out_of_range_quality_and_method_are_clamped() {
        let config = load(&[("quality", "1000"), ("method", "99")]);

        assert_eq!(config.quality, 100.0);
        assert_eq!(config.method, 6);
    }

    /// 値域の外のワーカー数は、走らせる機械の並列度の内側へ収まる
    ///
    /// 別の機械で書いた ini をそのまま読んでも、この機械で意味のある数になる。
    #[test]
    fn out_of_range_workers_are_clamped() {
        let over = (max_workers() + 1).to_string();

        assert_eq!(load(&[("workers", "0")]).workers, 1);
        assert_eq!(load(&[("workers", &over)]).workers, max_workers());
    }

    /// 読めない値の項目だけが既定値へ落ちる
    #[test]
    fn an_unreadable_value_falls_back_on_its_own() {
        let config = load(&[("repeat", "many"), ("lossless", "true")]);
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
