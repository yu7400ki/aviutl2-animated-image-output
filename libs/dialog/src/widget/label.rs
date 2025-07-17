use crate::layout::SizeValue;
use crate::{ControlId, DialogError, Result, Widget, get_text_size};
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
    id: ControlId,
    text: String,
    width: crate::layout::SizeValue,
    height: crate::layout::SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct Label(Rc<RefCell<LabelInner>>);

impl Label {
    pub fn new(text: &str) -> Self {
        Label(Rc::new(RefCell::new(LabelInner {
            hwnd: None,
            id: ControlId::new(),
            text: text.to_string(),
            width: SizeValue::Auto,
            height: SizeValue::Auto,
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

    pub fn get_text(&self) -> String {
        self.0.borrow().text.clone()
    }
}

impl Widget for Label {
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
        font: Option<HFONT>,
    ) -> Result<taffy::NodeId> {
        let size = {
            let inner = self.0.borrow();

            // Calculate text size with the provided font only when needed
            let (text_width, text_height) =
                if inner.width == SizeValue::Auto || inner.height == SizeValue::Auto {
                    get_text_size(&inner.text, font)
                        .map(|(w, h)| (w as f32, h as f32))
                        .unwrap_or((0.0, 0.0))
                } else {
                    (0.0, 0.0)
                };

            taffy::Size {
                width: if inner.width == SizeValue::Auto {
                    taffy::Dimension::length(text_width)
                } else {
                    inner.width.clone().into()
                },
                height: if inner.height == SizeValue::Auto {
                    taffy::Dimension::length(text_height)
                } else {
                    inner.height.clone().into()
                },
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
            DialogError::InvalidOperation("Node ID not set for label".to_string())
        })?;
        let layout = taffy.layout(node_id)?;

        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let hstring = HSTRING::from(self.0.borrow().text.as_str());

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                PCWSTR(hstring.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                layout.location.x as i32 + position.0,
                layout.location.y as i32 + position.1,
                layout.size.width as i32,
                layout.size.height as i32,
                Some(parent),
                Some(HMENU(self.0.borrow().id.as_raw() as *mut c_void)),
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

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
