//! プラグイン設定の読み書きと、設定値の既定・上限

use ini::{Ini, Properties};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread::available_parallelism;

/// 設定が採れるループ回数の上限
///
/// ダイアログの数値欄が扱える上限。
pub const MAX_REPEAT: u32 = i32::MAX as u32;

/// 設定が採れるスレッド数の上限
///
/// 走らせる機械の論理CPU数。読めなければ1を返す。
pub fn max_threads() -> usize {
    available_parallelism().map_or(1, NonZeroUsize::get)
}

/// 既定で使うスレッド数
///
/// 上限の半分。上限が1の機械では1になる。
pub fn default_threads() -> usize {
    (max_threads() / 2).max(1)
}

/// スレッド数を1以上のワーカー数にする
pub fn workers(threads: usize) -> NonZeroUsize {
    NonZeroUsize::new(threads).unwrap_or(NonZeroUsize::MIN)
}

/// セクションからキーを読み、`FromStr` で解釈する
///
/// セクションが無い・キーが無い・値が解釈できない場合は `default` を返す。
pub fn read<T: FromStr>(section: Option<&Properties>, key: &str, default: T) -> T {
    section
        .and_then(|s| s.get(key))
        .and_then(|s| s.parse::<T>().ok())
        .unwrap_or(default)
}

/// プラグイン設定のini永続化
///
/// `load_from` / `save_to` でフィールドの読み書きだけを実装すれば、
/// ファイルパス解決・読み込み失敗時のフォールバックは既定実装が行う。
///
/// ```ignore
/// impl IniConfig for Config {
///     const FILE_NAME: &'static str = concat!(env!("CARGO_PKG_NAME"), ".ini");
///
///     fn load_from(section: Option<&ini::Properties>) -> Self { ... }
///     fn save_to(&self, ini: &mut ini::Ini) { ... }
/// }
/// ```
pub trait IniConfig: Default + Sized {
    /// iniファイル名 (プラグイン側で `concat!(env!("CARGO_PKG_NAME"), ".ini")` を指定する)
    const FILE_NAME: &'static str;
    /// セクション名
    const SECTION: &'static str = "Config";

    /// セクションから設定を読み込む (ファイルやセクションが無い場合は `None` が渡される)
    fn load_from(section: Option<&ini::Properties>) -> Self;

    /// iniへ設定を書き込む (`ini.with_section(Some(Self::SECTION)).set(...)` を行う)
    fn save_to(&self, ini: &mut Ini);

    /// 設定ファイルのパスを取得する
    fn config_path() -> Result<PathBuf, String> {
        Ok(crate::module::dll_dir()?.join(Self::FILE_NAME))
    }

    /// 設定を読み込む (失敗時は `Self::default()`)
    fn load() -> Self {
        let config_path = match Self::config_path() {
            Ok(path) => path,
            Err(_) => return Self::default(),
        };

        if !config_path.exists() {
            return Self::default();
        }

        let ini = match Ini::load_from_file(&config_path) {
            Ok(ini) => ini,
            Err(_) => return Self::default(),
        };

        Self::load_from(ini.section(Some(Self::SECTION)))
    }

    /// 設定を保存する
    fn save(&self) -> Result<(), String> {
        let config_path = Self::config_path()?;
        let mut ini = Ini::new();
        self.save_to(&mut ini);
        ini.write_to_file(&config_path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties(entries: &[(&str, &str)]) -> Properties {
        let mut properties = Properties::new();
        for (key, value) in entries {
            properties.insert(*key, *value);
        }
        properties
    }

    #[test]
    fn a_zero_worker_count_becomes_one() {
        assert_eq!(workers(0).get(), 1);
    }

    #[test]
    fn threads_at_or_above_one_pass_through_unchanged() {
        for threads in [1, 2, 7] {
            assert_eq!(workers(threads).get(), threads);
        }
    }

    #[test]
    fn missing_section_falls_back_to_default() {
        let value: u32 = read(None, "repeat", 5);
        assert_eq!(value, 5);
    }

    #[test]
    fn missing_key_falls_back_to_default() {
        let properties = properties(&[("other", "1")]);
        let value: u32 = read(Some(&properties), "repeat", 5);
        assert_eq!(value, 5);
    }

    #[test]
    fn unparseable_value_falls_back_to_default() {
        let properties = properties(&[("repeat", "many")]);
        let value: u32 = read(Some(&properties), "repeat", 5);
        assert_eq!(value, 5);
    }

    #[test]
    fn parseable_value_is_read() {
        let properties = properties(&[("repeat", "3")]);
        let value: u32 = read(Some(&properties), "repeat", 5);
        assert_eq!(value, 3);
    }

    /// 既定のスレッド数は上限の内側で控えめに採る
    ///
    /// 上限が1の機械では1つしか採れないので、そこだけ上限と一致する。
    #[test]
    fn default_threads_stay_inside_the_ceiling() {
        let default = default_threads();
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
}
