//! ログ出力機能の安全なAPI (`logger2.h`)
//!
//! プラグインが `InitializeLogger` を外部公開すると、`InitializePlugin` より先に
//! ハンドルが渡される。このモジュールはそのハンドルをグローバルに保持し、
//! [`log`]/[`info`]/[`warn`]/[`error`]/[`verbose`] で安全に呼び出せるようにする。
//! ハンドル未設定時は全てno-op。
//!
//! 通常は [`crate::register_logger!`] マクロで `InitializeLogger` エクスポートを
//! 生成し、[`init`] を直接呼ぶ必要はない。

use crate::sys::{self, LOG_HANDLE};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

static HANDLE: AtomicPtr<LOG_HANDLE> = AtomicPtr::new(ptr::null_mut());

/// ログ出力機能を初期化する (`InitializeLogger` から呼ぶ)
///
/// 通常は [`crate::register_logger!`] が生成するエクスポート経由で呼ばれる。
///
/// # Safety
/// `logger` はホストから渡された有効な `LOG_HANDLE` へのポインタであるか、
/// nullであること。ホストはプラグインのライフタイム中ポインタを有効に保つ。
pub unsafe fn init(logger: *mut LOG_HANDLE) {
    HANDLE.store(logger, Ordering::Release);
}

/// ログ出力の最大文字数 (`logger2.h` の制限、UTF-16コードユニット数)
const MAX_LEN: usize = 1024;

/// UTF-16へエンコードし、1024コードユニットを超える分を切り詰めてNUL終端する
///
/// 切り詰め位置がサロゲートペアの先頭(上位サロゲート)に当たる場合は
/// そのユニットごと落とし、不正なペア分断を避ける。
fn encode_truncated(message: &str) -> Vec<u16> {
    let mut buf: Vec<u16> = message.encode_utf16().collect();
    if buf.len() > MAX_LEN {
        buf.truncate(MAX_LEN);
        if matches!(buf.last(), Some(&unit) if (0xD800..=0xDBFF).contains(&unit)) {
            buf.pop();
        }
    }
    buf.push(0);
    buf
}

fn emit(
    selector: impl FnOnce(&LOG_HANDLE) -> Option<unsafe extern "C" fn(*mut LOG_HANDLE, sys::LPCWSTR)>,
    message: &str,
) {
    let handle = HANDLE.load(Ordering::Acquire);
    let Some(handle_ref) = (unsafe { handle.as_ref() }) else {
        return;
    };
    let Some(f) = selector(handle_ref) else {
        return;
    };
    let wide = encode_truncated(message);
    unsafe { f(handle, wide.as_ptr()) };
}

/// プラグイン用のログを出力します
pub fn log(message: &str) {
    emit(|h| h.log, message);
}

/// infoレベルのログを出力します
pub fn info(message: &str) {
    emit(|h| h.info, message);
}

/// warnレベルのログを出力します
pub fn warn(message: &str) {
    emit(|h| h.warn, message);
}

/// errorレベルのログを出力します
pub fn error(message: &str) {
    emit(|h| h.error, message);
}

/// verboseレベルのログを出力します
pub fn verbose(message: &str) {
    emit(|h| h.verbose, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use widestring::U16CStr;

    // HANDLE はプロセスグローバルなので、テスト間の並列実行による競合を防ぐ。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    static CAPTURED: Mutex<Option<Vec<u16>>> = Mutex::new(None);

    unsafe extern "C" fn recording_fn(_handle: *mut LOG_HANDLE, message: sys::LPCWSTR) {
        let s = unsafe { U16CStr::from_ptr_str(message) };
        *CAPTURED.lock().unwrap() = Some(s.as_slice().to_vec());
    }

    #[test]
    fn noop_when_handle_unset() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        HANDLE.store(ptr::null_mut(), Ordering::Release);
        *CAPTURED.lock().unwrap() = None;

        log("test");
        info("test");
        warn("test");
        error("test");
        verbose("test");

        assert!(CAPTURED.lock().unwrap().is_none());
    }

    #[test]
    fn calls_handle_fn_when_set() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *CAPTURED.lock().unwrap() = None;

        let mut handle = LOG_HANDLE {
            log: Some(recording_fn),
            info: Some(recording_fn),
            warn: Some(recording_fn),
            error: Some(recording_fn),
            verbose: None,
        };
        unsafe { init(&mut handle as *mut LOG_HANDLE) };

        info("こんにちは");
        assert_eq!(
            CAPTURED.lock().unwrap().as_deref(),
            Some("こんにちは".encode_utf16().collect::<Vec<u16>>().as_slice())
        );

        // verbose未設定はno-op (前回のCAPTUREDが残ったままなことを確認)
        *CAPTURED.lock().unwrap() = None;
        verbose("skip");
        assert!(CAPTURED.lock().unwrap().is_none());

        HANDLE.store(ptr::null_mut(), Ordering::Release);
    }

    #[test]
    fn truncates_at_max_len() {
        let long = "a".repeat(MAX_LEN + 100);
        let encoded = encode_truncated(&long);
        assert_eq!(encoded.len(), MAX_LEN + 1); // +1 はNUL終端
        assert_eq!(encoded[MAX_LEN], 0);
        assert!(encoded[..MAX_LEN].iter().all(|&u| u == b'a' as u16));
    }

    #[test]
    fn truncation_does_not_split_surrogate_pair() {
        // 'a' を1023個 + サロゲートペアが必要な文字(絵文字) + 埋め文字
        let mut s = "a".repeat(MAX_LEN - 1);
        s.push('😀'); // U+1F600: 上位/下位サロゲートの2ユニット
        s.push_str(&"b".repeat(50));

        let encoded = encode_truncated(&s);
        // 境界(index MAX_LEN-1)は絵文字の上位サロゲートになるはずだが、
        // ペア分断を避けるため落とされ、1023ユニット+NULになる。
        assert_eq!(encoded.len(), MAX_LEN); // MAX_LEN-1文字 + NUL
        assert!((0xD800..=0xDBFF).contains(&0xD83D_u16)); // 前提確認: 😀の上位サロゲート
        assert!(
            !encoded[..encoded.len() - 1]
                .iter()
                .any(|&u| (0xD800..=0xDFFF).contains(&u))
        );
        assert_eq!(*encoded.last().unwrap(), 0);
    }
}
