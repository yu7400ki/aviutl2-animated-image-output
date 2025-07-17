use crate::{DialogError, Result};
use std::sync::OnceLock;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

static DEFAULT_FONT: OnceLock<isize> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct FontManager {
    font: HFONT,
}

impl FontManager {
    pub fn new() -> Result<Self> {
        let font = Self::create_default_font()?;
        Ok(FontManager { font })
    }

    pub fn with_font(font: HFONT) -> Self {
        FontManager { font }
    }

    fn create_default_font() -> Result<HFONT> {
        unsafe {
            let font = CreateFontW(
                -12,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                SHIFTJIS_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Meiryo UI"),
            );

            if font.is_invalid() {
                Err(DialogError::Win32Error(windows::core::Error::from_win32()))
            } else {
                Ok(font)
            }
        }
    }

    pub fn get_font(&self) -> HFONT {
        self.font
    }

    pub fn get_default_font() -> HFONT {
        let font_handle = *DEFAULT_FONT.get_or_init(|| {
            Self::create_default_font()
                .map(|f| f.0 as isize)
                .unwrap_or_else(|_| 0)
        });
        HFONT(font_handle as *mut std::ffi::c_void)
    }

    pub fn apply_to_window(&self, hwnd: HWND) {
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }

    pub fn get_text_size(&self, text: &str) -> Result<(i32, i32)> {
        crate::get_text_size(text, Some(self.font))
    }
}

impl Default for FontManager {
    fn default() -> Self {
        FontManager {
            font: Self::get_default_font(),
        }
    }
}

impl Drop for FontManager {
    fn drop(&mut self) {
        if self.font.0 != Self::get_default_font().0 && !self.font.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.font.into());
            }
        }
    }
}
