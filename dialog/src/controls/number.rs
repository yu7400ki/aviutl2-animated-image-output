use crate::{Control, ControlId, Result};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use widestring::U16CString;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::{Controls::*, WindowsAndMessaging::*};
use windows::core::*;

struct NumberInner {
    hwnd: Option<HWND>,
    id: ControlId,
    position: (i32, i32),
    size: (i32, i32),
    value: i32,
    range: Option<(i32, i32)>,
}

#[derive(Clone)]
pub struct Number {
    inner: Rc<RefCell<NumberInner>>,
}

impl Number {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(NumberInner {
                hwnd: None,
                id: ControlId::new(),
                position: (0, 0),
                size: (100, 25),
                value: 0,
                range: None,
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

    pub fn value(self, val: i32) -> Self {
        self.inner.borrow_mut().value = val;
        self
    }

    pub fn range(self, min: i32, max: i32) -> Self {
        self.inner.borrow_mut().range = Some((min, max));
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

    pub fn get_value<T: std::str::FromStr>(&self) -> std::result::Result<T, T::Err> {
        self.get_text().parse::<T>()
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

    pub fn set_value<T: ToString>(&self, value: T) {
        self.set_text(&value.to_string());
    }
}

impl Control for Number {
    fn create(&mut self, parent: HWND) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let mut inner = self.inner.borrow_mut();

            let hwnd_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                inner.position.0,
                inner.position.1,
                inner.size.0,
                inner.size.1,
                Some(parent),
                Some(HMENU(inner.id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            let hwnd_updown = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                UPDOWN_CLASS,
                PCWSTR::null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WINDOW_STYLE(UDS_ALIGNRIGHT as u32)
                    | WINDOW_STYLE(UDS_SETBUDDYINT as u32)
                    | WINDOW_STYLE(UDS_ARROWKEYS as u32),
                inner.position.0,
                inner.position.1,
                0,
                inner.size.1,
                Some(parent),
                Some(HMENU((inner.id.as_raw() + 1000) as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            let initial_wide = U16CString::from_str(&inner.value.to_string()).unwrap_or_default();
            SetWindowTextW(hwnd_edit, PCWSTR(initial_wide.as_ptr())).ok();

            // UpDownコントロールのバディをEditコントロールに設定
            SendMessageW(
                hwnd_updown,
                UDM_SETBUDDY,
                Some(WPARAM(hwnd_edit.0 as usize)),
                Some(LPARAM(0)),
            );

            if let Some((min, max)) = inner.range {
                SendMessageW(
                    hwnd_updown,
                    UDM_SETRANGE32,
                    Some(WPARAM(min as usize)),
                    Some(LPARAM(max as isize)),
                );
            }

            inner.hwnd = Some(hwnd_edit);

            Ok(hwnd_edit)
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
