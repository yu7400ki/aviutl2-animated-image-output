use crate::config::{ColorFormat, Config};
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
        .value(default_config.repeat)
        .range(0, i32::MAX);

    let color_options = vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()];
    let color_combobox = ComboBox::new(color_options).selected(match default_config.color_format {
        ColorFormat::Rgb24 => 0,
        ColorFormat::Rgba32 => 1,
    });

    let lossless_checkbox = CheckBox::new("ロスレス圧縮").checked(default_config.lossless);

    let quality_input = Number::new()
        .value(default_config.quality as i32)
        .range(0, 100);

    let method_input = Number::new()
        .value(default_config.method as i32)
        .range(0, 6);

    let dialog = Dialog::new("WebP出力設定");
    let handle = dialog.handle();

    // 入力値を検証してからダイアログを閉じる。無効ならダイアログは開いたまま
    // ロスレス時は品質・メソッドを使用しないため検証しない
    let ok_button = Button::primary("OK").on_click({
        let handle = handle.clone();
        let repeat_input = repeat_input.clone();
        let lossless_checkbox = lossless_checkbox.clone();
        let quality_input = quality_input.clone();
        let method_input = method_input.clone();
        move || {
            let owner = handle.hwnd();
            if repeat_input.get_value::<i32>().is_err() {
                MessageBox::error(
                    owner,
                    "ループ回数の値が無効です。正しい数値を入力してください。",
                    "エラー",
                );
                return;
            }
            if !lossless_checkbox.is_checked() {
                if quality_input.get_value::<i32>().is_err() {
                    MessageBox::error(
                        owner,
                        "品質の値が無効です。0-100の値を入力してください。",
                        "エラー",
                    );
                    return;
                }
                if method_input.get_value::<i32>().is_err() {
                    MessageBox::error(
                        owner,
                        "メソッドの値が無効です。0-6の値を入力してください。",
                        "エラー",
                    );
                    return;
                }
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
        .with_widget(lossless_checkbox.clone())
        .with_layout(labeled("品質 (0-100)", quality_input.clone()))
        .with_layout(labeled("メソッド (0-6)", method_input.clone()))
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

    let lossless = lossless_checkbox.is_checked();

    // acceptはOKハンドラの検証を通過した場合のみ呼ばれるため、ここでのパースは成功する
    Ok(Some(Config {
        repeat: repeat_input.get_value().map_err(|_| ())?,
        color_format: match color_combobox.selected_index() {
            0 => ColorFormat::Rgb24,
            1 => ColorFormat::Rgba32,
            _ => Default::default(),
        },
        lossless,
        quality: if lossless {
            100.0
        } else {
            quality_input.get_value::<i32>().map_err(|_| ())? as f32
        },
        method: if lossless {
            6
        } else {
            method_input.get_value::<i32>().map_err(|_| ())? as u8
        },
    }))
}
