mod button;
mod checkbox;
mod combobox;
mod label;
mod number;
mod textbox;

pub use button::*;
pub use checkbox::*;
pub use combobox::*;
pub use label::*;
pub use number::*;
pub use textbox::*;

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

use crate::font::Font;
use crate::{DialogError, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// ダイアログに配置できるコントロール。
///
/// 実装型は`Rc<RefCell<..>>`を内包するクローン可能なハンドルであることを想定し、
/// 全メソッドが`&self`で動作する。
pub trait Widget {
    /// taffyノードを構築する(論理px単位)
    fn build_node(&self, tree: &mut taffy::TaffyTree, ctx: &MeasureCtx) -> Result<taffy::NodeId>;

    /// コントロールを生成し、`WM_COMMAND`を受け取るコマンドIDの一覧を返す
    fn create(&self, ctx: &mut CreateCtx, offset: (f32, f32)) -> Result<Vec<i32>>;

    /// `WM_COMMAND`通知(`code`は通知コード = HIWORD(wParam))。
    /// 呼び出し側は借用を一切保持せずに呼ぶため、ここからウィジェット自身の
    /// メソッドを安全に呼び出せる。
    fn on_command(&self, _code: u16) {}

    /// ウィンドウ破棄の直前に呼ばれる。HWNDから最終状態を取り込み、HWNDを手放す。
    /// これによりダイアログが閉じた後も各getterがユーザーの入力値を返せる。
    fn cache_state(&self);

    fn hwnd(&self) -> Option<HWND>;
}

/// (ウィジェット, そのウィジェットが受け取るコマンドID)の組
pub(crate) type WidgetEntry = (Rc<dyn Widget>, Vec<i32>);

/// レイアウト計算時のコンテキスト
pub struct MeasureCtx<'a> {
    pub font: &'a Font,
    /// 物理px / 論理px (DPIスケール)
    pub scale: f32,
}

impl MeasureCtx<'_> {
    /// テキストサイズを論理pxで返す
    pub fn text_size(&self, text: &str) -> Result<(f32, f32)> {
        let (w, h) = self.font.measure(text)?;
        Ok((w as f32 / self.scale, h as f32 / self.scale))
    }
}

/// コントロールの物理px矩形(ダイアログのクライアント座標)
#[derive(Debug, Clone, Copy)]
pub struct ControlRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// コントロール生成時のコンテキスト。
/// コマンドIDの採番と、ID→ウィジェットのディスパッチ表の収集も担う。
pub struct CreateCtx<'a> {
    parent: HWND,
    tree: &'a taffy::TaffyTree,
    scale: f32,
    hinstance: HINSTANCE,
    font: HFONT,
    next_id: i32,
    widgets: Vec<WidgetEntry>,
    default_id: Option<i32>,
}

impl<'a> CreateCtx<'a> {
    pub(crate) fn new(
        parent: HWND,
        tree: &'a taffy::TaffyTree,
        scale: f32,
        hinstance: HINSTANCE,
        font: HFONT,
        first_id: i32,
    ) -> Self {
        Self {
            parent,
            tree,
            scale,
            hinstance,
            font,
            next_id: first_id,
            widgets: Vec::new(),
            default_id: None,
        }
    }

    pub(crate) fn finish(self) -> (Vec<WidgetEntry>, Option<i32>) {
        (self.widgets, self.default_id)
    }

    pub(crate) fn register(&mut self, widget: Rc<dyn Widget>, ids: Vec<i32>) {
        self.widgets.push((widget, ids));
    }

    pub fn parent(&self) -> HWND {
        self.parent
    }

    pub fn tree(&self) -> &'a taffy::TaffyTree {
        self.tree
    }

    /// このダイアログ内で一意なコマンドIDを採番する(16bitに収まる)
    pub fn alloc_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Enterキーで押される既定ボタンを登録する(最初の1つが有効)
    pub fn set_default_button(&mut self, id: i32) {
        if self.default_id.is_none() {
            self.default_id = Some(id);
        }
    }

    /// ノードの矩形を物理px(ダイアログのクライアント座標)で返す。
    /// 端の座標をそれぞれ丸めることで、隣接コントロール間の累積誤差を防ぐ。
    pub fn control_rect(&self, node: taffy::NodeId, offset: (f32, f32)) -> Result<ControlRect> {
        let layout = self.tree.layout(node)?;
        let left = offset.0 + layout.location.x;
        let top = offset.1 + layout.location.y;
        let right = left + layout.size.width;
        let bottom = top + layout.size.height;
        let x = (left * self.scale).round() as i32;
        let y = (top * self.scale).round() as i32;
        Ok(ControlRect {
            x,
            y,
            width: (right * self.scale).round() as i32 - x,
            height: (bottom * self.scale).round() as i32 - y,
        })
    }

    /// 子コントロールを生成し、ダイアログのフォントを適用する共通処理
    pub fn create_control(
        &self,
        class: PCWSTR,
        text: PCWSTR,
        style: WINDOW_STYLE,
        ex_style: WINDOW_EX_STYLE,
        rect: ControlRect,
        id: i32,
    ) -> Result<HWND> {
        unsafe {
            let hwnd = CreateWindowExW(
                ex_style,
                class,
                text,
                WS_CHILD | WS_VISIBLE | style,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                Some(self.parent),
                Some(HMENU(id as usize as *mut c_void)),
                Some(self.hinstance),
                None,
            )?;
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(1)),
            );
            Ok(hwnd)
        }
    }
}

/// `build_node`前に`create`が呼ばれた場合のエラーを組み立てる
pub(crate) fn require_node(node: Option<taffy::NodeId>, widget: &str) -> Result<taffy::NodeId> {
    node.ok_or_else(|| {
        DialogError::InvalidOperation(format!("{widget}: layout has not been built"))
    })
}

/// ウィンドウテキストを長さを問い合わせてから取得する
pub(crate) fn get_window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

/// イベントハンドラを退避してから実行する。
/// 実行中はウィジェットの`RefCell`借用を保持しないため、ハンドラ内から
/// 同じウィジェットのメソッドを呼んでもパニックしない。
pub(crate) fn dispatch_event<Inner, E: Copy>(
    inner: &Rc<RefCell<Inner>>,
    get: impl for<'x> Fn(&'x mut Inner) -> &'x mut Vec<Box<dyn FnMut(E)>>,
    event: E,
) {
    let mut handlers = std::mem::take(get(&mut inner.borrow_mut()));
    for handler in handlers.iter_mut() {
        handler(event);
    }
    let mut guard = inner.borrow_mut();
    // ハンドラ実行中に追加された分を後ろに繋いで戻す
    let added = std::mem::take(get(&mut guard));
    let slot = get(&mut guard);
    *slot = handlers;
    slot.extend(added);
}
