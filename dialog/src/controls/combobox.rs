use crate::{Control, ControlId, Result};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

struct ComboBoxInner {
    hwnd: Option<HWND>,
    id: ControlId,
    position: (i32, i32),
    size: (i32, i32),
    items: Vec<String>,
    selected_index: i32,
}

#[derive(Clone)]
pub struct ComboBox {
    inner: Rc<RefCell<ComboBoxInner>>,
}

impl ComboBox {
    pub fn new(items: Vec<&str>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ComboBoxInner {
                hwnd: None,
                id: ControlId::new(),
                position: (0, 0),
                size: (150, 100),
                items: items.into_iter().map(|s| s.to_string()).collect(),
                selected_index: 0,
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

    pub fn selected(self, index: i32) -> Self {
        self.inner.borrow_mut().selected_index = index;
        self
    }

    pub fn get_selected_index(&self) -> i32 {
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                let result = SendMessageW(hwnd, CB_GETCURSEL, None, None);
                result.0 as i32
            }
        } else {
            self.inner.borrow().selected_index
        }
    }

    pub fn set_selected_index(&self, index: i32) {
        self.inner.borrow_mut().selected_index = index;
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index as usize)), None);
            }
        }
    }

    pub fn get_selected_text(&self) -> String {
        let index = self.get_selected_index();
        let inner = self.inner.borrow();
        if index >= 0 && (index as usize) < inner.items.len() {
            inner.items[index as usize].clone()
        } else {
            String::new()
        }
    }
}

impl Control for ComboBox {
    fn create(&mut self, parent: HWND) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let mut inner = self.inner.borrow_mut();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("COMBOBOX"),
                PCWSTR::null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WS_VSCROLL
                    | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                inner.position.0,
                inner.position.1,
                inner.size.0,
                inner.size.1,
                Some(parent),
                Some(HMENU(inner.id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            // Add items to combobox
            for item in &inner.items {
                let wide_text: Vec<u16> = item.encode_utf16().chain([0]).collect();
                SendMessageW(
                    hwnd,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(wide_text.as_ptr() as isize)),
                );
            }

            // Set initial selection
            SendMessageW(
                hwnd,
                CB_SETCURSEL,
                Some(WPARAM(inner.selected_index as usize)),
                None,
            );

            inner.hwnd = Some(hwnd);
            Ok(hwnd)
        }
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_COMMAND => {
                let notification = (wparam.0 >> 16) as u16;
                if u32::from(notification) == CBN_SELCHANGE {
                    // Update internal state when selection changes
                    self.inner.borrow_mut().selected_index = self.get_selected_index();
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
