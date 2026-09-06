use crate::config::{ColorFormat, Config};
use aviutl2::dialog::{RangedInput, repeat_input};
use aviutl2::{ConfigDialog, MAX_REPEAT, max_threads};
use jxl_encoder::{EFFORT_RANGE, QUALITY_RANGE};
use std::ops::RangeInclusive;
use win32_ui::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, ComboBox},
};
use windows::Win32::Foundation::HWND;

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
struct Inputs {
    repeat: RangedInput,
    color: ComboBox,
    quality: RangedInput,
    effort: RangedInput,
    threads: RangedInput,
}

impl Inputs {
    fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: repeat_input(MAX_REPEAT, default_config.repeat),
            color: ComboBox::new(vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()])
                .selected(match default_config.color_format {
                    ColorFormat::Rgb24 => 0,
                    ColorFormat::Rgba32 => 1,
                }),
            quality: RangedInput::new("品質", quality_range(), default_config.quality as i32),
            effort: RangedInput::new("均衡", effort_range(), i32::from(default_config.effort)),
            threads: RangedInput::new(
                "スレッド数",
                // 上限は走らせる機械の並列度で決まる
                1..=max_threads() as i32,
                default_config.threads as i32,
            ),
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
            .with_layout(labeled(self.quality.label(), self.quality.input().clone()))
            .with_layout(labeled(self.effort.label(), self.effort.input().clone()))
            .with_layout(labeled(self.threads.label(), self.threads.input().clone()))
    }

    /// 入力欄の値を設定へ組む
    ///
    /// # Errors
    /// 読めない欄か値域の外の欄があるとき、画面へ出す文言。
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
            quality: quality as f32,
            effort: effort as u8,
            threads: threads as usize,
        })
    }
}

pub fn show_config_dialog(parent_hwnd: HWND, default_config: Config) -> ConfigDialog<Config> {
    let inputs = Inputs::new(&default_config);

    let dialog = Dialog::new("JPEG XL出力設定");
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
    use aviutl2::IniConfig;
    use aviutl2::ini::Ini;
    use win32_ui::widget::Number;

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

    /// 数値を打ち込む4つの入力欄
    fn number_inputs(inputs: &Inputs) -> [(&'static str, Number); 4] {
        [
            ("ループ回数", inputs.repeat.input().clone()),
            ("品質", inputs.quality.input().clone()),
            ("均衡", inputs.effort.input().clone()),
            ("スレッド数", inputs.threads.input().clone()),
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
    /// どの数値欄も、前後に空白のある入力を等しく受け取る
    #[test]
    fn every_number_field_accepts_surrounding_whitespace() {
        for name in ["ループ回数", "品質", "均衡", "スレッド数"] {
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
        assert_eq!(config.quality, 40.0);
        assert_eq!(config.effort, 2);
        assert_eq!(config.threads, threads);
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
