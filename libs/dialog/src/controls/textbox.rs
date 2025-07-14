use crate::{Control, ControlId, Result};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use widestring::U16CString;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

struct TextBoxInner {
    hwnd: Option<HWND>,
    id: ControlId,
    position: (i32, i32),
    size: (i32, i32),
    text: String,
}

#[derive(Clone)]
pub struct TextBox {
    inner: Rc<RefCell<TextBoxInner>>,
}

impl TextBox {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(TextBoxInner {
                hwnd: None,
                id: ControlId::new(),
                position: (0, 0),
                size: (150, 25),
                text: String::new(),
            })),
        }
    }

    pub fn position(self, x: i32, y: i32) -> Self {
        self.inner.borrow_mut().position = (x, y);
        self
    }

    pub fn size(self, width: i32, height: i32) -> Self {
        self.inner.borrow_mut().size = (width, height);
        self
    }

    pub fn text(self, txt: &str) -> Self {
        self.inner.borrow_mut().text = txt.to_string();
        self
    }

    pub fn get_text(&self) -> String {
        let inner = self.inner.borrow();
        match inner.hwnd {
            Some(hwnd) => unsafe {
                let mut buffer = [0u16; 256];
                let len = GetWindowTextW(hwnd, &mut buffer);
                if len > 0 {
                    String::from_utf16_lossy(&buffer[..len as usize])
                } else {
                    String::new()
                }
            },
            None => String::new(),
        }
    }

    pub fn set_text(&self, text: &str) {
        let inner = self.inner.borrow();
        if let Some(hwnd) = inner.hwnd {
            let wide_text = U16CString::from_str(text).unwrap_or_default();
            unsafe {
                SetWindowTextW(hwnd, PCWSTR(wide_text.as_ptr())).ok();
            }
        }
    }
}

impl Control for TextBox {
    fn create(&mut self, parent: HWND) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let mut inner = self.inner.borrow_mut();

            let hwnd = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP,
                inner.position.0,
                inner.position.1,
                inner.size.0,
                inner.size.1,
                Some(parent),
                Some(HMENU(inner.id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            let initial_wide = U16CString::from_str(&inner.text).unwrap_or_default();
            SetWindowTextW(hwnd, PCWSTR(initial_wide.as_ptr())).ok();

            inner.hwnd = Some(hwnd);

            Ok(hwnd)
        }
    }

    fn handle_message(&mut self, _msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        None
    }

    fn get_id(&self) -> ControlId {
        self.inner.borrow().id
    }

    fn get_hwnd(&self) -> Option<HWND> {
        self.inner.borrow().hwnd
    }

    fn set_font(&self, font: HFONT) {
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
    }
}
