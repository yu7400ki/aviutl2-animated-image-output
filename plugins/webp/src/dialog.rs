use crate::config::{ColorFormat, Config, MAX_NUM_PLAYS};
use aviutl2::dialog::{ConfigInputs, RangedInput, repeat_input};
use aviutl2::max_threads;
use std::ops::RangeInclusive;
use webp_encoder::{METHOD_RANGE, QUALITY_RANGE};
use win32_ui::{
    layout::{FlexLayout, SizeValue, labeled},
    widget::{CheckBox, ComboBox},
};

/// 入力欄が扱う品質の値域
fn quality_range() -> RangeInclusive<i32> {
    *QUALITY_RANGE.start() as i32..=*QUALITY_RANGE.end() as i32
}

/// 入力欄が扱うメソッドの値域
fn method_range() -> RangeInclusive<i32> {
    i32::from(*METHOD_RANGE.start())..=i32::from(*METHOD_RANGE.end())
}

/// ダイアログの入力欄
#[derive(Clone)]
pub(crate) struct Inputs {
    repeat: RangedInput,
    color: ComboBox,
    lossless: CheckBox,
    quality: RangedInput,
    method: RangedInput,
    threads: RangedInput,
}

impl Inputs {
    /// 既定の設定を初期値として入力欄を組む
    pub(crate) fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: repeat_input(Some(MAX_NUM_PLAYS), default_config.repeat),
            color: ComboBox::new(vec![
                ColorFormat::Rgb24.label(),
                ColorFormat::Rgba32.label(),
            ])
            .selected(match default_config.color_format {
                ColorFormat::Rgb24 => 0,
                ColorFormat::Rgba32 => 1,
            }),
            lossless: CheckBox::new("ロスレス圧縮").checked(default_config.lossless),
            quality: RangedInput::new("品質", quality_range(), i32::from(default_config.quality)),
            method: RangedInput::new("メソッド", method_range(), default_config.method as i32),
            threads: RangedInput::new(
                "スレッド数",
                // 上限は走らせる機械の並列度で決まる
                1..=max_threads() as i32,
                default_config.threads as i32,
            ),
        }
    }
}

impl ConfigInputs for Inputs {
    type Config = Config;

    fn layout(&self) -> FlexLayout {
        FlexLayout::column()
            .with_width(SizeValue::Points(300.0))
            .with_padding(15.0)
            .with_gap(10.0)
            .with_layout(labeled(self.repeat.label(), self.repeat.input().clone()))
            .with_layout(labeled("カラーフォーマット", self.color.clone()))
            .with_widget(self.lossless.clone())
            .with_layout(labeled(self.quality.label(), self.quality.input().clone()))
            .with_layout(labeled(self.method.label(), self.method.input().clone()))
            .with_layout(labeled(self.threads.label(), self.threads.input().clone()))
    }

    fn collect(&self) -> Result<Config, String> {
        let repeat = self.repeat.read()?;
        let quality = self.quality.read()?;
        let method = self.method.read()?;
        let threads = self.threads.read()?;

        Ok(Config {
            repeat: repeat as u32,
            color_format: match self.color.selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            },
            lossless: self.lossless.is_checked(),
            quality: quality as u8,
            method: method as u8,
            threads: threads as usize,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aviutl2::IniConfig;
    use aviutl2::ini::Ini;
    use win32_ui::widget::Number;

    fn inputs() -> Inputs {
        Inputs::new(&Config::default())
    }

    /// 数値を打ち込む4つの入力欄
    fn number_inputs(inputs: &Inputs) -> [(&'static str, Number); 4] {
        [
            ("ループ回数", inputs.repeat.input().clone()),
            ("品質", inputs.quality.input().clone()),
            ("メソッド", inputs.method.input().clone()),
            ("スレッド数", inputs.threads.input().clone()),
        ]
    }

    /// ロスレスでも、品質とメソッドは画面に出ている値がそのまま設定になる
    ///
    /// どちらもロスレスでは画素を動かさず、ファイルサイズと時間を決める。
    #[test]
    fn lossless_keeps_the_quality_and_method_shown_on_the_dialog() {
        let inputs = inputs();
        inputs.lossless.set_checked(true);
        inputs.quality.input().set_value(40);
        inputs.method.input().set_value(2);

        let config = inputs.collect().expect("値域の内側なので組める");

        assert!(config.lossless);
        assert_eq!(config.quality, 40);
        assert_eq!(config.method, 2);
    }

    /// 品質とメソッドは、ロスレスでも値域まで検める
    #[test]
    fn an_invalid_quality_or_method_is_refused_even_when_lossless() {
        let inputs = inputs();
        inputs.lossless.set_checked(true);

        inputs.quality.input().set_text("high");
        assert!(inputs.collect().is_err(), "読めない品質");
        inputs.quality.input().set_value(*quality_range().end() + 1);
        assert!(inputs.collect().is_err(), "値域の外の品質");
        inputs.quality.input().set_value(*quality_range().end());

        inputs.method.input().set_value(*method_range().end() + 1);
        assert!(inputs.collect().is_err(), "値域の外のメソッド");
        inputs.method.input().set_value(*method_range().end());

        assert!(inputs.collect().is_ok(), "値域へ戻せば組める");
    }

    /// スレッド数の値域も、打ち込みに対して効く
    #[test]
    fn an_out_of_range_worker_count_is_refused() {
        let inputs = inputs();
        // 名乗る値域と、入力欄が実際に検める値域を突き合わせる
        let (min, max) = inputs
            .threads
            .input()
            .range_bounds()
            .expect("値域を持つ入力欄");
        assert_eq!(inputs.threads.label(), format!("スレッド数 ({min}-{max})"));

        inputs.threads.input().set_value(min - 1);
        assert!(inputs.collect().is_err(), "下限より下");
        inputs.threads.input().set_value(max + 1);
        assert!(inputs.collect().is_err(), "上限より上");
        inputs.threads.input().set_value(max);
        assert!(inputs.collect().is_ok(), "上限そのもの");
    }

    /// 無効な欄が複数あるとき、画面の並びで先に来る欄の文言が出る
    #[test]
    fn the_field_shown_first_is_the_one_reported() {
        let inputs = inputs();
        let (_, threads_max) = inputs
            .threads
            .input()
            .range_bounds()
            .expect("値域を持つ入力欄");
        inputs.quality.input().set_text("high");
        inputs.threads.input().set_value(threads_max + 1);

        let Err(message) = inputs.collect() else {
            panic!("どちらの欄も無効なので弾かれる");
        };
        assert!(message.starts_with("品質"), "{message}");
    }

    /// どの数値欄も、前後に空白のある入力を等しく受け取る
    #[test]
    fn every_number_field_accepts_surrounding_whitespace() {
        for name in ["ループ回数", "品質", "メソッド", "スレッド数"] {
            let inputs = inputs();
            let (_, input) = number_inputs(&inputs)
                .into_iter()
                .find(|(field, _)| *field == name)
                .expect("名前の一致する欄がある");

            let (min, _) = input.range_bounds().expect("値域を持つ入力欄");
            input.set_text(&format!(" {min} "));

            assert!(inputs.collect().is_ok(), "{name}: 前後の空白");
        }
    }

    /// ループ回数の欄は、ANIMのループ数欄が持てる回数を名乗る
    #[test]
    fn a_repeat_outside_the_range_is_refused() {
        let inputs = inputs();
        let (min, max) = inputs
            .repeat
            .input()
            .range_bounds()
            .expect("値域を持つ入力欄");
        assert_eq!((min, max), (0, i32::from(u16::MAX)));

        let message = format!("ループ回数の値が無効です。{min}-{max}の値を入力してください。");
        for value in [min - 1, max + 1] {
            inputs.repeat.input().set_value(value);
            let Err(refused) = inputs.collect() else {
                panic!("{value}: 値域の外なので弾かれる");
            };
            assert_eq!(refused, message);
        }
    }

    /// 欄が持てない回数を書いたiniを読み直しても、ダイアログはその値のまま開ける
    #[test]
    fn a_number_of_plays_read_from_the_ini_fits_the_input() {
        let mut ini = Ini::new();
        ini.with_section(Some(Config::SECTION))
            .set("repeat", "3000000000");
        let config = Config::load_from(ini.section(Some(Config::SECTION)));
        assert_eq!(config.repeat, u32::from(u16::MAX));

        let collected = Inputs::new(&config)
            .collect()
            .expect("入力欄が扱える値になっている");

        assert_eq!(collected.repeat, config.repeat);
    }
}
