//! 設定ダイアログの入力欄

use std::ops::RangeInclusive;
use win32_ui::widget::Number;

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
        RangedInput {
            label: format!("{name} ({span})"),
            error: format!("{name}の値が無効です。{span}の値を入力してください。"),
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

/// ループ回数の入力欄。上限は書き出す形式が持てる回数で決まる。
pub fn repeat_input(max: u32, value: u32) -> RangedInput {
    let max = i32::try_from(max).unwrap_or(i32::MAX);
    let value = i32::try_from(value).unwrap_or(max);
    RangedInput {
        label: "ループ回数 (0=無限ループ)".to_string(),
        error: "ループ回数の値が無効です。0以上の数値を入力してください。".to_string(),
        input: Number::new().value(value).range(0, max),
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

    /// ループ回数の欄は、値域ではなく0の意味を名乗る
    #[test]
    fn the_repeat_field_names_what_zero_means() {
        let field = repeat_input(MAX_REPEAT, 3);

        assert_eq!(field.label(), "ループ回数 (0=無限ループ)");
        assert_eq!(
            field.input().range_bounds(),
            Some((0, MAX_REPEAT as i32)),
            "受け取る回数の上限"
        );

        field.input().set_value(-1);
        assert_eq!(
            field.read(),
            Err("ループ回数の値が無効です。0以上の数値を入力してください。".to_string())
        );
        field.input().set_text(" 12 ");
        assert_eq!(field.read(), Ok(12), "前後の空白");
    }
}
