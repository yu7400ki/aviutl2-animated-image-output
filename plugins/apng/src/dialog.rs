use crate::config::{ColorFormat, Config, max_threads};
use apng_encoder::COMPRESSION_LEVELS;
use aviutl2::MAX_REPEAT;
use std::ops::RangeInclusive;
use win32_ui::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, ComboBox, Number},
};
use windows::Win32::Foundation::HWND;

/// 入力欄が扱う圧縮レベルの値域
fn compression_range() -> RangeInclusive<i32> {
    *COMPRESSION_LEVELS.start() as i32..=*COMPRESSION_LEVELS.end() as i32
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
    compression: RangedInput,
    threads: RangedInput,
}

impl Inputs {
    fn new(default_config: &Config) -> Self {
        Inputs {
            repeat: Number::new()
                .value(default_config.repeat as i32)
                .range(0, MAX_REPEAT as i32),
            color: ComboBox::new(vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()])
                .selected(match default_config.color_format {
                    ColorFormat::Rgb24 => 0,
                    ColorFormat::Rgba32 => 1,
                }),
            compression: RangedInput::new(
                "圧縮レベル",
                compression_range(),
                default_config.compression_level as i32,
            ),
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
            .with_layout(labeled("ループ回数 (0=無限ループ)", self.repeat.clone()))
            .with_layout(labeled("カラーフォーマット", self.color.clone()))
            .with_layout(labeled(
                &self.compression.label(),
                self.compression.input.clone(),
            ))
            .with_layout(labeled(&self.threads.label(), self.threads.input.clone()))
    }

    /// 入力欄の値を設定へ組む
    ///
    /// # Errors
    /// 読めない欄か値域の外の欄があるとき、画面へ出す文言。
    fn collect(&self) -> Result<Config, String> {
        let repeat = self
            .repeat
            .validate()
            .map_err(|_| "ループ回数の値が無効です。0以上の数値を入力してください。".to_string())?;
        let compression_level = self.compression.read()?;
        let threads = self.threads.read()?;

        Ok(Config {
            repeat: repeat as u32,
            color_format: match self.color.selected_index() {
                0 => ColorFormat::Rgb24,
                1 => ColorFormat::Rgba32,
                _ => Default::default(),
            },
            compression_level: compression_level as u32,
            threads: threads as usize,
        })
    }
}

pub fn show_config_dialog(
    parent_hwnd: HWND,
    default_config: Config,
) -> std::result::Result<Option<Config>, ()> {
    let inputs = Inputs::new(&default_config);

    let dialog = Dialog::new("APNG出力設定");
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
    use aviutl2::IniConfig;
    use aviutl2::ini::Ini;

    fn inputs() -> Inputs {
        Inputs::new(&Config::default())
    }

    /// 値域を検める2つの入力欄
    fn ranged_inputs(inputs: &Inputs) -> [RangedInput; 2] {
        [inputs.compression.clone(), inputs.threads.clone()]
    }

    /// 値域の内側の入力は、そのまま設定になる
    #[test]
    fn values_inside_the_range_become_the_config() {
        let inputs = inputs();
        inputs.repeat.set_value(3);
        inputs.color.set_selected_index(1);
        inputs
            .compression
            .input
            .set_value(*COMPRESSION_LEVELS.end() as i32);
        inputs.threads.input.set_value(max_threads() as i32);

        let config = inputs.collect().expect("値域の内側なので組める");

        assert_eq!(config.repeat, 3);
        assert!(config.color_format == ColorFormat::Rgba32);
        assert_eq!(config.compression_level, *COMPRESSION_LEVELS.end());
        assert_eq!(config.threads, max_threads());
    }

    /// 値域を検めるどの欄も、名乗る値域の内側だけを受け取り、外を文言で弾く
    #[test]
    fn every_field_names_the_range_that_is_checked() {
        // 値域の外を打ち込んだ欄で読み出しが止まるため、欄ごとにダイアログを組み直す
        for position in 0..ranged_inputs(&inputs()).len() {
            let inputs = inputs();
            let field = ranged_inputs(&inputs)[position].clone();
            let name = field.name;
            // 画面へ出す文字列を、入力欄が実際に検める値域と突き合わせる
            let (min, max) = field.input.range_bounds().expect("値域を持つ入力欄");
            assert_eq!(field.label(), format!("{name} ({min}-{max})"));

            field.input.set_value(max);
            assert!(inputs.collect().is_ok(), "{name}: 上限そのもの");
            field.input.set_value(min - 1);
            assert!(inputs.collect().is_err(), "{name}: 下限より下");

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

    /// どの数値欄も、前後に空白のある入力を等しく受け取る
    #[test]
    fn every_number_field_accepts_surrounding_whitespace() {
        let inputs = inputs();
        for (name, input) in [
            ("ループ回数", inputs.repeat.clone()),
            ("圧縮レベル", inputs.compression.input.clone()),
            ("スレッド数", inputs.threads.input.clone()),
        ] {
            let (min, _) = input.range_bounds().expect("値域を持つ入力欄");
            input.set_text(&format!(" {min} "));

            assert!(inputs.collect().is_ok(), "{name}: 前後の空白");
        }
    }

    /// 読めないループ回数も、0より小さいループ回数も、受け付ける値を名乗る文言で弾かれる
    #[test]
    fn an_invalid_repeat_count_is_refused() {
        let inputs = inputs();

        for text in ["abc", "", "2147483648", "-1"] {
            inputs.repeat.set_text(text);
            let Err(message) = inputs.collect() else {
                panic!("ループ回数{text:?}は弾かれる");
            };
            assert_eq!(
                message,
                "ループ回数の値が無効です。0以上の数値を入力してください。"
            );
        }
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
}
