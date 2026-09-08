//! プラグインDLL自身のパス取得

use std::path::PathBuf;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HMODULE, MAX_PATH};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::core::PCWSTR;

/// このプラグインDLL自身のフルパスを取得する
///
/// 本クレートはプラグインDLLに静的リンクされるため、この関数自身のアドレスから
/// 取得したモジュールはプラグインDLLになる。パスは走っている間変わらないので
/// 初回の結果を返し続ける。
pub fn dll_path() -> Result<PathBuf, String> {
    static PATH: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    PATH.get_or_init(query_dll_path).clone()
}

/// ホストが読み込んだモジュールへ問い合わせる
fn query_dll_path() -> Result<PathBuf, String> {
    let (buffer, len) = unsafe {
        let mut hmodule: HMODULE = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(dll_path as *const () as *const u16),
            &mut hmodule as *mut HMODULE,
        )
        .map_err(|e| format!("GetModuleHandleExW failed: {}", e))?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let len = GetModuleFileNameW(Some(hmodule), &mut buffer);

        (buffer, len)
    };

    let len = written_len(len, buffer.len())?;
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}

/// 書き込まれた文字数のうち、パスとして読める長さ
///
/// バッファに収まらなかったときは切り詰められた文字列と満杯の長さが残るため、
/// 満杯は失敗になる。
fn written_len(len: u32, capacity: usize) -> Result<usize, String> {
    match len as usize {
        0 => Err("GetModuleFileNameW failed".to_string()),
        len if len == capacity => Err(format!("プラグインのパスが{}文字に収まりません", capacity)),
        len => Ok(len),
    }
}

/// このプラグインDLL自身のディレクトリを取得する
pub fn dll_dir() -> Result<PathBuf, String> {
    let path = dll_path()?;
    let dir = path
        .parent()
        .ok_or("プラグインのディレクトリが取得できません")?;
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 静的リンクした先のモジュール (テストでは実行ファイル自身) を指す
    #[test]
    fn the_path_points_at_the_module_that_linked_this_crate() {
        assert_eq!(dll_path().unwrap(), std::env::current_exe().unwrap());
    }

    /// バッファが満杯なら切り詰められているので、パスとして読まない
    #[test]
    fn a_length_that_fills_the_buffer_is_a_failure() {
        assert_eq!(written_len(259, 260), Ok(259));
        assert_eq!(
            written_len(260, 260),
            Err("プラグインのパスが260文字に収まりません".to_string())
        );
        assert_eq!(
            written_len(0, 260),
            Err("GetModuleFileNameW failed".to_string())
        );
    }

    /// 何度呼んでも同じ結果を返す
    #[test]
    fn the_path_stays_the_same_across_calls() {
        assert_eq!(dll_path(), dll_path());
        assert_eq!(dll_dir().unwrap(), dll_path().unwrap().parent().unwrap());
    }
}
