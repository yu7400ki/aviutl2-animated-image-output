use crate::config::{ColorFormat, Config, max_workers};
use std::ops::RangeInclusive;
use webp_encoder::{METHOD_RANGE, QUALITY_RANGE};
use win32_ui::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, CheckBox, ComboBox, Number},
};
use windows::Win32::Foundation::HWND;

/// 入力欄が扱う品質の値域
fn quality_range() -> RangeInclusive<i32> {
    *QUALITY_RANGE.start() as i32..=*QUALITY_RANGE.end() as i32
}

/// 入力欄が扱うメソッドの値域
fn method_range() -> RangeInclusive<i32> {
    i32::from(*METHOD_RANGE.start())..=i32::from(*METHOD_RANGE.end())
}

/// 値域を検める入力欄。
///
/// 画面へ出す項目名も、弾いたときの文言も、読む値の判定も、
/// ここが持つ名前と値域から決まる。
#[derive(Clone)]
struct RangedInput {
    name: &'static str,
    range: RangeInclusive<i32>,
    input: Number,
}

impl RangedInput {
    fn new(name: &'static str, range: RangeInclusive<i32>, value: i32) -> Self {
        let input = Number::new()
            .value(value)
            .range(*range.start(), *range.end());
        RangedInput { name, range, input }
    }

    /// 名乗る値域
    fn span(&self) -> String {
        format!("{}-{}", self.range.start(), self.range.end())
    }

    /// 値域を添えた項目名
    fn label(&self) -> String {
        format!("{} ({})", self.name, self.span())
    }

    /// 値域の外を弾いたことを伝える文言
    fn error(&self) -> String {
        format!(
            "{}の値が無効です。{}の値を入力してください。",
            self.name,
            self.span()
        )
    }

    /// 入力欄の値を読む
    ///
    /// # Errors
    /// 読めない値か値域の外の値のとき、画面へ出す文言。
    fn read(&self) -> Result<i32, String> {
        self.input.validate().map_err(|_| self.error())
    }
}

/// ダイアログの入力欄
#[derive(Clone)]
struct Inputs {
    repeat: Number,
    color: ComboBox,
    lossless: CheckBox,
    quality: RangedInput,
    method: RangedInput,
    workers: RangedInput,
}

impl Inputs {
    fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: Number::new()
                .value(default_config.repeat)
                .range(0, i32::MAX),
            color: ComboBox::new(vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()])
                .selected(match default_config.color_format {
                    ColorFormat::Rgb24 => 0,
                    ColorFormat::Rgba32 => 1,
                }),
            lossless: CheckBox::new("ロスレス圧縮").checked(default_config.lossless),
            quality: RangedInput::new("品質", quality_range(), default_config.quality as i32),
            method: RangedInput::new("メソッド", method_range(), default_config.method as i32),
            workers: RangedInput::new(
                "ワーカー数",
                // 上限は走らせる機械の並列度で決まる
                1..=max_workers() as i32,
                default_config.workers as i32,
            ),
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
            .with_widget(self.lossless.clone())
            .with_layout(labeled(&self.quality.label(), self.quality.input.clone()))
            .with_layout(labeled(&self.method.label(), self.method.input.clone()))
            .with_layout(labeled(&self.workers.label(), self.workers.input.clone()))
    }

    /// 入力欄の値を設定へ組む
    ///
    /// # Errors
    /// 読めない欄か値域の外の欄があるとき、画面へ出す文言。
    fn collect(&self) -> Result<Config, String> {
        let repeat = self
            .repeat
            .get_value::<i32>()
            .map_err(|_| "ループ回数の値が無効です。正しい数値を入力してください。".to_string())?;
        let workers = self.workers.read()?;
        let quality = self.quality.read()?;
        let method = self.method.read()?;

        Ok(Config {
            repeat,
            color_format: match self.color.selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            },
            lossless: self.lossless.is_checked(),
            quality: quality as f32,
            method: method as u8,
            workers: workers as usize,
        })
    }
}

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let inputs = Inputs::new(&default_config);

    let dialog = Dialog::new("WebP出力設定");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs {
        Inputs::new(&Config::default())
    }

    /// 値域を検める3つの入力欄
    fn ranged_inputs(inputs: &Inputs) -> [RangedInput; 3] {
        [
            inputs.quality.clone(),
            inputs.method.clone(),
            inputs.workers.clone(),
        ]
    }

    /// ロスレスでも、品質とメソッドは画面に出ている値がそのまま設定になる
    ///
    /// どちらもロスレスでは画素を動かさず、ファイルサイズと時間を決める。
    #[test]
    fn lossless_keeps_the_quality_and_method_shown_on_the_dialog() {
        let inputs = inputs();
        inputs.lossless.set_checked(true);
        inputs.quality.input.set_value(40);
        inputs.method.input.set_value(2);

        let config = inputs.collect().expect("値域の内側なので組める");

        assert!(config.lossless);
        assert_eq!(config.quality, 40.0);
        assert_eq!(config.method, 2);
    }

    /// 品質とメソッドは、ロスレスでも値域まで検める
    #[test]
    fn an_invalid_quality_or_method_is_refused_even_when_lossless() {
        let inputs = inputs();
        inputs.lossless.set_checked(true);

        inputs.quality.input.set_text("high");
        assert!(inputs.collect().is_err(), "読めない品質");
        inputs.quality.input.set_value(*quality_range().end() + 1);
        assert!(inputs.collect().is_err(), "値域の外の品質");
        inputs.quality.input.set_value(*quality_range().end());

        inputs.method.input.set_value(*method_range().end() + 1);
        assert!(inputs.collect().is_err(), "値域の外のメソッド");
        inputs.method.input.set_value(*method_range().end());

        assert!(inputs.collect().is_ok(), "値域へ戻せば組める");
    }

    /// ワーカー数の値域も、打ち込みに対して効く
    #[test]
    fn an_out_of_range_worker_count_is_refused() {
        let inputs = inputs();
        let range = inputs.workers.range.clone();

        inputs.workers.input.set_value(*range.start() - 1);
        assert!(inputs.collect().is_err(), "下限より下");
        inputs.workers.input.set_value(*range.end() + 1);
        assert!(inputs.collect().is_err(), "上限より上");
        inputs.workers.input.set_value(*range.end());
        assert!(inputs.collect().is_ok(), "上限そのもの");
    }

    /// 項目名も、値域の外を弾いたときの文言も、検める値域をそのまま名乗る
    #[test]
    fn every_field_names_the_range_that_is_checked() {
        for name in ["品質", "メソッド", "ワーカー数"] {
            let inputs = inputs();
            let field = ranged_inputs(&inputs)
                .into_iter()
                .find(|field| field.name == name)
                .expect("名前の一致する欄がある");
            // 画面へ出す文字列を、入力欄が実際に検める値域と突き合わせる
            let (min, max) = field.input.range_bounds().expect("値域を持つ入力欄");
            assert_eq!(field.label(), format!("{name} ({min}-{max})"));

            field.input.set_value(max + 1);
            let Err(message) = inputs.collect() else {
                panic!("{name}: 値域の外なので弾かれる");
            };
            assert_eq!(
                message,
                format!("{name}の値が無効です。{min}-{max}の値を入力してください。")
            );
        }
    }
}
