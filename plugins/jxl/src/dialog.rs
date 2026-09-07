use crate::config::{ColorFormat, Config};
use aviutl2::dialog::{ConfigInputs, RangedInput, repeat_input};
use aviutl2::max_threads;
use jxl_encoder::{EFFORT_RANGE, QUALITY_RANGE};
use std::num::NonZeroUsize;
use std::ops::RangeInclusive;
use win32_ui::{
    layout::{FlexLayout, SizeValue, labeled},
    widget::ComboBox,
};

/// 入力欄が扱う品質の値域
fn quality_range() -> RangeInclusive<i32> {
    *QUALITY_RANGE.start() as i32..=*QUALITY_RANGE.end() as i32
}

/// 入力欄が扱う均衡の値域
fn effort_range() -> RangeInclusive<i32> {
    i32::from(*EFFORT_RANGE.start())..=i32::from(*EFFORT_RANGE.end())
}

/// ダイアログの入力欄
#[derive(Clone)]
pub(crate) struct Inputs {
    repeat: RangedInput,
    color: ComboBox,
    quality: RangedInput,
    effort: RangedInput,
    threads: RangedInput,
}

impl Inputs {
    /// 既定の設定を初期値として入力欄を組む
    pub(crate) fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: repeat_input(None, default_config.repeat),
            color: ComboBox::new(vec![
                ColorFormat::Rgb24.label(),
                ColorFormat::Rgba32.label(),
            ])
            .selected(match default_config.color_format {
                ColorFormat::Rgb24 => 0,
                ColorFormat::Rgba32 => 1,
            }),
            quality: RangedInput::new("品質", quality_range(), i32::from(default_config.quality)),
            effort: RangedInput::new("均衡", effort_range(), i32::from(default_config.effort)),
            threads: RangedInput::new(
                "スレッド数",
                // 上限は走らせる機械の並列度で決まる
                1..=max_threads() as i32,
                default_config.threads.get() as i32,
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
            .with_layout(labeled(self.quality.label(), self.quality.input().clone()))
            .with_layout(labeled(self.effort.label(), self.effort.input().clone()))
            .with_layout(labeled(self.threads.label(), self.threads.input().clone()))
    }

    fn collect(&self) -> Result<Config, String> {
        let repeat = self.repeat.read()?;
        let quality = self.quality.read()?;
        let effort = self.effort.read()?;
        let threads = self.threads.read()?;

        Ok(Config {
            repeat: repeat as u32,
            color_format: match self.color.selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            },
            quality: quality as u8,
            effort: effort as u8,
            threads: NonZeroUsize::new(threads as usize).unwrap_or(NonZeroUsize::MIN),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aviutl2::IniConfig;
    use aviutl2::ini::Ini;

    fn inputs() -> Inputs {
        Inputs::new(&Config::default())
    }

    /// 値域を検める3つの入力欄
    fn ranged_inputs(inputs: &Inputs) -> [RangedInput; 3] {
        [
            inputs.quality.clone(),
            inputs.effort.clone(),
            inputs.threads.clone(),
        ]
    }

    /// 品質と均衡が検める値域は、符号化器の定めるところから来る
    #[test]
    fn the_encoder_decides_the_quality_and_effort_ranges() {
        let inputs = inputs();

        assert_eq!(
            inputs
                .quality
                .input()
                .range_bounds()
                .expect("値域を持つ入力欄"),
            (*QUALITY_RANGE.start() as i32, *QUALITY_RANGE.end() as i32)
        );
        assert_eq!(
            inputs
                .effort
                .input()
                .range_bounds()
                .expect("値域を持つ入力欄"),
            (
                i32::from(*EFFORT_RANGE.start()),
                i32::from(*EFFORT_RANGE.end())
            )
        );
    }

    /// 値域を検める欄は、上下どちらの外側も弾く
    #[test]
    fn every_ranged_field_refuses_both_sides_of_its_range() {
        // 値域の外を打ち込んだ欄で読み出しが止まるため、欄ごとにダイアログを組み直す
        for position in 0..ranged_inputs(&inputs()).len() {
            let inputs = inputs();
            let field = ranged_inputs(&inputs)[position].clone();
            let label = field.label().to_string();
            // 名乗る値域と、入力欄が実際に検める値域を突き合わせる
            let (min, max) = field.input().range_bounds().expect("値域を持つ入力欄");
            assert!(label.ends_with(&format!("({min}-{max})")), "{label}");

            field.input().set_value(min - 1);
            assert!(inputs.collect().is_err(), "{label}: 下限より下");
            field.input().set_value(max + 1);
            assert!(inputs.collect().is_err(), "{label}: 上限より上");
            field.input().set_value(max);
            assert!(inputs.collect().is_ok(), "{label}: 上限そのもの");
        }
    }

    /// 画面に出ている値は、どの欄もそのまま設定になる
    #[test]
    fn every_field_reaches_the_config() {
        // 既定は論理CPU数の半分なので、値域の上端を採る
        let threads = max_threads();
        let inputs = inputs();
        inputs.repeat.input().set_value(7);
        inputs.color.set_selected_index(1);
        inputs.quality.input().set_value(40);
        inputs.effort.input().set_value(2);
        inputs.threads.input().set_value(threads as i32);

        let config = inputs.collect().expect("値域の内側なので組める");

        assert_eq!(config.repeat, 7);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert_eq!(config.quality, 40);
        assert_eq!(config.effort, 2);
        assert_eq!(config.threads.get(), threads);
    }

    /// i32へ折り返す回数を持つiniを読み直しても、ダイアログはその値のまま開ける
    #[test]
    fn a_number_of_plays_read_from_the_ini_fits_the_input() {
        let mut ini = Ini::new();
        ini.with_section(Some(Config::SECTION))
            .set("repeat", "3000000000");
        let config = Config::load_from(ini.section(Some(Config::SECTION)));

        let collected = Inputs::new(&config)
            .collect()
            .expect("入力欄が扱える値になっている");

        assert_eq!(collected.repeat, config.repeat);
    }

    /// ループ回数は0以上を受け取り、弾いたときの文言もそれを名乗る
    #[test]
    fn a_negative_number_of_plays_is_refused() {
        let inputs = inputs();
        inputs.repeat.input().set_value(-1);

        let Err(message) = inputs.collect() else {
            panic!("0より小さいループ回数は弾かれる");
        };
        assert_eq!(
            message,
            "ループ回数の値が無効です。0以上の数値を入力してください。"
        );
    }
}
