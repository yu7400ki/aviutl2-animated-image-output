use super::{CreateCtx, MeasureCtx, Widget, dispatch_event, require_node};
use crate::Result;
use crate::layout::SizeValue;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

struct CheckBoxInner {
    hwnd: Option<HWND>,
    label: String,
    checked: bool,
    handlers: Vec<Box<dyn FnMut(bool)>>,
    width: SizeValue,
    height: SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct CheckBox(Rc<RefCell<CheckBoxInner>>);

impl CheckBox {
    pub fn new(label: &str) -> Self {
        CheckBox(Rc::new(RefCell::new(CheckBoxInner {
            hwnd: None,
            label: label.to_string(),
            checked: false,
            handlers: Vec::new(),
            width: SizeValue::Auto,
            height: SizeValue::Auto,
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

    pub fn checked(self, checked: bool) -> Self {
        self.set_checked(checked);
        self
    }

    /// チェック状態の変化時のハンドラを追加する
    pub fn on_change<F>(self, handler: F) -> Self
    where
        F: FnMut(bool) + 'static,
    {
        self.0.borrow_mut().handlers.push(Box::new(handler));
        self
    }

    pub fn is_checked(&self) -> bool {
        match self.hwnd() {
            Some(hwnd) => unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None).0 == 1 },
            None => self.0.borrow().checked,
        }
    }

    pub fn set_checked(&self, checked: bool) {
        if let Some(hwnd) = self.hwnd() {
            unsafe {
                SendMessageW(
                    hwnd,
                    BM_SETCHECK,
                    Some(WPARAM(if checked { 1 } else { 0 })),
                    None,
                );
            }
        }
        self.0.borrow_mut().checked = checked;
    }
}

impl Widget for CheckBox {
    fn build_node(&self, tree: &mut taffy::TaffyTree, ctx: &MeasureCtx) -> Result<taffy::NodeId> {
        let size = {
            let inner = self.0.borrow();

            let (text_width, text_height) =
                if inner.width == SizeValue::Auto || inner.height == SizeValue::Auto {
                    ctx.text_size(&inner.label)?
                } else {
                    (0.0, 0.0)
                };

            taffy::Size {
                width: match &inner.width {
                    // チェックマーク分の余白を足す
                    SizeValue::Auto => taffy::Dimension::length(text_width + 20.0),
                    other => other.clone().into(),
                },
                height: match &inner.height {
                    SizeValue::Auto => taffy::Dimension::length(text_height + 5.0),
                    other => other.clone().into(),
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

    fn create(&self, ctx: &mut CreateCtx, offset: (f32, f32)) -> Result<Vec<i32>> {
        let node_id = require_node(self.0.borrow().node_id, "CheckBox")?;
        let rect = ctx.control_rect(node_id, offset)?;
        let id = ctx.alloc_id();

        let (label, checked) = {
            let inner = self.0.borrow();
            (HSTRING::from(inner.label.as_str()), inner.checked)
        };

        let hwnd = ctx.create_control(
            w!("BUTTON"),
            PCWSTR(label.as_ptr()),
            WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
            WINDOW_EX_STYLE(0),
            rect,
            id,
        )?;

        if checked {
            unsafe {
                SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(1)), None);
            }
        }

        self.0.borrow_mut().hwnd = Some(hwnd);
        Ok(vec![id])
    }

    fn on_command(&self, code: u16) {
        if u32::from(code) == BN_CLICKED {
            let checked = self.is_checked();
            self.0.borrow_mut().checked = checked;
            dispatch_event(&self.0, |inner| &mut inner.handlers, checked);
        }
    }

    fn text(&self) -> Option<String> {
        Some(self.0.borrow().label.clone())
    }

    fn cache_state(&self) {
        let checked = self.is_checked();
        let mut inner = self.0.borrow_mut();
        inner.checked = checked;
        inner.hwnd = None;
    }

    fn hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }
}
