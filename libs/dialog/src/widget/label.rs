use super::{CreateCtx, MeasureCtx, Widget, require_node};
use crate::Result;
use crate::layout::SizeValue;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

struct LabelInner {
    hwnd: Option<HWND>,
    text: String,
    width: SizeValue,
    height: SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct Label(Rc<RefCell<LabelInner>>);

impl Label {
    pub fn new(text: &str) -> Self {
        Label(Rc::new(RefCell::new(LabelInner {
            hwnd: None,
            text: text.to_string(),
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
    fn build_node(&self, tree: &mut taffy::TaffyTree, ctx: &MeasureCtx) -> Result<taffy::NodeId> {
        let size = {
            let inner = self.0.borrow();

            let (text_width, text_height) =
                if inner.width == SizeValue::Auto || inner.height == SizeValue::Auto {
                    ctx.text_size(&inner.text)?
                } else {
                    (0.0, 0.0)
                };

            taffy::Size {
                width: match &inner.width {
                    SizeValue::Auto => taffy::Dimension::length(text_width),
                    other => other.clone().into(),
                },
                height: match &inner.height {
                    SizeValue::Auto => taffy::Dimension::length(text_height),
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
        let node_id = require_node(self.0.borrow().node_id, "Label")?;
        let rect = ctx.control_rect(node_id, offset)?;
        let id = ctx.alloc_id();

        let text = HSTRING::from(self.0.borrow().text.as_str());
        let hwnd = ctx.create_control(
            w!("STATIC"),
            PCWSTR(text.as_ptr()),
            WINDOW_STYLE(0),
            WINDOW_EX_STYLE(0),
            rect,
            id,
        )?;

        self.0.borrow_mut().hwnd = Some(hwnd);
        // ラベルはWM_COMMAND通知を受け取らない
        Ok(Vec::new())
    }

    fn cache_state(&self) {
        self.0.borrow_mut().hwnd = None;
    }

    fn hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }
}
