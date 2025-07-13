use crate::config::{ColorFormat, CompressionType, Config, FilterType};
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

    let mut dialog = Dialog::create("APNG出力設定").size(300, 320);

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
    let color_options = vec!["RGB 24bit", "RGBA 32bit"];
    let color_combobox = ComboBox::new(color_options)
        .position(20, 90)
        .size(245, 20)
        .selected(match default_config.color_format {
            ColorFormat::Rgb24 => 0,
            ColorFormat::Rgba32 => 1,
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

    let ok_button = Button::new("OK").position(95, 240).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let repeat_input = repeat_input.clone();
        let color_combobox = color_combobox.clone();
        let compression_combobox = compression_combobox.clone();
        let filter_combobox = filter_combobox.clone();
        move || {
            if let Ok(repeat) = repeat_input.get_value::<u32>() {
                let color_format = match color_combobox.get_selected_index() {
                    0 => ColorFormat::Rgb24,
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
                if let Ok(mut guard) = result.lock() {
                    *guard = Some(Config {
                        repeat,
                        color_format,
                        compression_type,
                        filter_type,
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
        .position(185, 240)
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
