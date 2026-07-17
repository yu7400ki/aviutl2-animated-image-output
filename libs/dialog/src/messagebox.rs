use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::HSTRING;

/// アイコン種別を指定してメッセージボックスを表示するラッパー
pub struct MessageBox;

impl MessageBox {
    /// エラーメッセージを表示する
    pub fn error(parent: Option<HWND>, message: &str, title: &str) {
        Self::show_message(parent, message, title, MB_OK | MB_ICONERROR);
    }

    /// 警告メッセージを表示する
    pub fn warning(parent: Option<HWND>, message: &str, title: &str) {
        Self::show_message(parent, message, title, MB_OK | MB_ICONWARNING);
    }

    /// 情報メッセージを表示する
    pub fn info(parent: Option<HWND>, message: &str, title: &str) {
        Self::show_message(parent, message, title, MB_OK | MB_ICONINFORMATION);
    }

    /// 任意のフラグでメッセージボックスを表示する
    pub fn show_message(parent: Option<HWND>, message: &str, title: &str, flags: MESSAGEBOX_STYLE) {
        let message_wide = HSTRING::from(message);
        let title_wide = HSTRING::from(title);

        unsafe {
            MessageBoxW(parent, &message_wide, &title_wide, flags);
        }
    }
}
