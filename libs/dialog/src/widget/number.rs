use crate::layout::SizeValue;
use crate::{ControlId, DialogError, Result, Widget};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::{Controls::*, WindowsAndMessaging::*};
use windows::core::*;

struct NumberInner {
    hwnd: Option<HWND>,
    updown_hwnd: Option<HWND>,
    id: ControlId,
    value: i32,
    range: Option<(i32, i32)>,
    enabled: bool,
    width: crate::layout::SizeValue,
    height: crate::layout::SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct Number(Rc<RefCell<NumberInner>>);

impl Number {
    pub fn new() -> Self {
        Number(Rc::new(RefCell::new(NumberInner {
            hwnd: None,
            updown_hwnd: None,
            id: ControlId::new(),
            value: 0,
            range: None,
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

    pub fn value(self, val: i32) -> Self {
        self.0.borrow_mut().value = val;
        self
    }

    pub fn range(self, min: i32, max: i32) -> Self {
        self.0.borrow_mut().range = Some((min, max));
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
            None => inner.value.to_string(),
        }
    }

    pub fn get_value<T: std::str::FromStr>(&self) -> std::result::Result<T, T::Err> {
        self.get_text().parse::<T>()
    }

    pub fn set_text(&self, text: &str) {
        let inner = self.0.borrow();
        if let Some(hwnd) = inner.hwnd {
            let hstring = HSTRING::from(text);
            unsafe {
                let _ = SetWindowTextW(hwnd, &hstring);
            }
        }
    }

    pub fn set_value<T: ToString>(&self, value: T) {
        self.set_text(&value.to_string());
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

impl Widget for Number {
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
            DialogError::InvalidOperation("Node ID not set for number".to_string())
        })?;
        let layout = taffy.layout(node_id)?;

        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            // Create the edit control
            let hwnd_edit = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
                layout.location.x as i32 + position.0,
                layout.location.y as i32 + position.1,
                layout.size.width as i32,
                layout.size.height as i32,
                Some(parent),
                Some(HMENU(self.0.borrow().id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            let enabled = self.0.borrow().enabled;
            let _ = EnableWindow(hwnd_edit, enabled);

            // Create the updown control
            let hwnd_updown = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                UPDOWN_CLASS,
                PCWSTR::null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WINDOW_STYLE(UDS_ALIGNRIGHT as u32)
                    | WINDOW_STYLE(UDS_SETBUDDYINT as u32)
                    | WINDOW_STYLE(UDS_ARROWKEYS as u32),
                layout.location.x as i32,
                layout.location.y as i32,
                0,
                layout.size.height as i32,
                Some(parent),
                Some(HMENU((self.0.borrow().id.as_raw() + 1000) as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            let initial_value = self.0.borrow().value;
            let hstring = HSTRING::from(initial_value.to_string().as_str());
            let _ = SetWindowTextW(hwnd_edit, &hstring);

            // Set updown control's buddy to the edit control
            SendMessageW(
                hwnd_updown,
                UDM_SETBUDDY,
                Some(WPARAM(hwnd_edit.0 as usize)),
                Some(LPARAM(0)),
            );

            // Set range if specified
            if let Some((min, max)) = self.0.borrow().range {
                SendMessageW(
                    hwnd_updown,
                    UDM_SETRANGE32,
                    Some(WPARAM(min as usize)),
                    Some(LPARAM(max as isize)),
                );
            }

            self.0.borrow_mut().hwnd = Some(hwnd_edit);
            self.0.borrow_mut().updown_hwnd = Some(hwnd_updown);

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

impl Default for Number {
    fn default() -> Self {
        Self::new()
    }
}
