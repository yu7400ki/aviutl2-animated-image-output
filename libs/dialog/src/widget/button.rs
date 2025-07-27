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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
}

impl Default for ButtonVariant {
    fn default() -> Self {
        ButtonVariant::Primary
    }
}

impl Into<WINDOW_STYLE> for ButtonVariant {
    fn into(self) -> WINDOW_STYLE {
        match self {
            ButtonVariant::Primary => WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            ButtonVariant::Secondary => WINDOW_STYLE(BS_PUSHBUTTON as u32),
        }
    }
}

pub enum ButtonEvent {
    Click,
}

struct ButtonInner {
    hwnd: Option<HWND>,
    id: ControlId,
    label: String,
    variant: ButtonVariant,
    event_handlers: Vec<Box<dyn FnMut(ButtonEvent)>>,
    width: crate::layout::SizeValue,
    height: crate::layout::SizeValue,
    node_id: Option<taffy::NodeId>,
}

#[derive(Clone)]
pub struct Button(Rc<RefCell<ButtonInner>>);

impl Button {
    pub fn new(label: &str) -> Self {
        Button(Rc::new(RefCell::new(ButtonInner {
            hwnd: None,
            id: ControlId::new(),
            label: label.to_string(),
            variant: ButtonVariant::default(),
            event_handlers: Vec::new(),
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

    pub fn with_width(self, width: crate::layout::SizeValue) -> Self {
        self.0.borrow_mut().width = width;
        self
    }

    pub fn with_height(self, height: crate::layout::SizeValue) -> Self {
        self.0.borrow_mut().height = height;
        self
    }

    pub fn get_hwnd(&self) -> Option<HWND> {
        self.0.borrow().hwnd
    }

    pub fn with_variant(self, variant: ButtonVariant) -> Self {
        self.0.borrow_mut().variant = variant;
        self
    }

    pub fn add_event_handler<F>(self, handler: F) -> Self
    where
        F: FnMut(ButtonEvent) + 'static,
    {
        self.0.borrow_mut().event_handlers.push(Box::new(handler));
        self
    }
}

impl Widget for Button {
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

                // このボタンのIDと一致するかチェック
                let id = self.0.borrow().id.as_raw();
                if command_id == id && u32::from(notification) == BN_CLICKED {
                    for handler in &mut self.0.borrow_mut().event_handlers {
                        handler(ButtonEvent::Click);
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
        font: Option<HFONT>,
    ) -> Result<taffy::NodeId> {
        let size = {
            let inner = self.0.borrow();

            let (text_width, text_height) = if inner.width == crate::layout::SizeValue::Auto
                || inner.height == crate::layout::SizeValue::Auto
            {
                get_text_size(&inner.label, font)
                    .map(|(w, h)| (w as f32, h as f32))
                    .unwrap_or((0.0, 0.0))
            } else {
                (0.0, 0.0)
            };

            taffy::Size {
                width: if inner.width == crate::layout::SizeValue::Auto {
                    taffy::Dimension::length(text_width + 30.0) // Add padding
                } else {
                    inner.width.clone().into()
                },
                height: if inner.height == crate::layout::SizeValue::Auto {
                    taffy::Dimension::length(text_height + 10.0) // Add padding
                } else {
                    inner.height.clone().into()
                },
            }
        };

        let node = tree.new_leaf(taffy::Style {
            size,
            ..Default::default()
        })?;

        // Store the node ID in the button for later use
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
            DialogError::InvalidOperation("Node ID not set for button".to_string())
        })?;
        let layout = taffy.layout(node_id)?;

        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let hstring = HSTRING::from(self.0.borrow().label.as_str());

            let button_style: WINDOW_STYLE = self.0.borrow().variant.into();

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                PCWSTR(hstring.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | button_style,
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
