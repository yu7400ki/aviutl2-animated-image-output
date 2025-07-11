use crate::types::LPCWSTR;

pub fn to_wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_to_string(ptr: LPCWSTR) -> String {
    unsafe {
        let len = (0..).take_while(|&i| *ptr.offset(i) != 0).count();
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}
