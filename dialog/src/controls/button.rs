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

struct ButtonInner {
    hwnd: Option<HWND>,
    id: ControlId,
    position: (i32, i32),
    size: (i32, i32),
    label: String,
    click_handler: Option<Box<dyn FnMut() -> Result<()>>>,
}

#[derive(Clone)]
pub struct Button {
    inner: Rc<RefCell<ButtonInner>>,
}

impl Button {
    pub fn new(label: &str) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ButtonInner {
                hwnd: None,
                id: ControlId::new(),
                position: (0, 0),
                size: (80, 25),
                label: label.to_string(),
                click_handler: None,
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

    pub fn on_click<F>(self, handler: F) -> Self
    where
        F: FnMut() -> Result<()> + 'static,
    {
        self.inner.borrow_mut().click_handler = Some(Box::new(handler));
        self
    }
}

impl Control for Button {
    fn create(&mut self, parent: HWND) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let mut inner = self.inner.borrow_mut();
            let wide_text = U16CString::from_str(&inner.label).unwrap_or_default();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                PCWSTR(wide_text.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                inner.position.0,
                inner.position.1,
                inner.size.0,
                inner.size.1,
                Some(parent),
                Some(HMENU(inner.id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            inner.hwnd = Some(hwnd);
            Ok(hwnd)
        }
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_COMMAND => {
                let notification = (wparam.0 >> 16) as u16;
                if u32::from(notification) == BN_CLICKED {
                    let mut inner = self.inner.borrow_mut();
                    if let Some(ref mut handler) = inner.click_handler {
                        let _ = handler();
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
