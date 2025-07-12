use crate::{Control, Result};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

struct LabelInner {
    hwnd: Option<HWND>,
    id: i32,
    position: (i32, i32),
    size: (i32, i32),
    label: String,
}

#[derive(Clone)]
pub struct Label {
    inner: Rc<RefCell<LabelInner>>,
}

impl Label {
    pub fn new(id: i32, position: (i32, i32), size: (i32, i32), label: &str) -> Self {
        Self {
            inner: Rc::new(RefCell::new(LabelInner {
                hwnd: None,
                id,
                position,
                size,
                label: label.to_string(),
            })),
        }
    }

    pub fn set_label(&self, text: &str) {
        let mut inner = self.inner.borrow_mut();
        inner.label = text.to_string();
        if let Some(hwnd) = inner.hwnd {
            let wide_text: Vec<u16> = text.encode_utf16().chain([0]).collect();
            unsafe {
                let _ = SetWindowTextW(hwnd, PCWSTR::from_raw(wide_text.as_ptr()));
            }
        }
    }
}

impl Control for Label {
    fn create(&mut self, parent: HWND) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let mut inner = self.inner.borrow_mut();
            let wide_text: Vec<u16> = inner.label.encode_utf16().chain([0]).collect();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                PCWSTR::from_raw(wide_text.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                inner.position.0,
                inner.position.1,
                inner.size.0,
                inner.size.1,
                Some(parent),
                Some(HMENU(inner.id as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            inner.hwnd = Some(hwnd);
            Ok(hwnd)
        }
    }

    fn handle_message(&mut self, _msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        None
    }

    fn get_id(&self) -> i32 {
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
