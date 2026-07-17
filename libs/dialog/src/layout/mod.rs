mod flex;

pub use flex::FlexLayout;

pub use taffy::{
    AlignItems, AvailableSpace, Dimension, FlexDirection, JustifyContent, LengthPercentage, Size,
};

use crate::Result;
use crate::widget::{CreateCtx, Label, MeasureCtx, Widget};
use std::rc::Rc;

/// レイアウト要素を表すtrait。
/// サイズ・座標はすべて論理px(96DPI基準)で扱い、物理pxへの変換は
/// `CreateCtx`がコントロール生成時に行う。
pub trait Layout {
    /// taffyノードを構築する
    fn build(&mut self, tree: &mut taffy::TaffyTree, ctx: &MeasureCtx) -> Result<taffy::NodeId>;

    /// レイアウト結果に従ってコントロールを生成する。
    /// `offset`は親からの累積位置(論理px)。
    fn create(&self, ctx: &mut CreateCtx, offset: (f32, f32)) -> Result<()>;
}

/// Layoutの中に入れられるアイテム(WidgetかLayoutのどちらか)
pub enum LayoutItem {
    Widget(Rc<dyn Widget>),
    Layout(Box<dyn Layout>),
}

/// 「ラベル+入力コントロール」の縦組みセクションを作る補助関数
pub fn labeled(text: &str, widget: impl Widget + 'static) -> FlexLayout {
    FlexLayout::column()
        .with_gap(5.0)
        .with_widget(Label::new(text))
        .with_widget(widget)
}

/// サイズ指定のためのAPI
#[derive(Debug, Clone, PartialEq)]
pub enum SizeValue {
    /// 固定値(論理px)
    Points(f32),
    /// 親に対する割合(0.0〜1.0)
    Percent(f32),
    /// 内容に応じた自動サイズ
    Auto,
}

impl SizeValue {
    pub fn points(value: f32) -> Self {
        SizeValue::Points(value)
    }

    pub fn percent(value: f32) -> Self {
        assert!(
            (0.0..=1.0).contains(&value),
            "Percentage must be between 0.0 and 1.0"
        );
        SizeValue::Percent(value)
    }

    pub fn auto() -> Self {
        SizeValue::Auto
    }
}

impl From<SizeValue> for Dimension {
    fn from(val: SizeValue) -> Self {
        match val {
            SizeValue::Points(p) => Dimension::length(p),
            SizeValue::Percent(pc) => Dimension::percent(pc),
            SizeValue::Auto => Dimension::auto(),
        }
    }
}

impl From<f32> for SizeValue {
    fn from(value: f32) -> Self {
        SizeValue::Points(value)
    }
}
