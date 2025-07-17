mod flex;

pub use flex::FlexLayout;

pub use taffy::{
    AlignItems, AvailableSpace, Dimension, FlexDirection, JustifyContent, LengthPercentage, Size,
};

use crate::{Result, widget::Widget};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HFONT;

/// 基本的なレイアウト要素を表すtrait
pub trait Layout {
    fn compute(
        &mut self,
        tree: &mut taffy::TaffyTree,
        font: Option<HFONT>,
    ) -> Result<taffy::NodeId>;
    fn create_window(
        &mut self,
        parent: HWND,
        taffy: &taffy::TaffyTree,
        position: (i32, i32),
    ) -> Result<()>;
    fn handle_message(
        &mut self,
        msg: u32,
        wparam: windows::Win32::Foundation::WPARAM,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> Option<windows::Win32::Foundation::LRESULT>;
    fn apply_font(&self, font: HFONT);
}

/// Layoutの中に入れられるアイテム（WidgetかLayoutのどちらか）
pub enum LayoutItem {
    Widget(Box<dyn Widget>),
    Layout(Box<dyn Layout>),
}

/// SizeValue provides a more ergonomic API for specifying dimensions
#[derive(Debug, Clone, PartialEq)]
pub enum SizeValue {
    /// Fixed pixel values
    Points(f32),
    /// Percentage values (0.0 to 1.0)
    Percent(f32),
    /// Automatic sizing based on content
    Auto,
}

impl SizeValue {
    /// Create a fixed pixel size
    pub fn points(value: f32) -> Self {
        SizeValue::Points(value)
    }

    /// Create a percentage size
    pub fn percent(value: f32) -> Self {
        assert!(
            value >= 0.0 && value <= 1.0,
            "Percentage must be between 0.0 and 1.0"
        );
        SizeValue::Percent(value)
    }

    /// Create an automatic size
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
