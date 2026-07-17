use super::{CreateCtx, MeasureCtx, Widget, get_window_text, require_node};
use crate::Result;
use crate::layout::SizeValue;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

struct TextBoxInner {
    hwnd: Option<HWND>,
    text: String,
    enabled: bool,
    width: SizeValue,
    height: SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct TextBox(Rc<RefCell<TextBoxInner>>);

impl TextBox {
    pub fn new() -> Self {
        TextBox(Rc::new(RefCell::new(TextBoxInner {
            hwnd: None,
            text: String::new(),
            enabled: true,
            width: SizeValue::Percent(1.0),
            height: SizeValue::Points(25.0),
            node_id: None,
        })))
    }

    pub fn with_width(self, width: SizeValue) -> Self {
        self.0.borrow_mut().width = width;
        self
    }

    pub fn with_height(self, height: SizeValue) -> Self {
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
        match self.hwnd() {
            Some(hwnd) => get_window_text(hwnd),
            None => self.0.borrow().text.clone(),
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
        if let Some(hwnd) = self.hwnd() {
            unsafe {
                let _ = EnableWindow(hwnd, enabled);
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.borrow().enabled
    }
}

impl Default for TextBox {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TextBox {
    fn build_node(&self, tree: &mut taffy::TaffyTree, _ctx: &MeasureCtx) -> Result<taffy::NodeId> {
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

    fn create(&self, ctx: &mut CreateCtx, offset: (f32, f32)) -> Result<Vec<i32>> {
        let node_id = require_node(self.0.borrow().node_id, "TextBox")?;
        let rect = ctx.control_rect(node_id, offset)?;
        let id = ctx.alloc_id();

        let hwnd = ctx.create_control(
            w!("EDIT"),
            PCWSTR::null(),
            WS_BORDER | WS_TABSTOP,
            WS_EX_CLIENTEDGE,
            rect,
            id,
        )?;

        unsafe {
            let text = self.0.borrow().text.clone();
            if !text.is_empty() {
                let hstring = HSTRING::from(text.as_str());
                let _ = SetWindowTextW(hwnd, &hstring);
            }

            let enabled = self.0.borrow().enabled;
            let _ = EnableWindow(hwnd, enabled);
        }

        self.0.borrow_mut().hwnd = Some(hwnd);
        Ok(Vec::new())
    }

    fn cache_state(&self) {
        let text = self.get_text();
        let mut inner = self.0.borrow_mut();
        inner.text = text;
        inner.hwnd = None;
    }

    fn hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }
}
