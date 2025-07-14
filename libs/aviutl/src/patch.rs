use crate::output2::OutputInfo;
use std::ffi::c_void;
use windows::Win32::System::Memory::*;

pub unsafe fn apply_rgba_patch(info: &OutputInfo) -> std::result::Result<u32, &str> {
    let base_addr = match info.func_get_video {
        Some(func) => func as usize,
        None => return Err("func_get_videoが設定されていません"),
    };
    let target = base_addr + 0xA0;
    let target_ptr = target as *mut u32;
    let mut old_protect = PAGE_PROTECTION_FLAGS::default();

    // メモリ保護を変更
    unsafe {
        if VirtualProtect(
            target_ptr as *mut c_void,
            4,
            PAGE_EXECUTE_READWRITE,
            &mut old_protect,
        )
        .is_err()
        {
            return Err("メモリ保護の変更に失敗しました");
        }

        let current_value = *target_ptr;
        if current_value != 0x000058BF {
            VirtualProtect(target_ptr as *mut c_void, 4, old_protect, &mut old_protect).ok();
            return Err("対象のメモリ値が期待値と一致しません");
        }

        *target_ptr = 0x000057BF;
    }

    Ok(old_protect.0)
}

pub unsafe fn restore_rgba_patch(info: &OutputInfo, old_protect: u32) {
    let base_addr = match info.func_get_video {
        Some(func) => func as usize,
        None => return,
    };
    let target = base_addr + 0xA0;
    let target_ptr = target as *mut u32;

    unsafe {
        *target_ptr = 0x000058BF;
        let old_protect_flags = PAGE_PROTECTION_FLAGS(old_protect);
        let _ = VirtualProtect(
            target_ptr as *mut c_void,
            4,
            old_protect_flags,
            &mut PAGE_PROTECTION_FLAGS::default(),
        );
    }
}
