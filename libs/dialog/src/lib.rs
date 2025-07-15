use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering};
use widestring::U16CString;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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

fn loword(dword: u32) -> u16 {
    (dword & 0xFFFF) as u16
}

pub trait Control {
    fn create(&mut self, parent: HWND) -> Result<HWND>;
    fn handle_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT>;
    fn get_id(&self) -> ControlId;
    fn get_hwnd(&self) -> Option<HWND>;
    fn set_font(&self, font: HFONT);
}

struct DialogInner {
    hwnd: Option<HWND>,
    title: String,
    size: (i32, i32),
    controls: HashMap<ControlId, Box<dyn Control>>,
    font: Option<HFONT>,
}

#[derive(Clone)]
pub struct Dialog {
    inner: Rc<RefCell<DialogInner>>,
}

impl Dialog {
    pub fn new(title: &str, size: (i32, i32)) -> Self {
        Self {
            inner: Rc::new(RefCell::new(DialogInner {
                hwnd: None,
                title: title.to_string(),
                size,
                controls: HashMap::new(),
                font: None,
            })),
        }
    }

    fn add_control(&mut self, control: Box<dyn Control>) {
        let id = control.get_id();
        self.inner.borrow_mut().controls.insert(id, control);
    }

    pub fn with_control<C: Control + 'static>(mut self, control: C) -> Self {
        self.add_control(Box::new(control));
        self
    }

    pub fn close(&self) {
        let mut inner = self.inner.borrow_mut();
        if let Some(hwnd) = inner.hwnd {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        if let Some(font) = inner.font {
            unsafe {
                let _ = DeleteObject(font.into());
            }
        }
        inner.font = None;
    }

    fn create_font(&self) -> Result<HFONT> {
        unsafe {
            let font = CreateFontW(
                -12,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                SHIFTJIS_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                DEFAULT_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Meiryo UI"),
            );

            if font.is_invalid() {
                Err("Failed to create font".into())
            } else {
                Ok(font)
            }
        }
    }

    pub fn open(&mut self, parent: HWND) -> Result<()> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let dialog_class = w!("WinDialogClass");

            let wc = WNDCLASSEXW {
                cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(dialog_window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: HINSTANCE(hinstance.0),
                hIcon: HICON::default(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut c_void),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: dialog_class,
                hIconSm: HICON::default(),
            };

            RegisterClassExW(&wc);

            let title_wide = {
                let inner = self.inner.borrow();
                U16CString::from_str(&inner.title).unwrap_or_default()
            };
            let size = {
                let inner = self.inner.borrow();
                inner.size
            };

            let dialog_hwnd = CreateWindowExW(
                WS_EX_DLGMODALFRAME,
                dialog_class,
                PCWSTR(title_wide.as_ptr()),
                WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                0,
                0,
                size.0,
                size.1,
                Some(parent),
                None,
                Some(HINSTANCE(hinstance.0)),
                None,
            )?;

            self.inner.borrow_mut().hwnd = Some(dialog_hwnd);

            let dialog_ptr = self.inner.as_ptr();
            SetWindowLongPtrW(dialog_hwnd, GWLP_USERDATA, dialog_ptr as isize);

            // Create and store the font
            let font = self.create_font()?;
            self.inner.borrow_mut().font = Some(font);

            self.center_window(dialog_hwnd);

            self.create_controls(dialog_hwnd)?;

            // Apply font to all controls
            self.apply_font_to_controls();

            let _ = ShowWindow(dialog_hwnd, SW_SHOW);
            let _ = UpdateWindow(dialog_hwnd);

            let mut msg = MSG::default();
            let mut dialog_running = true;

            while dialog_running {
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 == 0 || result.0 == -1 {
                    break;
                }

                if !IsWindow(Some(dialog_hwnd)).as_bool() {
                    dialog_running = false;
                }

                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        Ok(())
    }

    fn center_window(&self, hwnd: HWND) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);

            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let screen_width = GetSystemMetrics(SM_CXSCREEN);
            let screen_height = GetSystemMetrics(SM_CYSCREEN);

            let x = (screen_width - width) / 2;
            let y = (screen_height - height) / 2;

            let _ = SetWindowPos(hwnd, Some(HWND_TOP), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
        }
    }

    fn create_controls(&mut self, parent: HWND) -> Result<()> {
        let mut inner = self.inner.borrow_mut();
        for (_, control) in inner.controls.iter_mut() {
            control.create(parent)?;
        }
        Ok(())
    }

    fn apply_font_to_controls(&self) {
        let inner = self.inner.borrow();
        if let Some(font) = inner.font {
            for (_, control) in inner.controls.iter() {
                control.set_font(font);
            }
        }
    }
}

impl Dialog {
    /// Create dialog with fluent interface
    pub fn create(title: &str) -> Self {
        Self::new(title, (400, 300))
    }

    /// Set dialog size
    pub fn size(self, width: i32, height: i32) -> Self {
        self.inner.borrow_mut().size = (width, height);
        self
    }
}

unsafe extern "system" fn dialog_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let dialog_inner_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogInner;

        if !dialog_inner_ptr.is_null() {
            let dialog_inner = &mut *dialog_inner_ptr;

            match msg {
                WM_COMMAND => {
                    let control_id = ControlId::from_raw(loword(wparam.0 as u32) as i32);
                    if let Some(control) = dialog_inner.controls.get_mut(&control_id) {
                        if let Some(result) = control.handle_message(msg, wparam, lparam) {
                            return result;
                        }
                    }
                }
                WM_CLOSE => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                _ => {}
            }
        }

        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

pub mod controls;

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
        let message_wide = U16CString::from_str(message).unwrap_or_default();
        let title_wide = U16CString::from_str(title).unwrap_or_default();

        unsafe {
            MessageBoxW(
                parent,
                PCWSTR(message_wide.as_ptr()),
                PCWSTR(title_wide.as_ptr()),
                flags,
            );
        }
    }
}
