use crate::config::{ColorFormat, Config, KeyColor};
use std::sync::{Arc, Mutex};
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue},
    widget::{Button, ButtonEvent, CheckBox, CheckBoxEvent, Label, Number, TextBox},
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

    #[cfg(feature = "rgba")]
    let color_label = Label::new("カラーフォーマット");
    #[cfg(feature = "rgba")]
    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    #[cfg(feature = "rgba")]
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

    let chroma_key_enabled_checkbox =
        CheckBox::new("クロマキー透過を有効にする").checked(default_config.chroma_key_enabled);

    let chroma_key_color_label = Label::new("基準色（例:#0000FF）");
    let chroma_key_color_textbox =
        TextBox::new().text(&default_config.chroma_key_color.to_string());

    let hue_range_label = Label::new("色相範囲（0-360）");
    let hue_range_textbox = Number::new()
        .value(default_config.chroma_key_hue_range as i32)
        .range(0, 360);

    let saturation_range_label = Label::new("彩度範囲（0-100）");
    let saturation_range_textbox = Number::new()
        .value(default_config.chroma_key_saturation_range as i32)
        .range(0, 100);

    // Initially set enabled state based on checkbox
    if !default_config.chroma_key_enabled {
        chroma_key_color_textbox.set_enabled(false);
        hue_range_textbox.set_enabled(false);
        saturation_range_textbox.set_enabled(false);
    }

    // Handle checkbox change events
    let chroma_key_enabled_checkbox = chroma_key_enabled_checkbox.add_event_handler({
        let chroma_key_color_textbox = chroma_key_color_textbox.clone();
        let hue_range_textbox = hue_range_textbox.clone();
        let saturation_range_textbox = saturation_range_textbox.clone();
        move |event| match event {
            CheckBoxEvent::Changed(checked) => {
                chroma_key_color_textbox.set_enabled(checked);
                hue_range_textbox.set_enabled(checked);
                saturation_range_textbox.set_enabled(checked);
            }
        }
    });

    let mut dialog = Dialog::new("AVIF出力設定");

    let ok_button = Button::new("OK").add_event_handler({
        let result = Arc::clone(&result);
        let quality_number = quality_number.clone();
        let speed_number = speed_number.clone();
        #[cfg(feature = "rgba")]
        let color_combobox = color_combobox.clone();
        let chroma_key_enabled_checkbox = chroma_key_enabled_checkbox.clone();
        let chroma_key_color_textbox = chroma_key_color_textbox.clone();
        let hue_range_textbox = hue_range_textbox.clone();
        let saturation_range_textbox = saturation_range_textbox.clone();
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

            #[cfg(feature = "rgba")]
            let color_format = match color_combobox.get_selected_index() {
                0 => ColorFormat::Rgb24,
                #[cfg(feature = "rgba")]
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            };

            let chroma_key_enabled = chroma_key_enabled_checkbox.is_checked();

            let chroma_key_color = match KeyColor::parse(&chroma_key_color_textbox.get_text()) {
                Ok(color) => color,
                Err(e) => {
                    MessageBox::error(Some(parent_hwnd), &e, "エラー");
                    return;
                }
            };

            let chroma_key_hue_range = match hue_range_textbox.get_value::<u16>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "色相範囲の値が無効です。0-360の値を入力してください。",
                        "エラー",
                    );
                    return;
                }
            };

            let chroma_key_saturation_range = match saturation_range_textbox.get_value::<u8>() {
                Ok(value) => value,
                Err(_) => {
                    MessageBox::error(
                        Some(parent_hwnd),
                        "彩度範囲の値が無効です。0-100の値を入力してください。",
                        "エラー",
                    );
                    return;
                }
            };

            if let Ok(mut guard) = result.lock() {
                *guard = Some(Config {
                    quality,
                    speed,
                    #[cfg(feature = "rgba")]
                    color_format,
                    #[cfg(not(feature = "rgba"))]
                    color_format: ColorFormat::Rgb24,
                    threads: Config::default().threads,
                    chroma_key_enabled,
                    chroma_key_color,
                    chroma_key_hue_range,
                    chroma_key_saturation_range,
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

    let cancel_button = Button::new("キャンセル").add_event_handler({
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
    #[cfg(feature = "rgba")]
    {
        layout = layout.with_layout(
            FlexLayout::column()
                .with_widget(color_label)
                .with_widget(color_combobox),
        );
    }

    // Chroma Key Section
    layout = layout
        .with_widget(chroma_key_enabled_checkbox)
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(chroma_key_color_label)
                .with_widget(chroma_key_color_textbox),
        )
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(hue_range_label)
                .with_widget(hue_range_textbox),
        )
        .with_layout(
            FlexLayout::column()
                .with_gap(5.0)
                .with_widget(saturation_range_label)
                .with_widget(saturation_range_textbox),
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
