use crate::config::{ColorFormat, Config};
use aviutl2::dialog::{ConfigInputs, RangedInput, repeat_input};
use win32_ui::{
    layout::{FlexLayout, SizeValue, labeled},
    widget::ComboBox,
};

/// ダイアログの入力欄
#[derive(Clone)]
pub(crate) struct Inputs {
    repeat: RangedInput,
    color: ComboBox,
}

impl ConfigInputs for Inputs {
    type Config = Config;

    fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: repeat_input(Some(u32::from(u16::MAX)), u32::from(default_config.repeat)),
            color: ComboBox::new(vec![
                ColorFormat::Rgb24.label(),
                ColorFormat::Rgba32.label(),
            ])
            .selected(match default_config.color_format {
                ColorFormat::Rgb24 => 0,
                ColorFormat::Rgba32 => 1,
            }),
        }
    }

    fn layout(&self) -> FlexLayout {
        FlexLayout::column()
            .with_width(SizeValue::Points(300.0))
            .with_padding(15.0)
            .with_gap(10.0)
            .with_layout(labeled(self.repeat.label(), self.repeat.input().clone()))
            .with_layout(labeled("カラーフォーマット", self.color.clone()))
    }

    fn collect(&self) -> Result<Config, String> {
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
                format!("ループ回数の値が無効です。{min}-{max}の値を入力してください。")
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
