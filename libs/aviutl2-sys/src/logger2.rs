//! `logger2.h` — ログ出力機能 ヘッダーファイル for AviUtl ExEdit2 の忠実な翻訳

use crate::LPCWSTR;

/// ログ出力ハンドル
///
/// ログ出力は1024文字で制限される。
#[repr(C)]
pub struct LOG_HANDLE {
    /// プラグイン用のログを出力します
    /// - handle: ログ出力ハンドル
    /// - message: ログメッセージ
    pub log: Option<unsafe extern "C" fn(handle: *mut LOG_HANDLE, message: LPCWSTR)>,

    /// infoレベルのログを出力します
    /// - handle: ログ出力ハンドル
    /// - message: ログメッセージ
    pub info: Option<unsafe extern "C" fn(handle: *mut LOG_HANDLE, message: LPCWSTR)>,

    /// warnレベルのログを出力します
    /// - handle: ログ出力ハンドル
    /// - message: ログメッセージ
    pub warn: Option<unsafe extern "C" fn(handle: *mut LOG_HANDLE, message: LPCWSTR)>,

    /// errorレベルのログを出力します
    /// - handle: ログ出力ハンドル
    /// - message: ログメッセージ
    pub error: Option<unsafe extern "C" fn(handle: *mut LOG_HANDLE, message: LPCWSTR)>,

    /// verboseレベルのログを出力します
    /// - handle: ログ出力ハンドル
    /// - message: ログメッセージ
    pub verbose: Option<unsafe extern "C" fn(handle: *mut LOG_HANDLE, message: LPCWSTR)>,
}
