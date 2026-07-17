//! `register_output_plugin!` マクロの展開先から使用される内部実装
//!
//! このモジュールの内容は公開APIではない。

use crate::output::{OutputInfo, OutputPlugin};
use crate::sys;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

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

fn show_error_message_box(message: &str) {
    let message = HSTRING::from(message);
    let title = HSTRING::from("エラー");
    unsafe {
        MessageBoxW(None, &message, &title, MB_OK | MB_ICONERROR);
    }
}

// catch_unwindはrelease(panic = "abort")では実質no-opだが、
// devビルドでFFI境界を越えるunwindを防ぐ。

extern "C" fn output_shim<T: OutputPlugin>(oip: *mut sys::OUTPUT_INFO) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(info) = (unsafe { OutputInfo::from_raw(oip) }) else {
            return false;
        };
        match T::output(&info) {
            Ok(()) => true,
            Err(e) => {
                show_error_message_box(&e.to_string());
                false
            }
        }
    }))
    .unwrap_or(false)
}

extern "C" fn config_shim<T: OutputPlugin>(hwnd: HWND, dll_hinst: HINSTANCE) -> bool {
    catch_unwind(AssertUnwindSafe(|| T::config(hwnd, dll_hinst))).unwrap_or(false)
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
