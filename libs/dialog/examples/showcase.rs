use std::ffi::c_void;
use std::mem;
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, SizeValue},
    widget::{Button, CheckBox, ComboBox, Label, Number, TextBox},
};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

fn loword(dword: u32) -> u16 {
    (dword & 0xFFFF) as u16
}

fn show_dialog(hwnd: HWND) {
    let dialog = Dialog::new("ダイアログ");

    let layout = FlexLayout::column()
        .with_width(SizeValue::Points(400.0))
        .with_widget(Label::new("テキスト入力:"))
        .with_widget(TextBox::new())
        .with_widget(Label::new("数値入力:"))
        .with_widget(Number::new().range(0, 100))
        .with_widget(Label::new("チェックボックス:"))
        .with_widget(CheckBox::new("オプション1"))
        .with_widget(CheckBox::new("オプション2"))
        .with_widget(Label::new("コンボボックス:"))
        .with_widget(ComboBox::new(vec!["選択肢1", "選択肢2", "選択肢3"]))
        .with_widget(Button::new("OK").add_event_handler({
            let hwnd = hwnd.clone();
            move |_| {
                MessageBox::info(Some(hwnd), "OKがクリックされました", "情報");
            }
        }));

    let mut dialog = dialog.with_layout(layout);

    // ダイアログを表示
    let _ = dialog.open(hwnd);
}

// メインウィンドウのプロシージャ
unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            // ボタンを作成
            unsafe {
                let _button = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    w!("ダイアログを開く"),
                    WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    20,
                    20,
                    200,
                    40,
                    Some(hwnd),
                    Some(HMENU(1 as *mut c_void)),
                    None,
                    None,
                );
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let command_id = loword(wparam.0 as u32) as i32;
            if command_id == 1 {
                show_dialog(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;

        // ウィンドウクラスを登録
        let class_name = w!("MainWindow");
        let wc = WNDCLASSEXW {
            cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(hinstance.0),
            hIcon: LoadIconW(None, IDI_APPLICATION)?,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
            hIconSm: HICON::default(),
        };

        RegisterClassExW(&wc);

        // メインウィンドウを作成
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Win32 Dialog Showcase"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            600,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        )?;

        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let _ = UpdateWindow(hwnd);

        // メッセージループ
        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0);
            if result.0 == 0 {
                break; // WM_QUIT
            } else if result.0 == -1 {
                return Err(Error::from_win32());
            }

            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}
