mod error;
mod font;
pub mod layout;
mod messagebox;
pub mod widget;

pub use error::{DialogError, Result};
pub use font::Font;
pub use messagebox::MessageBox;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use crate::layout::Layout;
use crate::widget::{CreateCtx, MeasureCtx, WidgetEntry};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH, UpdateWindow};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleHandleExW,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, PCWSTR};

const BASE_DPI: f32 = 96.0;
/// コマンドIDの開始値。IDOK(1)/IDCANCEL(2)等の予約IDを避ける
const FIRST_CONTROL_ID: i32 = 100;

/// ダイアログの共有状態。
///
/// wnd_procからは`GWLP_USERDATA`経由で`&DialogState`(共有参照)として
/// アクセスされるため、全フィールドを`Cell`/`RefCell`で持つ。
/// `&mut`を作らないことでエイリアシングによる未定義動作を避けている。
struct DialogState {
    hwnd: Cell<Option<HWND>>,
    parent: Cell<Option<HWND>>,
    accepted: Cell<bool>,
    default_id: Cell<Option<i32>>,
    /// (ウィジェット, そのウィジェットが受け取るコマンドID)の平坦なディスパッチ表
    widgets: RefCell<Vec<WidgetEntry>>,
}

impl DialogState {
    fn new() -> Rc<Self> {
        Rc::new(DialogState {
            hwnd: Cell::new(None),
            parent: Cell::new(None),
            accepted: Cell::new(false),
            default_id: Cell::new(None),
            widgets: RefCell::new(Vec::new()),
        })
    }
}

/// モーダルダイアログ。
///
/// `open`はダイアログが閉じるまでブロックし、`DialogHandle::accept`で
/// 閉じられた場合に`true`を返す。閉じた後も各ウィジェットのgetterから
/// 入力値を読み出せる。
pub struct Dialog {
    state: Rc<DialogState>,
    title: String,
    layout: Box<dyn Layout>,
    font: Option<Font>,
}

/// イベントハンドラからダイアログを操作するためのクローン可能なハンドル
#[derive(Clone)]
pub struct DialogHandle(Rc<DialogState>);

impl DialogHandle {
    /// ダイアログが開いている間はそのHWNDを返す
    pub fn hwnd(&self) -> Option<HWND> {
        self.0.hwnd.get()
    }

    /// 結果を「確定」としてダイアログを閉じる(`open`が`true`を返す)
    pub fn accept(&self) {
        self.0.accepted.set(true);
        self.post_close();
    }

    /// 結果を「キャンセル」としてダイアログを閉じる(`open`が`false`を返す)
    pub fn cancel(&self) {
        self.0.accepted.set(false);
        self.post_close();
    }

    fn post_close(&self) {
        if let Some(hwnd) = self.0.hwnd.get() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl Dialog {
    pub fn new(title: &str) -> Self {
        Dialog {
            state: DialogState::new(),
            title: title.to_string(),
            layout: Box::new(layout::FlexLayout::new()),
            font: None,
        }
    }

    pub fn with_layout<L: Layout + 'static>(mut self, layout: L) -> Self {
        self.layout = Box::new(layout);
        self
    }

    /// 既定のDPI対応フォントの代わりに使用するフォントを指定する
    pub fn with_font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// イベントハンドラへ渡すためのハンドルを取得する
    pub fn handle(&self) -> DialogHandle {
        DialogHandle(Rc::clone(&self.state))
    }

    /// モーダルダイアログを開き、閉じられるまでブロックする。
    /// `accept`で閉じられた場合`Ok(true)`、それ以外(キャンセル・✕ボタン・Esc)は`Ok(false)`。
    pub fn open(&mut self, parent_hwnd: HWND) -> Result<bool> {
        if self.state.hwnd.get().is_some() {
            return Err(DialogError::InvalidOperation(
                "Dialog is already open".into(),
            ));
        }

        let dpi = match unsafe { GetDpiForWindow(parent_hwnd) } {
            0 => 96,
            dpi => dpi,
        };
        let scale = dpi as f32 / BASE_DPI;

        let font = match &self.font {
            Some(font) => font.clone(),
            None => Font::system(dpi)?,
        };

        // レイアウト計算(論理px)
        let mut tree = taffy::TaffyTree::new();
        let measure = MeasureCtx { font: &font, scale };
        let root = self.layout.build(&mut tree, &measure)?;
        tree.compute_layout(
            root,
            taffy::Size {
                width: taffy::AvailableSpace::MaxContent,
                height: taffy::AvailableSpace::MaxContent,
            },
        )?;
        let root_layout = tree.layout(root)?;
        let client_width = (root_layout.size.width * scale).round() as i32;
        let client_height = (root_layout.size.height * scale).round() as i32;

        let window_style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let window_ex_style = WS_EX_DLGMODALFRAME;
        let (width, height) = window_size_for_client(
            client_width,
            client_height,
            window_style,
            window_ex_style,
            dpi,
        );
        let (x, y) = center_position(parent_hwnd, width, height);

        let (hinstance, class_name) = register_window_class()?;

        self.state.accepted.set(false);
        self.state.parent.set(Some(parent_hwnd));
        self.state.default_id.set(None);
        self.state.widgets.borrow_mut().clear();

        unsafe {
            let _ = EnableWindow(parent_hwnd, false);
        }
        // ここから先はどの経路で抜けても、親の再有効化とウィンドウ破棄を保証する
        let _guard = ModalGuard {
            parent: parent_hwnd,
            state: Rc::clone(&self.state),
        };

        unsafe {
            let hwnd = CreateWindowExW(
                window_ex_style,
                PCWSTR(class_name.as_ptr()),
                &HSTRING::from(&self.title),
                window_style,
                x,
                y,
                width,
                height,
                Some(parent_hwnd),
                None,
                Some(hinstance),
                None,
            )?;
            self.state.hwnd.set(Some(hwnd));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::as_ptr(&self.state) as isize);

            disable_resize_menu(hwnd);

            // 子コントロール生成とディスパッチ表の構築
            let mut ctx = CreateCtx::new(
                hwnd,
                &tree,
                scale,
                hinstance,
                font.hfont(),
                FIRST_CONTROL_ID,
            );
            self.layout.create(&mut ctx, (0.0, 0.0))?;
            let (widgets, default_id) = ctx.finish();
            self.state.default_id.set(default_id);
            *self.state.widgets.borrow_mut() = widgets;

            let _ = ShowWindow(hwnd, SW_SHOW);
            // 最初のタブ移動可能なコントロールへフォーカス
            if let Ok(first) = GetNextDlgTabItem(hwnd, None, false) {
                let _ = SetFocus(Some(first));
            }
            let _ = UpdateWindow(hwnd);

            // モーダルメッセージループ
            let mut msg = MSG::default();
            while self.state.hwnd.get().is_some() {
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 == 0 {
                    // ホスト側のWM_QUIT: 自分は終了しつつ、再ポストして外側のループへ伝播させる
                    PostQuitMessage(msg.wParam.0 as i32);
                    break;
                }
                if result.0 == -1 {
                    return Err(DialogError::Win32Error(Error::from_thread()));
                }
                if let Some(dialog_hwnd) = self.state.hwnd.get() {
                    // Tab移動・Enter(既定ボタン)・Escを処理する
                    if IsDialogMessageW(dialog_hwnd, &msg).as_bool() {
                        continue;
                    }
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        self.state.parent.set(None);
        Ok(self.state.accepted.get())
    }
}

/// `open`の全ての脱出経路で親の再有効化と(残っていれば)ウィンドウ破棄を行う
struct ModalGuard {
    parent: HWND,
    state: Rc<DialogState>,
}

impl Drop for ModalGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = EnableWindow(self.parent, true);
            if let Some(hwnd) = self.state.hwnd.get() {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

unsafe extern "system" fn dialog_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const DialogState;
    // GWLP_USERDATA設定前(WM_CREATE等)はnull。
    // Cell/RefCell越しの共有参照としてのみ扱い、&mutは作らない。
    let state = if ptr.is_null() {
        None
    } else {
        Some(unsafe { &*ptr })
    };

    match msg {
        WM_COMMAND => {
            let Some(state) = state else {
                return LRESULT(0);
            };
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            if id == IDCANCEL.0 {
                // IsDialogMessageWがEscキーを変換して送ってくる
                state.accepted.set(false);
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            // ハンドラ実行中に借用を保持しないよう、対象のハンドルを取り出してから呼ぶ
            let target = state
                .widgets
                .borrow()
                .iter()
                .find(|(_, ids)| ids.contains(&id))
                .map(|(widget, _)| Rc::clone(widget));
            if let Some(widget) = target {
                widget.on_command(code);
            }
            LRESULT(0)
        }
        DM_GETDEFID => {
            // Enterキー押下時にIsDialogMessageWが既定ボタンを問い合わせてくる
            if let Some(id) = state.and_then(|s| s.default_id.get()) {
                return LRESULT(((DC_HASDEFID as isize) << 16) | (id as isize & 0xFFFF));
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // フォーカスが他アプリへ飛ばないよう、破棄より先に親を有効化する
            if let Some(state) = state
                && let Some(parent) = state.parent.get()
            {
                unsafe {
                    let _ = EnableWindow(parent, true);
                }
            }
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state) = state {
                // 子ウィンドウはこの後で破棄されるため、先に最終状態をキャッシュする
                for (widget, _) in state.widgets.borrow().iter() {
                    widget.cache_state();
                }
                state.hwnd.set(None);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// このコードが属するモジュール(プラグインDLLまたはEXE)ごとに一意な
/// ウィンドウクラスを登録する。
///
/// `GetModuleHandleW(None)`はホストEXEのハンドルを返すため、複数のプラグイン
/// DLLが同名クラスを共有し、最初に登録したDLLのwnd_procが他のDLLの
/// ダイアログでも使われてしまう。モジュール自身のハンドルを取得し、
/// それをクラス名に含めることで衝突を避ける。
fn register_window_class() -> Result<(HINSTANCE, Vec<u16>)> {
    unsafe {
        let mut module = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(dialog_wnd_proc as *const () as *const u16),
            &mut module,
        )?;
        let hinstance = HINSTANCE(module.0);

        let name = format!("win32_dialog_{:x}", module.0 as usize);
        let mut class_name: Vec<u16> = name.encode_utf16().collect();
        class_name.push(0);

        let mut existing = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            ..Default::default()
        };
        if GetClassInfoExW(Some(hinstance), PCWSTR(class_name.as_ptr()), &mut existing).is_err() {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(dialog_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: HICON::default(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut c_void),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                hIconSm: HICON::default(),
            };
            if RegisterClassExW(&wc) == 0 {
                return Err(DialogError::Win32Error(Error::from_thread()));
            }
        }
        Ok((hinstance, class_name))
    }
}

/// サイズ変更・最大化・最小化をシステムメニューから外す
fn disable_resize_menu(hwnd: HWND) {
    unsafe {
        let hmenu = GetSystemMenu(hwnd, false);
        if !hmenu.is_invalid() {
            let _ = EnableMenuItem(hmenu, SC_SIZE, MF_BYCOMMAND | MF_GRAYED);
            let _ = EnableMenuItem(hmenu, SC_MAXIMIZE, MF_BYCOMMAND | MF_GRAYED);
            let _ = EnableMenuItem(hmenu, SC_MINIMIZE, MF_BYCOMMAND | MF_GRAYED);
        }
    }
}

fn window_size_for_client(
    client_width: i32,
    client_height: i32,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    dpi: u32,
) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn center_position(parent_hwnd: HWND, width: i32, height: i32) -> (i32, i32) {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(parent_hwnd, &mut rect).is_ok() {
            let parent_width = rect.right - rect.left;
            let parent_height = rect.bottom - rect.top;
            (
                rect.left + (parent_width - width) / 2,
                rect.top + (parent_height - height) / 2,
            )
        } else {
            (CW_USEDEFAULT, CW_USEDEFAULT)
        }
    }
}
