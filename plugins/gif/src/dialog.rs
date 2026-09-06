use crate::config::{ColorFormat, Config};
use win32_ui::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, ComboBox, Number},
};
use windows::Win32::Foundation::HWND;

/// ダイアログの入力欄
#[derive(Clone)]
struct Inputs {
    repeat: Number,
    color: ComboBox,
}

impl Inputs {
    fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: Number::new()
                .value(default_config.repeat as i32)
                .range(0, u16::MAX as i32),
            color: ComboBox::new(vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()])
                .selected(match default_config.color_format {
                    ColorFormat::Rgb24 => 0,
                    ColorFormat::Rgba32 => 1,
                }),
        }
    }

    /// 設定項目を縦へ並べる
    fn layout(&self) -> FlexLayout {
        FlexLayout::column()
            .with_width(SizeValue::Points(300.0))
            .with_padding(15.0)
            .with_gap(10.0)
            .with_layout(labeled("ループ回数 (0=無限ループ)", self.repeat.clone()))
            .with_layout(labeled("カラーフォーマット", self.color.clone()))
    }

    /// 入力欄の値を設定へ組む
    ///
    /// # Errors
    /// 読めない欄があるとき、画面へ出す文言。
    fn collect(&self) -> std::result::Result<Config, String> {
        let repeat = self
            .repeat
            .get_value::<u16>()
            .map_err(|_| "ループ回数の値が無効です。正しい数値を入力してください。".to_string())?;

        Ok(Config {
            repeat,
            color_format: match self.color.selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            },
        })
    }
}

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let inputs = Inputs::new(&default_config);

    let dialog = Dialog::new("GIF出力設定");
    let handle = dialog.handle();

    // 入力値を検証してからダイアログを閉じる。無効ならダイアログは開いたまま
    let ok_button = Button::primary("OK").on_click({
        let handle = handle.clone();
        let inputs = inputs.clone();
        move || match inputs.collect() {
            Ok(_) => handle.accept(),
            Err(message) => MessageBox::error(handle.hwnd(), &message, "エラー"),
        }
    });

    let cancel_button = Button::secondary("キャンセル").on_click({
        let handle = handle.clone();
        move || handle.cancel()
    });

    let layout = inputs.layout().with_layout(
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

    inputs.collect().map(Some).map_err(|_| ())
}
