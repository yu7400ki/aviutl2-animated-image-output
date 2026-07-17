use super::{CreateCtx, MeasureCtx, Widget, get_window_text, require_node};
use crate::Result;
use crate::layout::SizeValue;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

struct NumberInner {
    hwnd: Option<HWND>,
    updown_hwnd: Option<HWND>,
    text: String,
    range: Option<(i32, i32)>,
    enabled: bool,
    width: SizeValue,
    height: SizeValue,
    node_id: Option<taffy::NodeId>,
}

/// スピンボタン付きの数値入力コントロール
#[derive(Clone)]
pub struct Number(Rc<RefCell<NumberInner>>);

impl Number {
    pub fn new() -> Self {
        Number(Rc::new(RefCell::new(NumberInner {
            hwnd: None,
            updown_hwnd: None,
            text: "0".to_string(),
            range: None,
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

    pub fn value(self, val: i32) -> Self {
        self.0.borrow_mut().text = val.to_string();
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
        match self.hwnd() {
            Some(hwnd) => get_window_text(hwnd),
            None => self.0.borrow().text.clone(),
        }
    }

    pub fn get_value<T: std::str::FromStr>(&self) -> std::result::Result<T, T::Err> {
        self.get_text().parse::<T>()
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

    pub fn set_value<T: ToString>(&self, value: T) {
        self.set_text(&value.to_string());
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

impl Default for Number {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Number {
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
        let node_id = require_node(self.0.borrow().node_id, "Number")?;
        let rect = ctx.control_rect(node_id, offset)?;
        let edit_id = ctx.alloc_id();
        let updown_id = ctx.alloc_id();

        let hwnd_edit = ctx.create_control(
            w!("EDIT"),
            PCWSTR::null(),
            WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32),
            WS_EX_CLIENTEDGE,
            rect,
            edit_id,
        )?;

        // スピンボタン。UDS_ALIGNRIGHTによりUDM_SETBUDDYでエディットの右端に配置される
        let updown_rect = super::ControlRect {
            x: 0,
            y: 0,
            width: 0,
            height: rect.height,
        };
        let hwnd_updown = ctx.create_control(
            UPDOWN_CLASS,
            PCWSTR::null(),
            WINDOW_STYLE(UDS_ALIGNRIGHT)
                | WINDOW_STYLE(UDS_SETBUDDYINT)
                | WINDOW_STYLE(UDS_ARROWKEYS),
            WINDOW_EX_STYLE(0),
            updown_rect,
            updown_id,
        )?;

        unsafe {
            let text = self.0.borrow().text.clone();
            let hstring = HSTRING::from(text.as_str());
            let _ = SetWindowTextW(hwnd_edit, &hstring);

            SendMessageW(
                hwnd_updown,
                UDM_SETBUDDY,
                Some(WPARAM(hwnd_edit.0 as usize)),
                Some(LPARAM(0)),
            );

            if let Some((min, max)) = self.0.borrow().range {
                SendMessageW(
                    hwnd_updown,
                    UDM_SETRANGE32,
                    Some(WPARAM(min as usize)),
                    Some(LPARAM(max as isize)),
                );
            }

            let enabled = self.0.borrow().enabled;
            let _ = EnableWindow(hwnd_edit, enabled);
        }

        let mut inner = self.0.borrow_mut();
        inner.hwnd = Some(hwnd_edit);
        inner.updown_hwnd = Some(hwnd_updown);
        // 通知は使用しないためコマンドIDは登録しない
        Ok(Vec::new())
    }

    fn cache_state(&self) {
        let text = self.get_text();
        let mut inner = self.0.borrow_mut();
        inner.text = text;
        inner.hwnd = None;
        inner.updown_hwnd = None;
    }

    fn hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }
}
