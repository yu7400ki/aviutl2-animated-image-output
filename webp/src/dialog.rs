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

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let result = Arc::new(Mutex::new(None::<Config>));

    let mut dialog = Dialog::create("WebP出力設定").size(300, 210);

    let number_input_label = Label::new("ループ回数 (0=無限ループ)")
        .position(20, 15)
        .size(245, 20);
    let number_input = Number::new()
        .position(20, 35)
        .size(245, 20)
        .value(default_config.repeat as i32)
        .range(0, i32::MAX);

    let color_combo_label = Label::new("カラーフォーマット")
        .position(20, 70)
        .size(245, 20);
    let color_options = vec!["RGB 24bit", "RGBA 32bit"];
    let color_combobox = ComboBox::new(color_options)
        .position(20, 90)
        .size(245, 100)
        .selected(match default_config.color_format {
            ColorFormat::Rgb24 => 0,
            ColorFormat::Rgba32 => 1,
        });

    let ok_button = Button::new("OK").position(95, 130).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let number_input = number_input.clone();
        let color_combobox = color_combobox.clone();
        move || {
            if let Ok(value) = number_input.get_value::<i32>() {
                let color_format = match color_combobox.get_selected_index() {
                    0 => ColorFormat::Rgb24,
                    1 => ColorFormat::Rgba32,
                    _ => ColorFormat::Rgb24,
                };
                if let Ok(mut guard) = result.lock() {
                    *guard = Some(Config {
                        repeat: value,
                        color_format,
                    });
                    dialog.close();
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
        }
    });

    let cancel_button = Button::new("キャンセル")
        .position(185, 130)
        .size(80, 25)
        .on_click({
            let dialog = dialog.clone();
            move || {
                dialog.close();
                Ok(())
            }
        });

    dialog = dialog
        .with_control(number_input_label)
        .with_control(number_input)
        .with_control(color_combo_label)
        .with_control(color_combobox)
        .with_control(ok_button)
        .with_control(cancel_button);

    if dialog.open(parent_hwnd).is_ok() {
        match result.lock() {
            Ok(guard) => Ok(guard.clone()),
            Err(_) => Err(()),
        }
    } else {
        Err(())
    }
}
