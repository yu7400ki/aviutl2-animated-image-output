use super::{CreateCtx, MeasureCtx, Widget, dispatch_event, require_node};
use crate::Result;
use crate::layout::SizeValue;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

struct ComboBoxInner {
    hwnd: Option<HWND>,
    items: Vec<String>,
    selected_index: i32,
    handlers: Vec<Box<dyn FnMut(i32)>>,
    width: SizeValue,
    height: SizeValue,
    node_id: Option<taffy::NodeId>,
    enabled: bool,
}

#[derive(Clone)]
pub struct ComboBox(Rc<RefCell<ComboBoxInner>>);

impl ComboBox {
    pub fn new(items: Vec<&str>) -> Self {
        ComboBox(Rc::new(RefCell::new(ComboBoxInner {
            hwnd: None,
            items: items.into_iter().map(|s| s.to_string()).collect(),
            selected_index: 0,
            handlers: Vec::new(),
            width: SizeValue::Percent(1.0),
            height: SizeValue::Points(25.0),
            node_id: None,
            enabled: true,
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

    pub fn selected(self, index: i32) -> Self {
        self.set_selected_index(index);
        self
    }

    /// 選択変更時のハンドラを追加する
    pub fn on_change<F>(self, handler: F) -> Self
    where
        F: FnMut(i32) + 'static,
    {
        self.0.borrow_mut().handlers.push(Box::new(handler));
        self
    }

    pub fn selected_index(&self) -> i32 {
        match self.hwnd() {
            Some(hwnd) => unsafe { SendMessageW(hwnd, CB_GETCURSEL, None, None).0 as i32 },
            None => self.0.borrow().selected_index,
        }
    }

    pub fn set_selected_index(&self, index: i32) {
        self.0.borrow_mut().selected_index = index;
        if let Some(hwnd) = self.hwnd() {
            unsafe {
                SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index as usize)), None);
            }
        }
    }

    pub fn selected_text(&self) -> String {
        let index = self.selected_index();
        let inner = self.0.borrow();
        if index >= 0 && (index as usize) < inner.items.len() {
            inner.items[index as usize].clone()
        } else {
            String::new()
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
}

impl Widget for ComboBox {
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
        let node_id = require_node(self.0.borrow().node_id, "ComboBox")?;
        let rect = ctx.control_rect(node_id, offset)?;
        let id = ctx.alloc_id();

        let hwnd = ctx.create_control(
            w!("COMBOBOX"),
            PCWSTR::null(),
            WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
            WINDOW_EX_STYLE(0),
            rect,
            id,
        )?;

        unsafe {
            for item in &self.0.borrow().items {
                let hstring = HSTRING::from(item.as_str());
                SendMessageW(
                    hwnd,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(hstring.as_ptr() as isize)),
                );
            }

            // CreateWindowExWに渡した高さは閉じた状態のもの。ドロップダウン
            // リストの分を、フォント適用後の実際のアイテム高さから計算して確保する
            let item_count = self.0.borrow().items.len() as i32;
            let item_height = SendMessageW(hwnd, CB_GETITEMHEIGHT, Some(WPARAM(0)), None).0 as i32;
            if item_height > 0 {
                let total_height = rect.height + item_height * item_count.max(1) + 2;
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    rect.width,
                    total_height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }

            let selected_index = self.0.borrow().selected_index;
            SendMessageW(
                hwnd,
                CB_SETCURSEL,
                Some(WPARAM(selected_index as usize)),
                None,
            );

            let enabled = self.0.borrow().enabled;
            let _ = EnableWindow(hwnd, enabled);
        }

        self.0.borrow_mut().hwnd = Some(hwnd);
        Ok(vec![id])
    }

    fn on_command(&self, code: u16) {
        if u32::from(code) == CBN_SELCHANGE {
            let index = self.selected_index();
            self.0.borrow_mut().selected_index = index;
            dispatch_event(&self.0, |inner| &mut inner.handlers, index);
        }
    }

    fn cache_state(&self) {
        let index = self.selected_index();
        let mut inner = self.0.borrow_mut();
        if index >= 0 {
            inner.selected_index = index;
        }
        inner.hwnd = None;
    }

    fn hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }
}
