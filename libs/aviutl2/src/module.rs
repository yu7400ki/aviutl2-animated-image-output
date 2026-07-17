//! プラグインDLL自身のパス取得

use std::path::PathBuf;
use windows::Win32::Foundation::{HMODULE, MAX_PATH};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleFileNameW, GetModuleHandleExW,
};
use windows::core::PCWSTR;

/// このプラグインDLL自身のフルパスを取得する
///
/// 本クレートはプラグインDLLに静的リンクされるため、この関数自身のアドレスから
/// 取得したモジュールはプラグインDLLになる。
pub fn dll_path() -> Result<PathBuf, String> {
    let (buffer, len) = unsafe {
        let mut hmodule: HMODULE = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCWSTR(dll_path as *const () as *const u16),
            &mut hmodule as *mut HMODULE,
        )
        .map_err(|e| format!("GetModuleHandleExW failed: {}", e))?;

        let mut buffer = [0u16; MAX_PATH as usize];
        let len = GetModuleFileNameW(Some(hmodule), &mut buffer);

        (buffer, len)
    };

    if len > 0 {
        let dll_path = String::from_utf16_lossy(&buffer[..len as usize]);
        Ok(PathBuf::from(dll_path))
    } else {
        Err("GetModuleFileNameW failed".to_string())
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
