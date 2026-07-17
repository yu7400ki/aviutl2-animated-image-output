use super::{CreateCtx, MeasureCtx, Widget, dispatch_event, require_node};
use crate::Result;
use crate::layout::SizeValue;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    /// 既定ボタン。ダイアログ内でEnterキーを押したときに反応する
    #[default]
    Primary,
    Secondary,
}

impl From<ButtonVariant> for WINDOW_STYLE {
    fn from(variant: ButtonVariant) -> Self {
        match variant {
            ButtonVariant::Primary => WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            ButtonVariant::Secondary => WINDOW_STYLE(BS_PUSHBUTTON as u32),
        }
    }
}

struct ButtonInner {
    hwnd: Option<HWND>,
    label: String,
    variant: ButtonVariant,
    handlers: Vec<Box<dyn FnMut(())>>,
    width: SizeValue,
    height: SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct Button(Rc<RefCell<ButtonInner>>);

impl Button {
    pub fn new(label: &str) -> Self {
        Button(Rc::new(RefCell::new(ButtonInner {
            hwnd: None,
            label: label.to_string(),
            variant: ButtonVariant::default(),
            handlers: Vec::new(),
            width: SizeValue::Auto,
            height: SizeValue::Auto,
            node_id: None,
        })))
    }

    pub fn primary(label: &str) -> Self {
        Button::new(label).with_variant(ButtonVariant::Primary)
    }

    pub fn secondary(label: &str) -> Self {
        Button::new(label).with_variant(ButtonVariant::Secondary)
    }

    pub fn with_variant(self, variant: ButtonVariant) -> Self {
        self.0.borrow_mut().variant = variant;
        self
    }

    pub fn with_width(self, width: SizeValue) -> Self {
        self.0.borrow_mut().width = width;
        self
    }

    pub fn with_height(self, height: SizeValue) -> Self {
        self.0.borrow_mut().height = height;
        self
    }

    /// クリック時のハンドラを追加する
    pub fn on_click<F>(self, mut handler: F) -> Self
    where
        F: FnMut() + 'static,
    {
        self.0
            .borrow_mut()
            .handlers
            .push(Box::new(move |()| handler()));
        self
    }
}

impl Widget for Button {
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
                    SizeValue::Auto => taffy::Dimension::length(text_width + 30.0),
                    other => other.clone().into(),
                },
                height: match &inner.height {
                    SizeValue::Auto => taffy::Dimension::length(text_height + 10.0),
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
        let node_id = require_node(self.0.borrow().node_id, "Button")?;
        let rect = ctx.control_rect(node_id, offset)?;
        let id = ctx.alloc_id();

        let (label, variant) = {
            let inner = self.0.borrow();
            (HSTRING::from(inner.label.as_str()), inner.variant)
        };

        let hwnd = ctx.create_control(
            w!("BUTTON"),
            PCWSTR(label.as_ptr()),
            WS_TABSTOP | variant.into(),
            WINDOW_EX_STYLE(0),
            rect,
            id,
        )?;

        if variant == ButtonVariant::Primary {
            ctx.set_default_button(id);
        }

        self.0.borrow_mut().hwnd = Some(hwnd);
        Ok(vec![id])
    }

    fn on_command(&self, code: u16) {
        if u32::from(code) == BN_CLICKED {
            dispatch_event(&self.0, |inner| &mut inner.handlers, ());
        }
    }

    fn cache_state(&self) {
        self.0.borrow_mut().hwnd = None;
    }

    fn hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }
}
