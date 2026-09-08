//! `register_logger!` が生成する `InitializeLogger` のFFIエクスポートを検証する

use aviutl2::sys::LOG_HANDLE;
use aviutl2::{logger, register_logger};
use std::sync::Mutex;
use widestring::U16CStr;

register_logger!();

static CAPTURED: Mutex<Option<Vec<u16>>> = Mutex::new(None);

unsafe extern "C" fn recording_fn(_handle: *mut LOG_HANDLE, message: aviutl2::sys::LPCWSTR) {
    let s = unsafe { U16CStr::from_ptr_str(message) };
    *CAPTURED.lock().unwrap() = Some(s.as_slice().to_vec());
}

#[test]
fn initialize_logger_export_wires_up_logger_module() {
    *CAPTURED.lock().unwrap() = None;

    let mut handle = LOG_HANDLE {
        log: None,
        info: Some(recording_fn),
        warn: None,
        error: None,
        verbose: None,
    };

    // マクロが生成した `InitializeLogger` そのものを呼ぶ (catch_unwindラップ込み)
    unsafe {
        InitializeLogger(&mut handle as *mut LOG_HANDLE);
    }

    logger::info("hello");
    assert_eq!(
        CAPTURED.lock().unwrap().as_deref(),
        Some("hello".encode_utf16().collect::<Vec<u16>>().as_slice())
    );

    // 未設定レベル(warn)は引き続きno-op
    *CAPTURED.lock().unwrap() = None;
    logger::warn("skip");
    assert!(CAPTURED.lock().unwrap().is_none());
}
