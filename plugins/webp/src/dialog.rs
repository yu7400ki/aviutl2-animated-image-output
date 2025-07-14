use crate::config::{ColorFormat, Config};
use dialog::{
    Dialog,
    controls::{Button, CheckBox, ComboBox, Label, Number},
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

    let mut dialog = Dialog::create("WebP出力設定").size(300, 340);

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
    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options)
        .position(20, 90)
        .size(245, 100)
        .selected(match default_config.color_format {
            ColorFormat::Rgb24 => 0,
            ColorFormat::Rgba32 => 1,
        });

    let quality_label = Label::new("品質 (0-100)").position(20, 155).size(245, 20);
    let quality_input = Number::new()
        .position(20, 175)
        .size(245, 20)
        .value(default_config.quality as i32)
        .range(0, 100)
        .enabled(!default_config.lossless);

    let method_label = Label::new("圧縮方法 (0-6)").position(20, 205).size(245, 20);
    let method_input = Number::new()
        .position(20, 225)
        .size(245, 20)
        .value(default_config.method as i32)
        .range(0, 6)
        .enabled(!default_config.lossless);

    let lossless_checkbox = CheckBox::new("ロスレス圧縮")
        .position(20, 125)
        .size(245, 20)
        .checked(default_config.lossless)
        .on_change({
            let quality_input = quality_input.clone();
            let method_input = method_input.clone();
            move |checked| {
                quality_input.set_enabled(!checked);
                method_input.set_enabled(!checked);
                Ok(())
            }
        });

    let ok_button = Button::new("OK").position(95, 260).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let number_input = number_input.clone();
        let color_combobox = color_combobox.clone();
        let lossless_checkbox = lossless_checkbox.clone();
        let quality_input = quality_input.clone();
        let method_input = method_input.clone();
        move || {
            if let Ok(value) = number_input.get_value::<i32>() {
                let color_format = match color_combobox.get_selected_index() {
                    0 => ColorFormat::Rgb24,
                    1 => ColorFormat::Rgba32,
                    _ => ColorFormat::Rgb24,
                };
                let lossless = lossless_checkbox.is_checked();
                let quality = if lossless {
                    100.0
                } else {
                    quality_input.get_value::<i32>().unwrap_or(75) as f32
                };
                let method = if lossless {
                    6
                } else {
                    method_input.get_value::<i32>().unwrap_or(4) as u8
                };

                if let Ok(mut guard) = result.lock() {
                    *guard = Some(Config {
                        repeat: value,
                        color_format,
                        lossless,
                        quality,
                        method,
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
        .position(185, 260)
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
        .with_control(lossless_checkbox)
        .with_control(quality_label)
        .with_control(quality_input)
        .with_control(method_label)
        .with_control(method_input)
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
