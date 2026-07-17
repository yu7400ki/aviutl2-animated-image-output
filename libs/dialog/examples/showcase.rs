use std::ffi::c_void;
use std::mem;
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, CheckBox, ComboBox, Number, TextBox},
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
    let text_input = TextBox::new();
    let number_input = Number::new().range(0, 100);
    let check1 = CheckBox::new("オプション1");
    let check2 = CheckBox::new("オプション2").checked(true);
    let combo = ComboBox::new(vec!["選択肢1", "選択肢2", "選択肢3"]);

    let dialog = Dialog::new("ダイアログ");
    let handle = dialog.handle();

    let ok_button = Button::primary("OK").on_click({
        let handle = handle.clone();
        move || handle.accept()
    });
    let cancel_button = Button::secondary("キャンセル").on_click({
        let handle = handle.clone();
        move || handle.cancel()
    });

    let layout = FlexLayout::column()
        .with_width(SizeValue::Points(400.0))
        .with_padding(15.0)
        .with_gap(10.0)
        .with_layout(labeled("テキスト入力:", text_input.clone()))
        .with_layout(labeled("数値入力:", number_input.clone()))
        .with_widget(check1.clone())
        .with_widget(check2.clone())
        .with_layout(labeled("コンボボックス:", combo.clone()))
        .with_layout(
            FlexLayout::row()
                .with_gap(10.0)
                .with_justify_content(JustifyContent::End)
                .with_widget(ok_button)
                .with_widget(cancel_button),
        );

    // ダイアログを表示(閉じるまでブロック)
    match dialog.with_layout(layout).open(hwnd) {
        Ok(true) => {
            // 閉じた後でも各ウィジェットから入力値を読み出せる
            let message = format!(
                "テキスト: {}\n数値: {}\nオプション1: {}\nオプション2: {}\n選択: {}",
                text_input.get_text(),
                number_input.get_text(),
                check1.is_checked(),
                check2.is_checked(),
                combo.selected_text(),
            );
            MessageBox::info(Some(hwnd), &message, "OKで閉じられました");
        }
        Ok(false) => {
            MessageBox::info(Some(hwnd), "キャンセルされました", "結果");
        }
        Err(e) => {
            MessageBox::error(Some(hwnd), &format!("エラー: {e}"), "エラー");
        }
    }
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
                    Some(HMENU(std::ptr::without_provenance_mut(1))),
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
                return Err(Error::from_thread());
            }

            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Ok(())
    }
}
