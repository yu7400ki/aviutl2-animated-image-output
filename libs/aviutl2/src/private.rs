//! `register_output_plugin!` マクロの展開先から使用される内部実装
//!
//! このモジュールの内容は公開APIではない。

use crate::output::{ConfigOutcome, OutputInfo, OutputPlugin};
use crate::{logger, metrics, sys};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use win32_ui::MessageBox;
use windows::Win32::Foundation::{HINSTANCE, HWND};

pub use std::ffi::c_void;
pub use windows::core::BOOL;

pub const TRUE: BOOL = BOOL(1);

struct TableRepr {
    // OUTPUT_PLUGIN_TABLE内のポインタが指すUTF-16バッファ(NUL終端済み)。
    // Vecのヒープ領域はTableReprがmoveされても安定なのでポインタは有効なまま。
    _name: Vec<u16>,
    _filefilter: Vec<u16>,
    _information: Vec<u16>,
    table: sys::OUTPUT_PLUGIN_TABLE,
}

/// プラグインテーブルとその文字列バッファの所有者
///
/// マクロが `static` として展開し、`GetOutputPluginTable` 初回呼び出し時に構築する。
pub struct TableStorage {
    cell: OnceLock<TableRepr>,
}

// Safety: tableが含む生ポインタは同じTableReprが所有するバッファのみを指し、
// 初期化後は一切変更されない。
unsafe impl Sync for TableStorage {}

impl TableStorage {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    /// `T::info()` からテーブルを構築(初回のみ)してポインタを返す
    pub fn get_or_init<T: OutputPlugin>(&'static self) -> *mut sys::OUTPUT_PLUGIN_TABLE {
        let repr = self.cell.get_or_init(|| {
            let info = T::info();

            let name: Vec<u16> = encode_nul_terminated(&info.name);
            let filefilter: Vec<u16> = info.file_filter.to_wide();
            let information: Vec<u16> = encode_nul_terminated(&info.information);

            let table = sys::OUTPUT_PLUGIN_TABLE {
                flag: info.flags.as_raw(),
                name: name.as_ptr(),
                filefilter: filefilter.as_ptr(),
                information: information.as_ptr(),
                func_output: Some(
                    output_shim::<T> as unsafe extern "C" fn(*mut sys::OUTPUT_INFO) -> bool,
                ),
                func_config: if T::HAS_CONFIG_DIALOG {
                    Some(config_shim::<T> as unsafe extern "C" fn(HWND, HINSTANCE) -> bool)
                } else {
                    None
                },
                func_get_config_text: if T::HAS_CONFIG_TEXT {
                    Some(config_text_shim::<T> as unsafe extern "C" fn() -> sys::LPCWSTR)
                } else {
                    None
                },
                func_load_project_config: None,
                func_save_project_config: None,
            };

            TableRepr {
                _name: name,
                _filefilter: filefilter,
                _information: information,
                table,
            }
        });

        &repr.table as *const sys::OUTPUT_PLUGIN_TABLE as *mut sys::OUTPUT_PLUGIN_TABLE
    }
}

fn encode_nul_terminated(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 設定ダイアログが設定を返せなかったときに報せる文言
const CONFIG_FAILED_MESSAGE: &str = "設定の取得に失敗しました。";

/// パニックのペイロードから表示可能なメッセージを取り出す
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("不明なパニック")
}

/// 出力ファイルのサイズを整形する。取得できない場合は `"不明"`。
fn output_size(path: &Path) -> String {
    std::fs::metadata(path)
        .map(|m| metrics::format_bytes(m.len()))
        .unwrap_or_else(|_| "不明".to_string())
}

// catch_unwindはrelease(panic = "abort")では実質no-opだが、
// devビルドでFFI境界を越えるunwindを防ぐ。

extern "C" fn output_shim<T: OutputPlugin>(oip: *mut sys::OUTPUT_INFO) -> bool {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(info) = (unsafe { OutputInfo::from_raw(oip) }) else {
            logger::error("出力情報の取得に失敗しました");
            return false;
        };
        logger::info("出力を開始します");
        let start = Instant::now();
        match T::output(&info) {
            Ok(()) => {
                logger::info(&format!(
                    "出力完了 {}フレーム, {}, {:.2}秒",
                    info.raw_num_frames(),
                    output_size(&info.savefile()),
                    start.elapsed().as_secs_f64()
                ));
                true
            }
            Err(message) => {
                logger::error(&message);
                MessageBox::error(None, &message, "エラー");
                false
            }
        }
    }));

    result.unwrap_or_else(|payload| {
        logger::error(panic_message(&*payload));
        false
    })
}

extern "C" fn config_shim<T: OutputPlugin>(hwnd: HWND, dll_hinst: HINSTANCE) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let outcome = T::config(hwnd, dll_hinst);
        match &outcome {
            ConfigOutcome::Failed => {
                logger::error(CONFIG_FAILED_MESSAGE);
                MessageBox::error(Some(hwnd), CONFIG_FAILED_MESSAGE, "エラー");
            }
            ConfigOutcome::NotSaved(message) => {
                logger::warn(message);
                MessageBox::warning(Some(hwnd), message, "警告");
            }
            ConfigOutcome::Saved | ConfigOutcome::Cancelled => {}
        }
        outcome.accepted()
    }))
    .unwrap_or(false)
}

extern "C" fn config_text_shim<T: OutputPlugin>() -> sys::LPCWSTR {
    // 「次に関数が呼ばれるまで内容を有効にしておく」(output2.h) ため
    // staticに保持する。ジェネリック関数内のstaticは全単相化で共有されるが、
    // マクロ契約(1つのcdylibに1プラグイン)により実質1つ。
    static TEXT: Mutex<Vec<u16>> = Mutex::new(Vec::new());

    let text = catch_unwind(|| T::config_text()).unwrap_or_default();
    let mut guard = TEXT.lock().unwrap_or_else(|e| e.into_inner());
    *guard = encode_nul_terminated(&text);
    guard.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IniConfig;
    use crate::ini::{Ini, Properties};
    use crate::output::{FileFilter, PluginFlags, PluginInfo};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static INFO_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Default)]
    struct CountingConfig;

    impl IniConfig for CountingConfig {
        const FILE_NAME: &'static str = "private-test.ini";

        fn load_from(_section: Option<&Properties>) -> Self {
            CountingConfig
        }

        fn save_to(&self, _ini: &mut Ini) {}
    }

    struct CountingPlugin;

    impl OutputPlugin for CountingPlugin {
        type Config = CountingConfig;

        const FORMAT_NAME: &'static str = "計数";

        fn info() -> PluginInfo {
            INFO_CALLS.fetch_add(1, Ordering::Relaxed);
            PluginInfo {
                flags: PluginFlags::VIDEO,
                name: "計数出力プラグイン".into(),
                file_filter: FileFilter::new().add("All Files (*)", "*"),
                information: "計数出力プラグイン v0.1.0".into(),
            }
        }

        fn encode(_info: &OutputInfo, _config: &CountingConfig) -> Result<(), String> {
            Ok(())
        }
    }

    /// テーブルは初回だけ構築し、`info()` を評価し直さない
    #[test]
    fn info_is_evaluated_once() {
        static TABLE: TableStorage = TableStorage::new();

        assert!(!TABLE.get_or_init::<CountingPlugin>().is_null());
        assert!(!TABLE.get_or_init::<CountingPlugin>().is_null());

        assert_eq!(INFO_CALLS.load(Ordering::Relaxed), 1);
    }
}
