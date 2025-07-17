mod error;
pub mod font;
pub mod layout;
pub mod widget;
pub use error::{DialogError, Result};
pub use font::FontManager;

/// MessageBox wrapper for displaying messages with proper icon types
pub struct MessageBox;

impl MessageBox {
    /// Show an error message box
    pub fn error(parent: Option<HWND>, message: &str, title: &str) {
        Self::show_message(parent, message, title, MB_OK | MB_ICONERROR);
    }

    /// Show a warning message box
    pub fn warning(parent: Option<HWND>, message: &str, title: &str) {
        Self::show_message(parent, message, title, MB_OK | MB_ICONWARNING);
    }

    /// Show an info message box
    pub fn info(parent: Option<HWND>, message: &str, title: &str) {
        Self::show_message(parent, message, title, MB_OK | MB_ICONINFORMATION);
    }

    /// Show a message box with custom flags
    pub fn show_message(parent: Option<HWND>, message: &str, title: &str, flags: MESSAGEBOX_STYLE) {
        let message_wide = HSTRING::from(message);
        let title_wide = HSTRING::from(title);

        unsafe {
            MessageBoxW(parent, &message_wide, &title_wide, flags);
        }
    }
}

use crate::layout::Layout;
use crate::widget::Widget;
use std::cell::RefCell;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, PCWSTR, w};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlId(i32);

impl ControlId {
    pub fn new() -> Self {
        static COUNTER: AtomicI32 = AtomicI32::new(1000);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn from_raw(id: i32) -> Self {
        Self(id)
    }

    pub fn as_raw(&self) -> i32 {
        self.0
    }
}

struct DialogInner {
    /// Title displayed in the dialog's title bar
    title: String,
    hwnd: Option<HWND>,        // 自分自身のウィンドウHWND
    parent_hwnd: Option<HWND>, // 親ウィンドウのHWND
    layout: Box<dyn Layout>,   // ダイアログのレイアウト
    font_manager: FontManager, // フォント管理
}

#[derive(Clone)]
pub struct Dialog(Rc<RefCell<DialogInner>>);

impl Dialog {
    pub fn new(title: &str) -> Self {
        Dialog(Rc::new(RefCell::new(DialogInner {
            title: title.to_string(),
            hwnd: None,
            parent_hwnd: None,
            layout: Box::new(layout::FlexLayout::default()),
            font_manager: FontManager::default(),
        })))
    }

    pub fn with_layout<L: Layout + 'static>(self, layout: L) -> Self {
        self.0.borrow_mut().layout = Box::new(layout);
        self
    }

    pub fn with_font_manager(self, font_manager: FontManager) -> Self {
        self.0.borrow_mut().font_manager = font_manager;
        self
    }

    pub fn get_font_manager(&self) -> FontManager {
        self.0.borrow().font_manager.clone()
    }

    pub fn open(&mut self, parent_hwnd: HWND) -> Result<()> {
        if self.0.borrow().hwnd.is_some() {
            return Err(DialogError::InvalidOperation(
                "Dialog is already open".into(),
            ));
        }

        self.0.borrow_mut().parent_hwnd = Some(parent_hwnd);

        // Get font from font manager
        let font = Some(self.0.borrow().font_manager.get_font());

        let mut taffy = taffy::TaffyTree::new();
        let root = self.0.borrow_mut().layout.compute(&mut taffy, font)?;
        taffy.compute_layout(
            root,
            taffy::Size {
                width: taffy::AvailableSpace::MaxContent,
                height: taffy::AvailableSpace::MaxContent,
            },
        )?;
        let root_layout = taffy.layout(root)?;
        let (width, height) = (
            root_layout.size.width as i32,
            root_layout.size.height as i32,
        );

        let window_style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE;
        let window_ex_style = WS_EX_DLGMODALFRAME;
        let (dlg_width, dlg_height) = get_window_size(width, height, window_style, window_ex_style);
        let (x, y) = self.center_position(parent_hwnd, dlg_width, dlg_height);

        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let class = self.register_window_class(hinstance.into())?;

            let _ = EnableWindow(parent_hwnd, false); // Disable parent window while dialog is open

            let dialog_hwnd = CreateWindowExW(
                window_ex_style,
                class,
                &HSTRING::from(&self.0.borrow().title),
                window_style,
                x,
                y,
                dlg_width,
                dlg_height,
                Some(parent_hwnd),
                None,
                Some(hinstance.into()),
                None,
            );

            if dialog_hwnd.is_err() {
                let _ = EnableWindow(parent_hwnd, true); // Re-enable parent window
                self.0.borrow_mut().parent_hwnd = None;
                return Err(DialogError::from(dialog_hwnd.err().unwrap()));
            }
            let modal_hwnd = dialog_hwnd.unwrap();

            let hmenu = GetSystemMenu(modal_hwnd, false);
            if !hmenu.is_invalid() {
                // サイズ変更を無効化
                let _ = EnableMenuItem(hmenu, SC_SIZE, MF_BYCOMMAND | MF_GRAYED);
                // 最大化を無効化
                let _ = EnableMenuItem(hmenu, SC_MAXIMIZE, MF_BYCOMMAND | MF_GRAYED);
                // 最小化を無効化
                let _ = EnableMenuItem(hmenu, SC_MINIMIZE, MF_BYCOMMAND | MF_GRAYED);
            }

            self.0.borrow_mut().hwnd = Some(modal_hwnd);
            let dialog_ptr = self.0.as_ptr();
            SetWindowLongPtrW(modal_hwnd, GWLP_USERDATA, dialog_ptr as isize);

            // self.create_controls(modal_hwnd)?;
            self.0
                .borrow_mut()
                .layout
                .create_window(modal_hwnd, &taffy, (0, 0))?;

            // Apply font to all widgets
            self.apply_font_to_widgets();

            let _ = ShowWindow(modal_hwnd, SW_SHOW);
            let _ = UpdateWindow(modal_hwnd);

            // メッセージループ
            let mut msg = MSG::default();
            loop {
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 == 0 {
                    break; // WM_QUIT
                } else if result.0 == -1 {
                    return Err(DialogError::Win32Error(Error::from_win32()));
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // 親ウィンドウの有効化・前面化はWM_DESTROYで行う
            self.0.borrow_mut().parent_hwnd = None;
        }

        Ok(())
    }

    pub fn close(&self) {
        if let Some(hwnd) = self.0.borrow().hwnd {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn center_position(&self, parent_hwnd: HWND, dlg_width: i32, dlg_height: i32) -> (i32, i32) {
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(parent_hwnd, &mut rect).is_ok() {
                let parent_width = rect.right - rect.left;
                let parent_height = rect.bottom - rect.top;
                let center_x = rect.left + (parent_width - dlg_width) / 2;
                let center_y = rect.top + (parent_height - dlg_height) / 2;
                (center_x, center_y)
            } else {
                (CW_USEDEFAULT, CW_USEDEFAULT)
            }
        }
    }

    unsafe fn register_window_class(&self, hinstance: HINSTANCE) -> Result<PCWSTR> {
        let class_name = w!("DialogWindowClass");

        unsafe {
            let mut wc = WNDCLASSEXW::default();
            if GetClassInfoExW(Some(hinstance), class_name, &mut wc).is_err() {
                wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(modal_wnd_proc),
                    cbClsExtra: 0,
                    cbWndExtra: 0,
                    hInstance: hinstance,
                    hIcon: LoadIconW(None, IDI_APPLICATION)?,
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut c_void),
                    lpszClassName: class_name,
                    lpszMenuName: PCWSTR::null(),
                    hIconSm: HICON::default(),
                };
                if RegisterClassExW(&wc) == 0 {
                    return Err(DialogError::Win32Error(Error::from_win32()));
                }
            }
        }

        Ok(class_name)
    }

    fn apply_font_to_widgets(&self) {
        let inner = self.0.borrow();
        let font = inner.font_manager.get_font();
        inner.layout.apply_font(font);
    }
}

unsafe extern "system" fn modal_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let dialog_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    let dialog = if dialog_ptr == 0 {
        None
    } else {
        Some(unsafe { &mut *(dialog_ptr as *mut DialogInner) })
    };
    match msg {
        WM_CREATE => LRESULT(0),
        WM_COMMAND => {
            if let Some(dialog) = dialog {
                if let Some(result) = dialog.layout.handle_message(msg, w_param, l_param) {
                    return result;
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                if let Some(dialog) = dialog {
                    if let Some(parent_hwnd) = dialog.parent_hwnd {
                        let _ = EnableWindow(parent_hwnd, true);
                        let _ = SetForegroundWindow(parent_hwnd);
                    }
                    dialog.hwnd = None;
                }
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}

pub fn get_text_size(text: &str, font: Option<HFONT>) -> Result<(i32, i32)> {
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return Err(DialogError::Win32Error(Error::from_win32()));
        }

        if let Some(font) = font {
            SelectObject(hdc, font.into());
        }

        let mut size = SIZE::default();
        let result = GetTextExtentPoint32W(hdc, &HSTRING::from(text).to_vec(), &mut size);
        ReleaseDC(None, hdc);
        if result.as_bool() {
            Ok((size.cx, size.cy))
        } else {
            Err(DialogError::Win32Error(Error::from_win32()))
        }
    }
}

fn get_window_size(
    client_width: i32,
    client_height: i32,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    unsafe {
        let _ = AdjustWindowRectEx(&mut rect, style, false, ex_style);
    };
    let win_width = rect.right - rect.left;
    let win_height = rect.bottom - rect.top;
    (win_width, win_height)
}
