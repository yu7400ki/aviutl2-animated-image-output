use crate::config::{ColorFormat, Config, YuvFormat};
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, ComboBox, Number},
};
use windows::Win32::Foundation::HWND;

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let repeat_input = Number::new()
        .value(default_config.repeat as i32)
        .range(0, i32::MAX);

    let quality_input = Number::new()
        .value(default_config.quality as i32)
        .range(0, 100);

    let speed_input = Number::new()
        .value(default_config.speed as i32)
        .range(0, 10);

    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

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

    let dialog = Dialog::new("AVIF出力設定");
    let handle = dialog.handle();

    // 入力値を検証してからダイアログを閉じる。無効ならダイアログは開いたまま
    let ok_button = Button::primary("OK").on_click({
        let handle = handle.clone();
        let repeat_input = repeat_input.clone();
        let quality_input = quality_input.clone();
        let speed_input = speed_input.clone();
        move || {
            let owner = handle.hwnd();
            if repeat_input.get_value::<u32>().is_err() {
                MessageBox::error(
                    owner,
                    "ループ回数の値が無効です。正しい数値を入力してください。",
                    "エラー",
                );
                return;
            }
            if quality_input.get_value::<u8>().is_err() {
                MessageBox::error(
                    owner,
                    "品質の値が無効です。0-100の値を入力してください。",
                    "エラー",
                );
                return;
            }
            if speed_input.get_value::<u8>().is_err() {
                MessageBox::error(
                    owner,
                    "エンコード速度の値が無効です。0-10の値を入力してください。",
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
        .with_layout(labeled("品質 (0-100)", quality_input.clone()))
        .with_layout(labeled("エンコード速度 (0-10)", speed_input.clone()))
        .with_layout(labeled("カラーフォーマット", color_combobox.clone()))
        .with_layout(labeled("YUVフォーマット", yuv_combobox.clone()))
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
        quality: quality_input.get_value().map_err(|_| ())?,
        speed: speed_input.get_value().map_err(|_| ())?,
        color_format: match color_combobox.selected_index() {
            0 => ColorFormat::Rgb24,
            1 => ColorFormat::Rgba32,
            _ => Default::default(),
        },
        yuv_format: match yuv_combobox.selected_index() {
            0 => YuvFormat::Yuv420,
            1 => YuvFormat::Yuv422,
            2 => YuvFormat::Yuv444,
            _ => Default::default(),
        },
        threads: Config::default().threads,
    }))
}
