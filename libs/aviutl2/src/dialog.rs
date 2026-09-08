//! 設定ダイアログの入力欄と、それを並べたダイアログの表示

use crate::ConfigDialog;
use std::ops::RangeInclusive;
use win32_ui::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent},
    widget::{Button, Number},
};
use windows::Win32::Foundation::HWND;

/// 値域を検める入力欄。
///
/// 画面へ出す項目名も、弾いたときの文言も、読む値の判定も、ここが持つ。
#[derive(Clone)]
pub struct RangedInput {
    label: String,
    error: String,
    input: Number,
}

impl RangedInput {
    /// 項目名と値域から入力欄を組む。
    ///
    /// 画面へ出す項目名も弾いたときの文言も、この値域を名乗る。
    pub fn new(name: &str, range: RangeInclusive<i32>, value: i32) -> Self {
        let span = format!("{}-{}", range.start(), range.end());
        Self::build(
            format!("{name} ({span})"),
            format!("{name}の値が無効です。{span}の値を入力してください。"),
            &range,
            value,
        )
    }

    /// 項目名と弾いたときの文言を与えて、値域を検める入力欄を組む。
    fn build(label: String, error: String, range: &RangeInclusive<i32>, value: i32) -> Self {
        RangedInput {
            label,
            error,
            input: Number::new()
                .value(value)
                .range(*range.start(), *range.end()),
        }
    }

    /// 画面へ出す項目名
    pub fn label(&self) -> &str {
        &self.label
    }

    /// 画面へ並べる入力欄
    pub fn input(&self) -> &Number {
        &self.input
    }

    /// 入力欄の値を読む
    ///
    /// # Errors
    /// 読めない値か値域の外の値のとき、画面へ出す文言。
    pub fn read(&self) -> Result<i32, String> {
        self.input.validate().map_err(|_| self.error.clone())
    }
}

/// ループ回数の入力欄
///
/// `max` は書き出す形式が持てる回数。形式が上限を持つときだけ、項目名も
/// 弾いたときの文言もその値域を名乗る。上限を持たないときは入力欄が扱える
/// 回数まで受け取る。
pub fn repeat_input(max: Option<u32>, value: u32) -> RangedInput {
    let ceiling = max.map_or(i32::MAX, |max| i32::try_from(max).unwrap_or(i32::MAX));
    let value = i32::try_from(value).unwrap_or(ceiling);
    let (label, error) = match max {
        Some(_) => (
            format!("ループ回数 (0-{ceiling}, 0=無限ループ)"),
            format!("ループ回数の値が無効です。0-{ceiling}の値を入力してください。"),
        ),
        None => (
            "ループ回数 (0=無限ループ)".to_string(),
            "ループ回数の値が無効です。0以上の数値を入力してください。".to_string(),
        ),
    };
    RangedInput::build(label, error, &(0..=ceiling), value)
}

/// 設定ダイアログへ並べる入力欄の集まり
pub trait ConfigInputs {
    /// 入力欄が組み上げる設定
    type Config;

    /// 設定項目を並べる
    fn layout(&self) -> FlexLayout;

    /// 入力欄の値を設定へ組む
    ///
    /// # Errors
    /// 読めない欄か値域の外の欄があるとき、画面へ出す文言。
    fn collect(&self) -> Result<Self::Config, String>;
}

/// 入力欄とOK・キャンセルを並べた、`format_name` の出力設定ダイアログを出す
///
/// OKは入力欄がすべて読めるときだけ受け取り、読めない欄があるときは
/// 文言を出してダイアログを開いたまま残す。
pub fn show_config_dialog<I: ConfigInputs + Clone + 'static>(
    parent_hwnd: HWND,
    format_name: &str,
    inputs: I,
) -> ConfigDialog<I::Config> {
    let dialog = Dialog::new(&format!("{format_name}出力設定"));
    let handle = dialog.handle();

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
    use crate::MAX_REPEAT;

    /// 項目名も、値域の外を弾いたときの文言も、検める値域をそのまま名乗る
    #[test]
    fn every_field_names_the_range_that_is_checked() {
        let field = RangedInput::new("品質", 10..=90, 50);
        // 画面へ出す文字列を、入力欄が実際に検める値域と突き合わせる
        let (min, max) = field.input().range_bounds().expect("値域を持つ入力欄");
        assert_eq!(field.label(), format!("品質 ({min}-{max})"));

        let message = format!("品質の値が無効です。{min}-{max}の値を入力してください。");
        field.input().set_value(min - 1);
        assert_eq!(field.read(), Err(message.clone()), "下限より下");
        field.input().set_value(max + 1);
        assert_eq!(field.read(), Err(message.clone()), "上限より上");
        field.input().set_text("high");
        assert_eq!(field.read(), Err(message), "読めない値");

        field.input().set_value(min);
        assert_eq!(field.read(), Ok(min), "下限そのもの");
        field.input().set_value(max);
        assert_eq!(field.read(), Ok(max), "上限そのもの");
    }

    /// 上限を持たない形式のループ回数の欄は、0の意味だけを名乗る
    #[test]
    fn the_repeat_field_of_a_format_without_a_ceiling_only_names_what_zero_means() {
        let field = repeat_input(None, 3);

        assert_eq!(
            field.input().range_bounds(),
            Some((0, MAX_REPEAT as i32)),
            "入力欄が扱える回数の上限"
        );
        assert_eq!(field.label(), "ループ回数 (0=無限ループ)");

        field.input().set_value(-1);
        assert_eq!(
            field.read(),
            Err("ループ回数の値が無効です。0以上の数値を入力してください。".to_string())
        );
        field.input().set_text(" 12 ");
        assert_eq!(field.read(), Ok(12), "前後の空白");
    }

    /// 上限を持つ形式のループ回数の欄は、検める値域に続けて0の意味を名乗る
    #[test]
    fn the_repeat_field_of_a_format_with_a_ceiling_names_its_range() {
        let field = repeat_input(Some(u32::from(u16::MAX)), 3);

        let (min, max) = field.input().range_bounds().expect("値域を持つ入力欄");
        assert_eq!((min, max), (0, 65535), "形式が持てる回数の上限");
        assert_eq!(
            field.label(),
            format!("ループ回数 ({min}-{max}, 0=無限ループ)")
        );

        let message = format!("ループ回数の値が無効です。{min}-{max}の値を入力してください。");
        field.input().set_value(max + 1);
        assert_eq!(field.read(), Err(message.clone()), "上限より上");
        field.input().set_value(min - 1);
        assert_eq!(field.read(), Err(message), "下限より下");
        field.input().set_value(max);
        assert_eq!(field.read(), Ok(max), "上限そのもの");
    }
}
