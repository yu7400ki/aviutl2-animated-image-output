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

pub enum ComboBoxEvent {
    SelectionChanged(i32),
}

struct ComboBoxInner {
    hwnd: Option<HWND>,
    id: ControlId,
    items: Vec<String>,
    selected_index: i32,
    event_handlers: Vec<Box<dyn FnMut(ComboBoxEvent)>>,
    width: crate::layout::SizeValue,
    height: crate::layout::SizeValue,
    node_id: Option<taffy::NodeId>,
    enabled: bool,
}

#[derive(Clone)]
pub struct ComboBox(Rc<RefCell<ComboBoxInner>>);

impl ComboBox {
    pub fn new(items: Vec<&str>) -> Self {
        ComboBox(Rc::new(RefCell::new(ComboBoxInner {
            hwnd: None,
            id: ControlId::new(),
            items: items.into_iter().map(|s| s.to_string()).collect(),
            selected_index: 0,
            event_handlers: Vec::new(),
            width: SizeValue::Percent(1.0),
            height: SizeValue::Points(25.0),
            node_id: None,
            enabled: true,
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

    pub fn selected(self, index: i32) -> Self {
        self.0.borrow_mut().selected_index = index;
        self
    }

    pub fn add_event_handler<F>(self, handler: F) -> Self
    where
        F: FnMut(ComboBoxEvent) + 'static,
    {
        self.0.borrow_mut().event_handlers.push(Box::new(handler));
        self
    }

    pub fn get_selected_index(&self) -> i32 {
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                let result = SendMessageW(hwnd, CB_GETCURSEL, None, None);
                result.0 as i32
            }
        } else {
            self.0.borrow().selected_index
        }
    }

    pub fn set_selected_index(&self, index: i32) {
        self.0.borrow_mut().selected_index = index;
        if let Some(hwnd) = self.get_hwnd() {
            unsafe {
                SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index as usize)), None);
            }
        }
    }

    pub fn get_selected_text(&self) -> String {
        let index = self.get_selected_index();
        let inner = self.0.borrow();
        if index >= 0 && (index as usize) < inner.items.len() {
            inner.items[index as usize].clone()
        } else {
            String::new()
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
}

impl Widget for ComboBox {
    fn get_id(&self) -> ControlId {
        self.0.borrow().id.clone()
    }

    fn get_hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }

    fn handle_message(&mut self, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_COMMAND => {
                let command_id = (wparam.0 & 0xFFFF) as i32;
                let notification = (wparam.0 >> 16) as u16;

                let id = self.0.borrow().id.as_raw();
                if command_id == id && u32::from(notification) == CBN_SELCHANGE {
                    let new_index = self.get_selected_index();
                    self.0.borrow_mut().selected_index = new_index;

                    for handler in &mut self.0.borrow_mut().event_handlers {
                        handler(ComboBoxEvent::SelectionChanged(new_index));
                    }
                    return Some(LRESULT(0));
                }
            }
            _ => {}
        }
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
            DialogError::InvalidOperation("Node ID not set for combobox".to_string())
        })?;
        let layout = taffy.layout(node_id)?;

        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("COMBOBOX"),
                PCWSTR::null(),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WS_VSCROLL
                    | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
                layout.location.x as i32 + position.0,
                layout.location.y as i32 + position.1,
                layout.size.width as i32,
                (layout.size.height as i32) * self.0.borrow().items.len() as i32,
                Some(parent),
                Some(HMENU(self.0.borrow().id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            // Add items to combobox
            for item in &self.0.borrow().items {
                let hstring = HSTRING::from(item.as_str());
                SendMessageW(
                    hwnd,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(hstring.as_ptr() as isize)),
                );
            }

            // Set initial selection
            let selected_index = self.0.borrow().selected_index;
            SendMessageW(
                hwnd,
                CB_SETCURSEL,
                Some(WPARAM(selected_index as usize)),
                None,
            );

            // Set initial enabled state
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
