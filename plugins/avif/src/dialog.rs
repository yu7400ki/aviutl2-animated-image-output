use crate::config::{ColorFormat, Config, YuvFormat};
use std::sync::{Arc, Mutex};
use win32_dialog::widget::ComboBox;
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue},
    widget::{Button, ButtonEvent, Label, Number},
};
use windows::Win32::Foundation::*;

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let result = Arc::new(Mutex::new(None::<Config>));

    // Create widgets
    let quality_label = Label::new("品質 (0-100)");
    let quality_number = Number::new()
        .value(default_config.quality as i32)
        .range(0, 100);

    let speed_label = Label::new("速度 (0-10)");
    let speed_number = Number::new()
        .value(default_config.speed as i32)
        .range(0, 10);

    let color_label = Label::new("カラーフォーマット");
    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

    let yuv_label = Label::new("YUVフォーマット");
    let yuv_options = vec![
        YuvFormat::Yuv420.into(),
        YuvFormat::Yuv422.into(),
        YuvFormat::Yuv444.into(),
    ];
    let yuv_combobox = ComboBox::new(yuv_options).selected(match default_config.yuv_format {
        YuvFormat::Yuv420 => 0,
        YuvFormat::Yuv422 => 1,
        YuvFormat::Yuv444 => 2,
    });

    let mut dialog = Dialog::new("AVIF出力設定");

    let ok_button = Button::primary("OK").add_event_handler({
        let result = Arc::clone(&result);
        let quality_number = quality_number.clone();
        let speed_number = speed_number.clone();
        let color_combobox = color_combobox.clone();
        let yuv_combobox = yuv_combobox.clone();
        let dialog = dialog.clone();
        move |_: ButtonEvent| {
            let quality = match quality_number.get_value::<u8>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "品質の値が無効です。0-100の値を入力してください。",
                        "エラー",
                    );
                    return;
                }
            };

            let speed = match speed_number.get_value::<u8>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "速度の値が無効です。0-10の値を入力してください。",
                        "エラー",
                    );
                    return;
                }
            };

            let color_format = match color_combobox.get_selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            };

            let yuv_format = match yuv_combobox.get_selected_index() {
                0 => YuvFormat::Yuv420,
                1 => YuvFormat::Yuv422,
                2 => YuvFormat::Yuv444,
                _ => Default::default(),
            };

            if let Ok(mut guard) = result.lock() {
                *guard = Some(Config {
                    quality,
                    speed,
                    color_format,
                    yuv_format,
                    threads: Config::default().threads,
                });
                dialog.close();
            } else {
                MessageBox::error(
                    Some(parent_hwnd),
                    "内部エラー: 設定の保存に失敗しました。",
                    "エラー",
                );
            }
        }
    });

    let cancel_button = Button::secondary("キャンセル").add_event_handler({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });

    // Create layout with sections
    let mut layout = FlexLayout::column()
        .with_width(SizeValue::Points(300.0))
        .with_padding(15.0)
        .with_gap(10.0);

    // Basic Settings Section
    layout = layout
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(quality_label)
                .with_widget(quality_number),
        )
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(speed_label)
                .with_widget(speed_number),
        );

    // Color Format Section (only if RGBA feature is enabled)
    layout = layout.with_layout(
        FlexLayout::column()
            .with_gap(5.0)
            .with_widget(color_label)
            .with_widget(color_combobox),
    );

    // YUV Format Section
    layout = layout.with_layout(
        FlexLayout::column()
            .with_gap(5.0)
            .with_widget(yuv_label)
            .with_widget(yuv_combobox),
    );

    // Buttons Section
    let buttons_section = FlexLayout::row()
        .with_gap(10.0)
        .with_padding_rect(0.0, 0.0, 5.0, 0.0)
        .with_justify_content(JustifyContent::End)
        .with_widget(ok_button)
        .with_widget(cancel_button);

    layout = layout.with_layout(buttons_section);

    dialog = dialog.with_layout(layout);

    match dialog.open(parent_hwnd) {
        Ok(()) => match result.lock() {
            Ok(guard) => Ok(guard.clone()),
            Err(_) => Err(()),
        },
        Err(_) => Err(()),
    }
}
