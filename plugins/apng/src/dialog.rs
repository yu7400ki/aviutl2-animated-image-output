use crate::config::{ColorFormat, CompressionType, Config, FilterType};
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, CheckBox, ComboBox, Number},
};
use windows::Win32::Foundation::HWND;

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let repeat_input = Number::new()
        .value(default_config.repeat as i32)
        .range(0, i32::MAX);

    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

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

    // アダプティブフィルターが有効な間はフィルター選択を無効化する
    if default_config.adaptive_filter {
        filter_combobox.set_enabled(false);
    }
    let adaptive_filter_checkbox = CheckBox::new("アダプティブフィルター")
        .checked(default_config.adaptive_filter)
        .on_change({
            let filter_combobox = filter_combobox.clone();
            move |checked| filter_combobox.set_enabled(!checked)
        });

    let dialog = Dialog::new("APNG出力設定");
    let handle = dialog.handle();

    // 入力値を検証してからダイアログを閉じる。無効ならダイアログは開いたまま
    let ok_button = Button::primary("OK").on_click({
        let handle = handle.clone();
        let repeat_input = repeat_input.clone();
        move || {
            if repeat_input.get_value::<u32>().is_err() {
                MessageBox::error(
                    handle.hwnd(),
                    "無効な数値です。正しい数値を入力してください。",
                    "エラー",
                );
                return;
            }
            handle.accept();
        }
    });

    let cancel_button = Button::secondary("キャンセル").on_click({
        let handle = handle.clone();
        move || handle.cancel()
    });

    let layout = FlexLayout::column()
        .with_width(SizeValue::Points(300.0))
        .with_padding(15.0)
        .with_gap(10.0)
        .with_layout(labeled("ループ回数 (0=無限ループ)", repeat_input.clone()))
        .with_layout(labeled("カラーフォーマット", color_combobox.clone()))
        .with_layout(labeled("圧縮", compression_combobox.clone()))
        .with_widget(adaptive_filter_checkbox.clone())
        .with_layout(labeled("フィルター", filter_combobox.clone()))
        .with_layout(
            FlexLayout::row()
                .with_gap(10.0)
                .with_padding_rect(0.0, 0.0, 5.0, 0.0)
                .with_justify_content(JustifyContent::End)
                .with_widget(ok_button)
                .with_widget(cancel_button),
        );

    let accepted = dialog
        .with_layout(layout)
        .open(parent_hwnd)
        .map_err(|_| ())?;
    if !accepted {
        return Ok(None);
    }

    // acceptはOKハンドラの検証を通過した場合のみ呼ばれるため、ここでのパースは成功する
    Ok(Some(Config {
        repeat: repeat_input.get_value().map_err(|_| ())?,
        color_format: match color_combobox.selected_index() {
            0 => ColorFormat::Rgb24,
            1 => ColorFormat::Rgba32,
            _ => Default::default(),
        },
        compression_type: match compression_combobox.selected_index() {
            0 => CompressionType::Default,
            1 => CompressionType::Fast,
            2 => CompressionType::Best,
            _ => Default::default(),
        },
        filter_type: match filter_combobox.selected_index() {
            0 => FilterType::None,
            1 => FilterType::Sub,
            2 => FilterType::Up,
            3 => FilterType::Average,
            4 => FilterType::Paeth,
            _ => Default::default(),
        },
        adaptive_filter: adaptive_filter_checkbox.is_checked(),
    }))
}
