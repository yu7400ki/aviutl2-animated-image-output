use crate::config::{ColorFormat, CompressionType, Config, FilterType, KeyColor};
use dialog::{
    Dialog,
    controls::{Button, CheckBox, ComboBox, Label, Number, TextBox},
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

    let mut dialog = Dialog::create("APNG出力設定").size(300, 510);

    // ループ回数設定
    let repeat_label = Label::new("ループ回数 (0=無限ループ)")
        .position(20, 15)
        .size(245, 20);
    let repeat_input = Number::new()
        .position(20, 35)
        .size(245, 20)
        .value(default_config.repeat as i32)
        .range(0, i32::MAX);

    // カラーフォーマット設定
    let color_label = Label::new("カラーフォーマット")
        .position(20, 70)
        .size(245, 20);
    #[cfg(feature = "rgba")]
    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    #[cfg(not(feature = "rgba"))]
    let color_options = vec![ColorFormat::Rgb24.into()];
    let color_combobox = ComboBox::new(color_options)
        .position(20, 90)
        .size(245, 20)
        .selected(match default_config.color_format {
            ColorFormat::Rgb24 => 0,
            #[cfg(feature = "rgba")]
            ColorFormat::Rgba32 => 1,
            #[cfg(not(feature = "rgba"))]
            ColorFormat::Rgba32 => 0,
        });

    // 圧縮設定
    let compression_label = Label::new("圧縮").position(20, 125).size(295, 20);
    let compression_options = vec![
        CompressionType::Default.into(),
        CompressionType::Fast.into(),
        CompressionType::Best.into(),
    ];
    let compression_combobox = ComboBox::new(compression_options)
        .position(20, 145)
        .size(245, 20)
        .selected(match default_config.compression_type {
            CompressionType::Default => 0,
            CompressionType::Fast => 1,
            CompressionType::Best => 2,
        });

    // フィルター設定
    let filter_label = Label::new("フィルター").position(20, 180).size(295, 20);
    let filter_options = vec![
        FilterType::None.into(),
        FilterType::Sub.into(),
        FilterType::Up.into(),
        FilterType::Average.into(),
        FilterType::Paeth.into(),
    ];
    let filter_combobox = ComboBox::new(filter_options)
        .position(20, 200)
        .size(245, 20)
        .selected(match default_config.filter_type {
            FilterType::None => 0,
            FilterType::Sub => 1,
            FilterType::Up => 2,
            FilterType::Average => 3,
            FilterType::Paeth => 4,
        });

    let chroma_key_enabled_checkbox = CheckBox::new("クロマキー透過を有効にする")
        .position(20, 240)
        .size(245, 20)
        .checked(default_config.chroma_key_enabled);

    let chroma_key_color_label = Label::new("基準色（例:#0000FF）")
        .position(20, 270)
        .size(245, 20);

    let chroma_key_color_textbox = TextBox::new()
        .position(20, 290)
        .size(245, 20)
        .text(&default_config.chroma_key_color.to_string())
        .enabled(default_config.chroma_key_enabled);

    let hue_range_label = Label::new("色相範囲（0-360）")
        .position(20, 320)
        .size(245, 20);

    let hue_range_textbox = Number::new()
        .position(20, 340)
        .size(245, 20)
        .value(default_config.chroma_key_hue_range as i32)
        .range(0, 360)
        .enabled(default_config.chroma_key_enabled);

    let saturation_range_label = Label::new("彩度範囲（0-100）")
        .position(20, 370)
        .size(245, 20);

    let saturation_range_textbox = Number::new()
        .position(20, 390)
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

    let ok_button = Button::new("OK").position(95, 430).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let repeat_input = repeat_input.clone();
        let color_combobox = color_combobox.clone();
        let compression_combobox = compression_combobox.clone();
        let filter_combobox = filter_combobox.clone();
        let chroma_key_enabled_checkbox = chroma_key_enabled_checkbox.clone();
        let chroma_key_color_textbox = chroma_key_color_textbox.clone();
        let hue_range_textbox = hue_range_textbox.clone();
        let saturation_range_textbox = saturation_range_textbox.clone();
        move || {
            if let Ok(repeat) = repeat_input.get_value::<u32>() {
                let color_format = match color_combobox.get_selected_index() {
                    0 => ColorFormat::Rgb24,
                    #[cfg(feature = "rgba")]
                    1 => ColorFormat::Rgba32,
                    _ => Default::default(),
                };
                let compression_type = match compression_combobox.get_selected_index() {
                    0 => CompressionType::Default,
                    1 => CompressionType::Fast,
                    2 => CompressionType::Best,
                    _ => Default::default(),
                };
                let filter_type = match filter_combobox.get_selected_index() {
                    0 => FilterType::None,
                    1 => FilterType::Sub,
                    2 => FilterType::Up,
                    3 => FilterType::Average,
                    4 => FilterType::Paeth,
                    _ => Default::default(),
                };

                let chroma_key_enabled = chroma_key_enabled_checkbox.is_checked();

                let chroma_key_target_color =
                    match KeyColor::parse(&chroma_key_color_textbox.get_text()) {
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
                        repeat,
                        color_format,
                        compression_type,
                        filter_type,
                        chroma_key_enabled,
                        chroma_key_color: chroma_key_target_color,
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
                        w!("無効な数値です。正しい数値を入力してください。"),
                        w!("エラー"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            Ok(())
        }
    });

    let cancel_button = Button::new("キャンセル")
        .position(185, 430)
        .size(80, 25)
        .on_click({
            let dialog = dialog.clone();
            move || {
                dialog.close();
                Ok(())
            }
        });

    dialog = dialog
        .with_control(repeat_label)
        .with_control(repeat_input)
        .with_control(color_label)
        .with_control(color_combobox)
        .with_control(compression_label)
        .with_control(compression_combobox)
        .with_control(filter_label)
        .with_control(filter_combobox)
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
