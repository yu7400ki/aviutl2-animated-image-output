use crate::config::{ColorFormat, Config};
use dialog::{
    Dialog,
    controls::{Button, ComboBox, Label, Number},
};
use std::sync::{Arc, Mutex};
use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::*,
};

const REPEAT_LABEL_ID: i32 = 1001;
const REPEAT_TEXTBOX_ID: i32 = 1002;
const COLOR_LABEL_ID: i32 = 1003;
const COLOR_COMBOBOX_ID: i32 = 1004;
const OK_BUTTON_ID: i32 = 1005;
const CANCEL_BUTTON_ID: i32 = 1006;

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let result = Arc::new(Mutex::new(None::<Config>));
    let mut dialog = Dialog::new("GIF出力設定", (300, 210));

    // ループ回数ラベル
    let repeat_label = Label::new(
        REPEAT_LABEL_ID,
        (20, 15),
        (245, 20),
        "ループ回数 (0=無限ループ)",
    );
    dialog.add_control(Box::new(repeat_label));

    // ループ回数数値入力
    let number_input = Number::new(
        REPEAT_TEXTBOX_ID,
        (20, 35),
        (245, 20),
        default_config.repeat.into(),
        Some((0, u16::MAX as i32)),
    );
    dialog.add_control(Box::new(number_input.clone()));

    // カラーフォーマットラベル
    let color_label = Label::new(COLOR_LABEL_ID, (20, 70), (245, 20), "カラーフォーマット");
    dialog.add_control(Box::new(color_label));

    // カラーフォーマットコンボボックス
    let color_options = vec!["パレット化", "透明度あり"];
    let color_combobox = ComboBox::new(COLOR_COMBOBOX_ID, (20, 90), (245, 100), color_options);
    let initial_index = match default_config.color_format {
        ColorFormat::Palette => 0,
        ColorFormat::Transparent => 1,
    };
    color_combobox.set_selected_index(initial_index);
    dialog.add_control(Box::new(color_combobox.clone()));

    // OKボタン
    let number_input_ok = number_input.clone();
    let color_combobox_ok = color_combobox.clone();
    let dialog_ok = dialog.clone();
    let result_ok = Arc::clone(&result);

    let mut ok_button = Button::new(OK_BUTTON_ID, (95, 130), (80, 25), "OK");
    ok_button.on_click(move || {
        if let Ok(value) = number_input_ok.get_value::<u16>() {
            if let Ok(mut guard) = result_ok.lock() {
                let color_format = match color_combobox_ok.get_selected_index() {
                    0 => ColorFormat::Palette,
                    1 => ColorFormat::Transparent,
                    _ => ColorFormat::Palette,
                };
                *guard = Some(Config {
                    repeat: value,
                    color_format,
                });
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

    let mut cancel_button = Button::new(CANCEL_BUTTON_ID, (185, 130), (80, 25), "キャンセル");
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
