use crate::config::{ColorFormat, Config};
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
    let repeat_label = Label::new("ループ回数 (0=無限ループ)");
    let repeat_input = Number::new()
        .value(default_config.repeat as i32)
        .range(0, u16::MAX as i32);

    let speed_label = Label::new("パレット生成速度 (1-30)");
    let speed_input = Number::new()
        .value(default_config.speed as i32)
        .range(1, 30);

    let color_label = Label::new("カラーフォーマット");
    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

    let mut dialog = Dialog::new("GIF出力設定");

    let ok_button = Button::primary("OK").add_event_handler({
        let result = Arc::clone(&result);
        let repeat_input = repeat_input.clone();
        let speed_input = speed_input.clone();
        let color_combobox = color_combobox.clone();
        let dialog = dialog.clone();
        move |_: ButtonEvent| {
            let repeat = match repeat_input.get_value::<u16>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "ループ回数の値が無効です。正しい数値を入力してください。",
                        "エラー",
                    );
                    return;
                }
            };

            let speed = match speed_input.get_value::<i32>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "パレット生成速度の値が無効です。1-30の値を入力してください。",
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

            if let Ok(mut guard) = result.lock() {
                *guard = Some(Config {
                    repeat,
                    color_format,
                    speed,
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
                .with_widget(repeat_label)
                .with_widget(repeat_input),
        )
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(speed_label)
                .with_widget(speed_input),
        );

    // Color Format Section (only if RGBA feature is enabled)
    layout = layout.with_layout(
        FlexLayout::column()
            .with_widget(color_label)
            .with_widget(color_combobox),
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
