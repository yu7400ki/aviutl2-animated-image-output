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

struct CheckBoxInner {
    hwnd: Option<HWND>,
    id: ControlId,
    position: (i32, i32),
    size: (i32, i32),
    label: String,
    checked: bool,
    change_handler: Option<Box<dyn FnMut(bool) -> Result<()>>>,
}

#[derive(Clone)]
pub struct CheckBox {
    inner: Rc<RefCell<CheckBoxInner>>,
}

impl CheckBox {
    pub fn new(label: &str) -> Self {
        Self {
            inner: Rc::new(RefCell::new(CheckBoxInner {
                hwnd: None,
                id: ControlId::new(),
                position: (0, 0),
                size: (120, 20),
                label: label.to_string(),
                checked: false,
                change_handler: None,
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

    pub fn checked(self, checked: bool) -> Self {
        self.inner.borrow_mut().checked = checked;
        self
    }

    pub fn on_change<F>(self, handler: F) -> Self
    where
        F: FnMut(bool) -> Result<()> + 'static,
    {
        self.inner.borrow_mut().change_handler = Some(Box::new(handler));
        self
    }

    pub fn is_checked(&self) -> bool {
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                let result = SendMessageW(hwnd, BM_GETCHECK, None, None);
                result.0 == 1
            }
        } else {
            self.inner.borrow().checked
        }
    }

    pub fn set_checked(&self, checked: bool) {
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                SendMessageW(
                    hwnd,
                    BM_SETCHECK,
                    Some(WPARAM(if checked { 1 } else { 0 })),
                    None,
                );
            }
        }
        self.inner.borrow_mut().checked = checked;
    }
}

impl Control for CheckBox {
    fn create(&mut self, parent: HWND) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let mut inner = self.inner.borrow_mut();
            let wide_text = U16CString::from_str(&inner.label).unwrap_or_default();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                PCWSTR(wide_text.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                inner.position.0,
                inner.position.1,
                inner.size.0,
                inner.size.1,
                Some(parent),
                Some(HMENU(inner.id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            if inner.checked {
                SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(1)), None);
            }

            inner.hwnd = Some(hwnd);
            Ok(hwnd)
        }
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_COMMAND => {
                let notification = (wparam.0 >> 16) as u16;
                if u32::from(notification) == BN_CLICKED {
                    let checked = self.is_checked();
                    self.inner.borrow_mut().checked = checked;

                    let mut inner = self.inner.borrow_mut();
                    if let Some(ref mut handler) = inner.change_handler {
                        let _ = handler(checked);
                    }
                    return Some(LRESULT(0));
                }
            }
            _ => {}
        }
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
