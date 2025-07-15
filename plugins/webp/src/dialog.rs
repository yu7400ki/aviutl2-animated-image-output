use crate::config::{ColorFormat, Config, TargetColor};
use dialog::{
    Dialog,
    controls::{Button, CheckBox, ComboBox, Label, Number, TextBox},
};
use std::sync::{Arc, Mutex};
use widestring;
use windows::{
    Win32::{Foundation::*, UI::WindowsAndMessaging::*},
    core::*,
};

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let result = Arc::new(Mutex::new(None::<Config>));

    let mut dialog = Dialog::create("WebP出力設定").size(300, 520);

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
    let color_options = vec![
        ColorFormat::Rgb24.into(),
        #[cfg(feature = "rgba")]
        ColorFormat::Rgba32.into(),
    ];
    let color_combobox = ComboBox::new(color_options)
        .position(20, 90)
        .size(245, 100)
        .selected(match default_config.color_format {
            ColorFormat::Rgb24 => 0,
            #[cfg(feature = "rgba")]
            ColorFormat::Rgba32 => 1,
            #[cfg(not(feature = "rgba"))]
            ColorFormat::Rgba32 => 0,
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

    let chroma_key_enabled_checkbox = CheckBox::new("クロマキー透過を有効にする")
        .position(20, 255)
        .size(245, 20)
        .checked(default_config.chroma_key_enabled);

    let chroma_key_color_label = Label::new("基準色（例:#0000FF）")
        .position(20, 285)
        .size(245, 20);

    let chroma_key_color_textbox = TextBox::new()
        .position(20, 305)
        .size(245, 20)
        .text(&default_config.chroma_key_target_color.to_string())
        .enabled(default_config.chroma_key_enabled);

    let hue_range_label = Label::new("色相範囲（0-360）")
        .position(20, 335)
        .size(245, 20);

    let hue_range_textbox = Number::new()
        .position(20, 355)
        .size(245, 20)
        .value(default_config.chroma_key_hue_range as i32)
        .range(0, 360)
        .enabled(default_config.chroma_key_enabled);

    let saturation_range_label = Label::new("彩度範囲（0-100）")
        .position(20, 385)
        .size(245, 20);

    let saturation_range_textbox = Number::new()
        .position(20, 405)
        .size(245, 20)
        .value(default_config.chroma_key_saturation_range as i32)
        .range(0, 100)
        .enabled(default_config.chroma_key_enabled);

    let chroma_key_enabled_checkbox = chroma_key_enabled_checkbox.on_change({
        let chroma_key_color_textbox = chroma_key_color_textbox.clone();
        let hue_range_textbox = hue_range_textbox.clone();
        let saturation_range_textbox = saturation_range_textbox.clone();
        move |checked| {
            chroma_key_color_textbox.set_enabled(checked);
            hue_range_textbox.set_enabled(checked);
            saturation_range_textbox.set_enabled(checked);
            Ok(())
        }
    });

    let ok_button = Button::new("OK").position(95, 445).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let number_input = number_input.clone();
        let color_combobox = color_combobox.clone();
        let lossless_checkbox = lossless_checkbox.clone();
        let quality_input = quality_input.clone();
        let method_input = method_input.clone();
        let chroma_key_enabled_checkbox = chroma_key_enabled_checkbox.clone();
        let chroma_key_color_textbox = chroma_key_color_textbox.clone();
        let hue_range_textbox = hue_range_textbox.clone();
        let saturation_range_textbox = saturation_range_textbox.clone();
        move || {
            if let Ok(value) = number_input.get_value::<i32>() {
                let color_format = match color_combobox.get_selected_index() {
                    0 => ColorFormat::Rgb24,
                    #[cfg(feature = "rgba")]
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

                let chroma_key_enabled = chroma_key_enabled_checkbox.is_checked();

                let chroma_key_target_color =
                    match TargetColor::parse(&chroma_key_color_textbox.get_text()) {
                        Ok(color) => color,
                        Err(e) => {
                            unsafe {
                                let error_msg =
                                    widestring::U16CString::from_str(&e).unwrap_or_default();
                                MessageBoxW(
                                    Some(parent_hwnd),
                                    PCWSTR(error_msg.as_ptr()),
                                    w!("エラー"),
                                    MB_OK | MB_ICONERROR,
                                );
                            }
                            return Ok(());
                        }
                    };

                let chroma_key_hue_range = match hue_range_textbox.get_value::<u16>() {
                    Ok(value) => value,
                    Err(_) => {
                        unsafe {
                            MessageBoxW(
                                Some(parent_hwnd),
                                w!("色相範囲の値が無効です。0-360の値を入力してください。"),
                                w!("エラー"),
                                MB_OK | MB_ICONERROR,
                            );
                        }
                        return Ok(());
                    }
                };

                let chroma_key_saturation_range = match saturation_range_textbox.get_value::<u8>() {
                    Ok(value) => value,
                    Err(_) => {
                        unsafe {
                            MessageBoxW(
                                Some(parent_hwnd),
                                w!("彩度範囲の値が無効です。0-100の値を入力してください。"),
                                w!("エラー"),
                                MB_OK | MB_ICONERROR,
                            );
                        }
                        return Ok(());
                    }
                };

                if let Ok(mut guard) = result.lock() {
                    *guard = Some(Config {
                        repeat: value,
                        color_format,
                        lossless,
                        quality,
                        method,
                        chroma_key_enabled,
                        chroma_key_target_color,
                        chroma_key_hue_range,
                        chroma_key_saturation_range,
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
        .position(185, 445)
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
        .with_control(chroma_key_enabled_checkbox)
        .with_control(chroma_key_color_label)
        .with_control(chroma_key_color_textbox)
        .with_control(hue_range_label)
        .with_control(hue_range_textbox)
        .with_control(saturation_range_label)
        .with_control(saturation_range_textbox)
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
