use crate::config::{ColorFormat, Config};
use aviutl2::ConfigDialog;
use aviutl2::dialog::{RangedInput, repeat_input};
use win32_ui::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, ComboBox},
};
use windows::Win32::Foundation::HWND;

/// ダイアログの入力欄
#[derive(Clone)]
struct Inputs {
    repeat: RangedInput,
    color: ComboBox,
}

impl Inputs {
    fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: repeat_input(u32::from(u16::MAX), u32::from(default_config.repeat)),
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
            .with_layout(labeled(self.repeat.label(), self.repeat.input().clone()))
            .with_layout(labeled("カラーフォーマット", self.color.clone()))
    }

    /// 入力欄の値を設定へ組む
    ///
    /// # Errors
    /// 読めない欄か値域の外の欄があるとき、画面へ出す文言。
    fn collect(&self) -> std::result::Result<Config, String> {
        let repeat = self.repeat.read()?;

        Ok(Config {
            repeat: repeat as u16,
            color_format: match self.color.selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            },
        })
    }
}

pub fn show_config_dialog(parent_hwnd: HWND, default_config: Config) -> ConfigDialog<Config> {
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

    let Ok(accepted) = dialog.with_layout(layout).open(parent_hwnd) else {
        return ConfigDialog::Failed;
    };
    if !accepted {
        return ConfigDialog::Cancelled;
    }

    match inputs.collect() {
        Ok(config) => ConfigDialog::Accepted(config),
        Err(_) => ConfigDialog::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs {
        Inputs::new(&Config::default())
    }

    /// ループ回数の欄は、GIFが持てる回数の外を文言付きで弾く
    #[test]
    fn the_repeat_field_refuses_values_outside_its_range() {
        let inputs = inputs();
        // 入力欄が実際に検める値域は、NETSCAPE拡張の欄が持てる回数そのもの
        let (min, max) = inputs
            .repeat
            .input()
            .range_bounds()
            .expect("値域を持つ入力欄");
        assert_eq!((min, max), (0, i32::from(u16::MAX)));

        for value in [min - 1, max + 1] {
            inputs.repeat.input().set_value(value);
            let Err(refused) = inputs.collect() else {
                panic!("{value}: 値域の外なので弾かれる");
            };
            assert_eq!(
                refused,
                "ループ回数の値が無効です。0以上の数値を入力してください。"
            );
        }

        inputs.repeat.input().set_value(max);
        assert_eq!(inputs.collect().expect("上限そのもの").repeat, u16::MAX);
    }

    /// ループ回数の欄は、前後に空白のある入力を受け取る
    #[test]
    fn the_repeat_field_accepts_surrounding_whitespace() {
        let inputs = inputs();
        inputs.repeat.input().set_text(" 12 ");

        assert_eq!(inputs.collect().expect("前後の空白").repeat, 12);
    }
}
