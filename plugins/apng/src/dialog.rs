use crate::config::{ColorFormat, Config};
use apng_encoder::COMPRESSION_LEVELS;
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

    let compression_input = Number::new()
        .value(default_config.compression_level as i32)
        .range(
            *COMPRESSION_LEVELS.start() as i32,
            *COMPRESSION_LEVELS.end() as i32,
        );

    let reduce_color_checkbox = CheckBox::new("色数の最適化").checked(default_config.reduce_color);

    let dialog = Dialog::new("APNG出力設定");
    let handle = dialog.handle();

    // 入力値を検証してからダイアログを閉じる。無効ならダイアログは開いたまま
    let ok_button = Button::primary("OK").on_click({
        let handle = handle.clone();
        let repeat_input = repeat_input.clone();
        let compression_input = compression_input.clone();
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
            if !compression_input
                .get_value::<u32>()
                .is_ok_and(|level| COMPRESSION_LEVELS.contains(&level))
            {
                MessageBox::error(owner, &compression_error_message(), "エラー");
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
        .with_layout(labeled(&compression_label(), compression_input.clone()))
        .with_widget(reduce_color_checkbox.clone())
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
        compression_level: compression_input.get_value().map_err(|_| ())?,
        reduce_color: reduce_color_checkbox.is_checked(),
    }))
}

fn compression_label() -> String {
    format!(
        "圧縮レベル ({}-{})",
        COMPRESSION_LEVELS.start(),
        COMPRESSION_LEVELS.end()
    )
}

fn compression_error_message() -> String {
    format!(
        "圧縮レベルの値が無効です。{}-{}の値を入力してください。",
        COMPRESSION_LEVELS.start(),
        COMPRESSION_LEVELS.end()
    )
}
