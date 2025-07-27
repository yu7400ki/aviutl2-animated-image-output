use crate::config::{ColorFormat, CompressionType, Config, FilterType};
use std::sync::{Arc, Mutex};
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue},
    widget::{Button, ButtonEvent, ComboBox, Label, Number},
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
        .range(0, i32::MAX);

    let color_label = Label::new("カラーフォーマット");
    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

    let compression_label = Label::new("圧縮");
    let compression_options = vec![
        CompressionType::Default.into(),
        CompressionType::Fast.into(),
        CompressionType::Best.into(),
    ];
    let compression_combobox =
        ComboBox::new(compression_options).selected(match default_config.compression_type {
            CompressionType::Default => 0,
            CompressionType::Fast => 1,
            CompressionType::Best => 2,
        });

    let filter_label = Label::new("フィルター");
    let filter_options = vec![
        FilterType::None.into(),
        FilterType::Sub.into(),
        FilterType::Up.into(),
        FilterType::Average.into(),
        FilterType::Paeth.into(),
    ];
    let filter_combobox =
        ComboBox::new(filter_options).selected(match default_config.filter_type {
            FilterType::None => 0,
            FilterType::Sub => 1,
            FilterType::Up => 2,
            FilterType::Average => 3,
            FilterType::Paeth => 4,
        });

    let mut dialog = Dialog::new("APNG出力設定");

    let ok_button = Button::primary("OK").add_event_handler({
        let result = Arc::clone(&result);
        let repeat_input = repeat_input.clone();
        let color_combobox = color_combobox.clone();
        let compression_combobox = compression_combobox.clone();
        let filter_combobox = filter_combobox.clone();
        let dialog = dialog.clone();
        move |_: ButtonEvent| {
            let repeat = match repeat_input.get_value::<u32>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "無効な数値です。正しい数値を入力してください。",
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
    layout = layout.with_layout(
        FlexLayout::column()
            .with_gap(5.0)
            .with_widget(repeat_label)
            .with_widget(repeat_input),
    );

    // Color Format Section
    layout = layout.with_layout(
        FlexLayout::column()
            .with_gap(5.0)
            .with_widget(color_label)
            .with_widget(color_combobox),
    );

    // Compression Settings Section
    layout = layout
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(compression_label)
                .with_widget(compression_combobox),
        )
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(filter_label)
                .with_widget(filter_combobox),
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
