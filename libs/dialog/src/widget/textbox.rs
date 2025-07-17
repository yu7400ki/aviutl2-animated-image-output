use crate::layout::SizeValue;
use crate::{ControlId, DialogError, Result, Widget};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

struct TextBoxInner {
    hwnd: Option<HWND>,
    id: ControlId,
    text: String,
    enabled: bool,
    width: crate::layout::SizeValue,
    height: crate::layout::SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct TextBox(Rc<RefCell<TextBoxInner>>);

impl TextBox {
    pub fn new() -> Self {
        TextBox(Rc::new(RefCell::new(TextBoxInner {
            hwnd: None,
            id: ControlId::new(),
            text: String::new(),
            enabled: true,
            width: SizeValue::Percent(1.0),
            height: SizeValue::Points(25.0),
            node_id: None,
        })))
    }

    pub fn with_width(self, width: crate::layout::SizeValue) -> Self {
        self.0.borrow_mut().width = width;
        self
    }

    pub fn with_height(self, height: crate::layout::SizeValue) -> Self {
        self.0.borrow_mut().height = height;
        self
    }

    pub fn text(self, text: &str) -> Self {
        self.0.borrow_mut().text = text.to_string();
        self
    }

    pub fn enabled(self, enabled: bool) -> Self {
        self.0.borrow_mut().enabled = enabled;
        self
    }

    pub fn get_text(&self) -> String {
        let inner = self.0.borrow();
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
            None => inner.text.clone(),
        }
    }

    pub fn set_text(&self, text: &str) {
        let mut inner = self.0.borrow_mut();
        inner.text = text.to_string();
        if let Some(hwnd) = inner.hwnd {
            let hstring = HSTRING::from(text);
            unsafe {
                let _ = SetWindowTextW(hwnd, &hstring);
            }
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.0.borrow_mut().enabled = enabled;
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                let _ = EnableWindow(hwnd, enabled);
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.borrow().enabled
    }
}

impl Widget for TextBox {
    fn get_id(&self) -> ControlId {
        self.0.borrow().id.clone()
    }

    fn get_hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }

    fn handle_message(&mut self, _msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        None
    }

    fn create_node(
        &self,
        tree: &mut taffy::TaffyTree,
        _font: Option<HFONT>,
    ) -> Result<taffy::NodeId> {
        let size = {
            let inner = self.0.borrow();
            taffy::Size {
                width: inner.width.clone().into(),
                height: inner.height.clone().into(),
            }
        };

        let node = tree.new_leaf(taffy::Style {
            size,
            ..Default::default()
        })?;

        self.0.borrow_mut().node_id = Some(node);
        Ok(node)
    }

    fn create_window(
        &mut self,
        parent: HWND,
        taffy: &taffy::TaffyTree,
        position: (i32, i32),
    ) -> Result<()> {
        let node_id = self.0.borrow().node_id.ok_or_else(|| {
            DialogError::InvalidOperation("Node ID not set for textbox".to_string())
        })?;
        let layout = taffy.layout(node_id)?;

        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let hwnd = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP,
                layout.location.x as i32 + position.0,
                layout.location.y as i32 + position.1,
                layout.size.width as i32,
                layout.size.height as i32,
                Some(parent),
                Some(HMENU(self.0.borrow().id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            let initial_text = self.0.borrow().text.clone();
            if !initial_text.is_empty() {
                let hstring = HSTRING::from(initial_text.as_str());
                let _ = SetWindowTextW(hwnd, &hstring);
            }

            let enabled = self.0.borrow().enabled;
            let _ = EnableWindow(hwnd, enabled);

            self.0.borrow_mut().hwnd = Some(hwnd);
            Ok(())
        }
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

impl Default for TextBox {
    fn default() -> Self {
        Self::new()
    }
}
