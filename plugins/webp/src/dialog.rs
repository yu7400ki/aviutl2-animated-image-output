use crate::config::{ColorFormat, Config, max_workers};
use std::ops::RangeInclusive;
use win32_dialog::{
    Dialog, MessageBox,
    layout::{FlexLayout, JustifyContent, SizeValue, labeled},
    widget::{Button, CheckBox, ComboBox, Label, Number},
};
use windows::Win32::Foundation::HWND;

/// ロスレス圧縮のチェックボックスに出す名前
const LOSSLESS_LABEL: &str = "ロスレス圧縮";

/// ロスレス圧縮に添える但し書き
///
/// ロスレスでも品質とメソッドは効く。効く先が画質ではなく圧縮の手間になる。
const LOSSLESS_NOTE: &str = "品質・メソッドは圧縮の手間として効きます (画素は変わりません)";

/// 品質の値域
const QUALITY_RANGE: RangeInclusive<i32> = 0..=100;

/// メソッドの値域
const METHOD_RANGE: RangeInclusive<i32> = 0..=6;

/// 値域を添えた項目名
fn ranged_label(name: &str, range: &RangeInclusive<i32>) -> String {
    format!("{name} ({}-{})", range.start(), range.end())
}

/// 値域の外を弾いたことを伝える文言
fn range_error(name: &str, range: &RangeInclusive<i32>) -> String {
    format!(
        "{name}の値が無効です。{}-{}の値を入力してください。",
        range.start(),
        range.end()
    )
}

/// 値域の内側に収まっている値だけを読む
///
/// 上下ボタンの値域は打ち込みを縛らないので、読む側で値域まで検める。
fn read_in_range(input: &Number, range: &RangeInclusive<i32>) -> Option<i32> {
    input.get_value::<i32>().ok().filter(|n| range.contains(n))
}

/// ダイアログの入力欄
#[derive(Clone)]
struct Inputs {
    repeat: Number,
    color: ComboBox,
    lossless: CheckBox,
    quality: Number,
    method: Number,
    workers: Number,
    /// ワーカー数の上限
    ceiling: i32,
}

impl Inputs {
    fn new(default_config: &Config) -> Self {
        let ceiling = max_workers() as i32;

        Inputs {
            repeat: Number::new()
                .value(default_config.repeat)
                .range(0, i32::MAX),
            color: ComboBox::new(vec![ColorFormat::Rgb24.into(), ColorFormat::Rgba32.into()])
                .selected(match default_config.color_format {
                    ColorFormat::Rgb24 => 0,
                    ColorFormat::Rgba32 => 1,
                }),
            lossless: CheckBox::new(LOSSLESS_LABEL).checked(default_config.lossless),
            quality: Number::new()
                .value(default_config.quality as i32)
                .range(*QUALITY_RANGE.start(), *QUALITY_RANGE.end()),
            method: Number::new()
                .value(default_config.method as i32)
                .range(*METHOD_RANGE.start(), *METHOD_RANGE.end()),
            workers: Number::new()
                .value(default_config.workers as i32)
                .range(1, ceiling),
            ceiling,
        }
    }

    /// ワーカー数の値域
    fn workers_range(&self) -> RangeInclusive<i32> {
        1..=self.ceiling
    }

    /// 設定項目を縦へ並べる
    ///
    /// ロスレス圧縮だけは、名前の下に但し書きを添える。
    fn layout(&self) -> FlexLayout {
        FlexLayout::column()
            .with_width(SizeValue::Points(300.0))
            .with_padding(15.0)
            .with_gap(10.0)
            .with_layout(labeled("ループ回数 (0=無限ループ)", self.repeat.clone()))
            .with_layout(labeled("カラーフォーマット", self.color.clone()))
            .with_layout(
                FlexLayout::column()
                    .with_gap(3.0)
                    .with_widget(self.lossless.clone())
                    .with_widget(Label::new(LOSSLESS_NOTE)),
            )
            .with_layout(labeled(
                &ranged_label("品質", &QUALITY_RANGE),
                self.quality.clone(),
            ))
            .with_layout(labeled(
                &ranged_label("メソッド", &METHOD_RANGE),
                self.method.clone(),
            ))
            .with_layout(labeled(
                &ranged_label("ワーカー数", &self.workers_range()),
                self.workers.clone(),
            ))
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
        let workers = read_in_range(&self.workers, &self.workers_range())
            .ok_or_else(|| range_error("ワーカー数", &self.workers_range()))?;
        let quality = read_in_range(&self.quality, &QUALITY_RANGE)
            .ok_or_else(|| range_error("品質", &QUALITY_RANGE))?;
        let method = read_in_range(&self.method, &METHOD_RANGE)
            .ok_or_else(|| range_error("メソッド", &METHOD_RANGE))?;

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
    use win32_dialog::layout::Layout;

    fn inputs() -> Inputs {
        Inputs::new(&Config::default())
    }

    /// ロスレスでも、品質とメソッドは画面に出ている値がそのまま設定になる
    ///
    /// どちらもロスレスでは圧縮の手間として効く。据え置くと、選びようのない
    /// 最も重い動作点だけで書き出すことになる。
    #[test]
    fn lossless_keeps_the_quality_and_method_shown_on_the_dialog() {
        let inputs = inputs();
        inputs.lossless.set_checked(true);
        inputs.quality.set_value(40);
        inputs.method.set_value(2);

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

        inputs.quality.set_text("high");
        assert!(inputs.collect().is_err(), "読めない品質");
        inputs.quality.set_value(*QUALITY_RANGE.end() + 1);
        assert!(inputs.collect().is_err(), "値域の外の品質");
        inputs.quality.set_value(*QUALITY_RANGE.end());

        inputs.method.set_value(*METHOD_RANGE.end() + 1);
        assert!(inputs.collect().is_err(), "値域の外のメソッド");
        inputs.method.set_value(*METHOD_RANGE.end());

        assert!(inputs.collect().is_ok(), "値域へ戻せば組める");
    }

    /// ワーカー数の値域も、打ち込みに対して効く
    #[test]
    fn an_out_of_range_worker_count_is_refused() {
        let inputs = inputs();

        inputs.workers.set_value(0);
        assert!(inputs.collect().is_err(), "下限より下");
        inputs.workers.set_value(inputs.ceiling + 1);
        assert!(inputs.collect().is_err(), "上限より上");
        inputs.workers.set_value(inputs.ceiling);
        assert!(inputs.collect().is_ok(), "上限そのもの");
    }

    /// 項目名と、値域を弾いたときの文言は同じ値域から作る
    #[test]
    fn the_label_and_the_error_message_name_the_same_range() {
        assert_eq!(ranged_label("品質", &QUALITY_RANGE), "品質 (0-100)");
        assert_eq!(
            range_error("品質", &QUALITY_RANGE),
            "品質の値が無効です。0-100の値を入力してください。"
        );
    }

    /// ロスレスは、品質とメソッドが何に効くかをダイアログへ出す
    ///
    /// 名前だけでは、ロスレスでも両方が効くことが読めない。
    #[test]
    fn the_lossless_setting_shows_what_quality_and_method_do() {
        let texts = inputs().layout().texts();

        let checkbox = texts
            .iter()
            .position(|text| text == LOSSLESS_LABEL)
            .expect("ロスレス圧縮のチェックボックスが要る");
        assert_eq!(
            texts.get(checkbox + 1).map(String::as_str),
            Some(LOSSLESS_NOTE),
            "{texts:?}"
        );

        assert!(LOSSLESS_NOTE.contains("品質"), "{LOSSLESS_NOTE}");
        assert!(LOSSLESS_NOTE.contains("メソッド"), "{LOSSLESS_NOTE}");
    }
}
