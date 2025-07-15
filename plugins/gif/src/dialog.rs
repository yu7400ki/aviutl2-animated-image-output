use crate::config::{ColorFormat, Config, KeyColor};
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

    let mut dialog = Dialog::create("GIF出力設定").size(300, 440);

    let number_input_label = Label::new("ループ回数 (0=無限ループ)")
        .position(20, 15)
        .size(245, 20);
    let number_input = Number::new()
        .position(20, 35)
        .size(245, 20)
        .value(default_config.repeat as i32)
        .range(0, u16::MAX as i32);

    let speed_input_label = Label::new("パレット生成速度 (1-30)")
        .position(20, 70)
        .size(245, 20);
    let speed_input = Number::new()
        .position(20, 90)
        .size(245, 20)
        .value(default_config.speed as i32)
        .range(1, 30);

    let color_combo_label = Label::new("カラーフォーマット")
        .position(20, 125)
        .size(245, 20);
    let color_options = vec![
        ColorFormat::Rgb24.into(),
        #[cfg(feature = "rgba")]
        ColorFormat::Rgba32.into(),
    ];
    let color_combobox = ComboBox::new(color_options)
        .position(20, 145)
        .size(245, 100)
        .selected(match default_config.color_format {
            ColorFormat::Rgb24 => 0,
            #[cfg(feature = "rgba")]
            ColorFormat::Rgba32 => 1,
            #[cfg(not(feature = "rgba"))]
            ColorFormat::Rgba32 => 0,
        });

    // クロマキー設定
    let chroma_key_enabled_checkbox = CheckBox::new("クロマキー透過を有効にする")
        .position(20, 180)
        .size(245, 20)
        .checked(default_config.chroma_key_enabled);

    let chroma_key_color_label = Label::new("基準色（例:#00FF00）")
        .position(20, 210)
        .size(245, 20);

    let chroma_key_color_textbox = TextBox::new()
        .position(20, 230)
        .size(245, 20)
        .text(&default_config.chroma_key_color.to_string())
        .enabled(default_config.chroma_key_enabled);

    let hue_range_label = Label::new("色相範囲（0-360）")
        .position(20, 260)
        .size(245, 20);

    let hue_range_textbox = Number::new()
        .position(20, 280)
        .size(245, 20)
        .value(default_config.chroma_key_hue_range as i32)
        .range(0, 360)
        .enabled(default_config.chroma_key_enabled);

    let saturation_range_label = Label::new("彩度範囲（0-100）")
        .position(20, 310)
        .size(245, 20);

    let saturation_range_textbox = Number::new()
        .position(20, 330)
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

    let ok_button = Button::new("OK").position(95, 360).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let number_input = number_input.clone();
        let speed_input = speed_input.clone();
        let color_combobox = color_combobox.clone();
        let chroma_key_enabled_checkbox = chroma_key_enabled_checkbox.clone();
        let chroma_key_color_textbox = chroma_key_color_textbox.clone();
        let hue_range_textbox = hue_range_textbox.clone();
        let saturation_range_textbox = saturation_range_textbox.clone();
        move || {
            if let (Ok(repeat_value), Ok(speed_value)) = (
                number_input.get_value::<u16>(),
                speed_input.get_value::<i32>(),
            ) {
                let color_format = match color_combobox.get_selected_index() {
                    0 => ColorFormat::Rgb24,
                    #[cfg(feature = "rgba")]
                    1 => ColorFormat::Rgba32,
                    _ => ColorFormat::Rgb24,
                };

                let chroma_key_enabled = chroma_key_enabled_checkbox.is_checked();

                let chroma_key_color = match KeyColor::parse(&chroma_key_color_textbox.get_text()) {
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
                        repeat: repeat_value,
                        color_format,
                        speed: speed_value,
                        chroma_key_enabled,
                        chroma_key_color,
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
                        w!("無効な数値です。有効な範囲内の整数を入力してください。"),
                        w!("エラー"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            Ok(())
        }
    });

    let cancel_button = Button::new("キャンセル")
        .position(185, 360)
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
        .with_control(speed_input_label)
        .with_control(speed_input)
        .with_control(color_combo_label)
        .with_control(color_combobox)
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
