use crate::config::Config;
use dialog::{
    Dialog,
    controls::{Button, Label, TextBox},
};
use std::sync::{Arc, Mutex};
use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::*,
};

const LABEL_ID: i32 = 1001;
const TEXTBOX_ID: i32 = 1002;
const OK_BUTTON_ID: i32 = 1003;
const CANCEL_BUTTON_ID: i32 = 1004;

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let result = Arc::new(Mutex::new(None::<Config>));
    let mut dialog = Dialog::new("出力設定", (300, 160));

    // ラベルコントロール
    let label = Label::new(LABEL_ID, (20, 15), (245, 20), "ループ回数 (0=無限ループ)");
    dialog.add_control(Box::new(label));

    // テキストボックス
    let textbox = TextBox::new(
        TEXTBOX_ID,
        (20, 40),
        (245, 20),
        &default_config.repeat.to_string(),
    );
    dialog.add_control(Box::new(textbox.clone()));

    // OKボタン
    let textbox_ok = textbox.clone();
    let dialog_ok = dialog.clone();
    let result_ok = Arc::clone(&result);

    let mut ok_button = Button::new(OK_BUTTON_ID, (95, 80), (80, 25), "OK");
    ok_button.on_click(move || {
        let text = textbox_ok.get_text();
        if let Ok(value) = text.parse::<u32>() {
            if let Ok(mut guard) = result_ok.lock() {
                *guard = Some(Config { repeat: value });
                dialog_ok.close();
            } else {
                unsafe {
                    MessageBoxW(
                        Some(parent_hwnd),
                        w!("内部エラー: 設定の保存に失敗しました。"),
                        w!("エラー"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
        } else {
            unsafe {
                MessageBoxW(
                    Some(parent_hwnd),
                    w!("無効な数値です。0以上の整数を入力してください。"),
                    w!("エラー"),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        Ok(())
    });
    dialog.add_control(Box::new(ok_button));

    // キャンセルボタン
    let dialog_cancel = dialog.clone();

    let mut cancel_button = Button::new(CANCEL_BUTTON_ID, (185, 80), (80, 25), "キャンセル");
    cancel_button.on_click(move || {
        dialog_cancel.close();
        Ok(())
    });
    dialog.add_control(Box::new(cancel_button));

    // ダイアログを表示
    if dialog.show_modal(parent_hwnd).is_ok() {
        match result.lock() {
            Ok(guard) => Ok(guard.clone()),
            Err(_) => Err(()),
        }
    } else {
        Err(())
    }
}
