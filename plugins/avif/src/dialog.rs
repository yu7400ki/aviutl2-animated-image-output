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

    let mut dialog = Dialog::create("AVIF出力設定").size(300, 260);

    let quality_label = Label::new("品質 (0-100)").position(20, 15).size(245, 20);
    let quality_number = Number::new()
        .position(20, 35)
        .size(245, 20)
        .value(default_config.quality as i32)
        .range(0, 100);

    let speed_label = Label::new("速度 (0-10)").position(20, 70).size(245, 20);
    let speed_number = Number::new()
        .position(20, 90)
        .size(245, 20)
        .value(default_config.speed as i32)
        .range(0, 10);

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

    let ok_button = Button::new("OK").position(95, 180).size(80, 25).on_click({
        let result = Arc::clone(&result);
        let dialog = dialog.clone();
        let quality_number = quality_number.clone();
        let speed_number = speed_number.clone();
        let color_combobox = color_combobox.clone();
        move || {
            if let (Ok(quality), Ok(speed)) = (
                quality_number.get_value::<i32>(),
                speed_number.get_value::<i32>(),
            ) {
                if let Ok(mut guard) = result.lock() {
                    let color_format = match color_combobox.get_selected_index() {
                        0 => ColorFormat::Rgb24,
                        #[cfg(feature = "rgba")]
                        1 => ColorFormat::Rgba32,
                        _ => ColorFormat::Rgb24,
                    };
                    *guard = Some(Config {
                        quality: quality.clamp(0, 100) as u8,
                        speed: speed.clamp(0, 10) as u8,
                        color_format,
                        threads: Config::default().threads,
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
                        w!("無効な数値です。"),
                        w!("エラー"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            Ok(())
        }
    });

    let cancel_button = Button::new("キャンセル")
        .position(185, 180)
        .size(80, 25)
        .on_click({
            let dialog = dialog.clone();
            move || {
                dialog.close();
                Ok(())
            }
        });

    dialog = dialog
        .with_control(quality_label)
        .with_control(quality_number)
        .with_control(speed_label)
        .with_control(speed_number)
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
