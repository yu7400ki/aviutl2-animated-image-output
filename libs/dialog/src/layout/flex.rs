use super::{Layout, LayoutItem};
use crate::DialogError;
use crate::{Result, layout::SizeValue, widget::Widget};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HFONT;

#[derive(Default)]
pub struct FlexLayout {
    style: taffy::Style,
    items: Vec<LayoutItem>,
    node_id: Option<taffy::NodeId>,
}

impl FlexLayout {
    pub fn new() -> Self {
        let style = taffy::Style {
            display: taffy::Display::Flex,
            flex_direction: taffy::FlexDirection::Column,
            size: taffy::Size {
                width: SizeValue::auto().into(),
                height: SizeValue::auto().into(),
            },
            ..Default::default()
        };
        Self {
            style,
            items: Vec::new(),
            node_id: None,
        }
    }

    pub fn column() -> Self {
        let mut layout = Self::new();
        layout.style.flex_direction = taffy::FlexDirection::Column;
        layout
    }

    pub fn row() -> Self {
        let mut layout = Self::new();
        layout.style.flex_direction = taffy::FlexDirection::Row;
        layout
    }

    pub fn add_item(mut self, item: LayoutItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn with_layout<T: Layout + 'static>(mut self, layout: T) -> Self {
        self.items.push(LayoutItem::Layout(Box::new(layout)));
        self
    }

    pub fn with_widget<T: Widget + 'static>(mut self, widget: T) -> Self {
        self.items.push(LayoutItem::Widget(Box::new(widget)));
        self
    }

    pub fn with_direction(mut self, direction: super::FlexDirection) -> Self {
        self.style.flex_direction = direction;
        self
    }

    pub fn with_justify_content(mut self, justify: super::JustifyContent) -> Self {
        self.style.justify_content = Some(justify);
        self
    }

    pub fn with_align_items(mut self, align: super::AlignItems) -> Self {
        self.style.align_items = Some(align);
        self
    }

    pub fn with_gap(mut self, gap: f32) -> Self {
        self.style.gap = taffy::Size {
            width: taffy::LengthPercentage::length(gap),
            height: taffy::LengthPercentage::length(gap),
        };
        self
    }

    pub fn with_column_gap(mut self, gap: f32) -> Self {
        self.style.gap.width = taffy::LengthPercentage::length(gap);
        self
    }

    pub fn with_row_gap(mut self, gap: f32) -> Self {
        self.style.gap.height = taffy::LengthPercentage::length(gap);
        self
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.style.padding = taffy::Rect {
            left: taffy::LengthPercentage::length(padding),
            right: taffy::LengthPercentage::length(padding),
            top: taffy::LengthPercentage::length(padding),
            bottom: taffy::LengthPercentage::length(padding),
        };
        self
    }

    pub fn with_padding_rect(mut self, left: f32, right: f32, top: f32, bottom: f32) -> Self {
        self.style.padding = taffy::Rect {
            left: taffy::LengthPercentage::length(left),
            right: taffy::LengthPercentage::length(right),
            top: taffy::LengthPercentage::length(top),
            bottom: taffy::LengthPercentage::length(bottom),
        };
        self
    }

    pub fn with_padding_horizontal(mut self, horizontal: f32) -> Self {
        self.style.padding.left = taffy::LengthPercentage::length(horizontal);
        self.style.padding.right = taffy::LengthPercentage::length(horizontal);
        self
    }

    pub fn with_padding_vertical(mut self, vertical: f32) -> Self {
        self.style.padding.top = taffy::LengthPercentage::length(vertical);
        self.style.padding.bottom = taffy::LengthPercentage::length(vertical);
        self
    }

    pub fn with_width(mut self, width: impl Into<super::Dimension>) -> Self {
        self.style.size.width = width.into();
        self
    }

    pub fn with_height(mut self, height: impl Into<super::Dimension>) -> Self {
        self.style.size.height = height.into();
        self
    }

    pub fn with_min_width(mut self, min_width: impl Into<super::Dimension>) -> Self {
        self.style.min_size.width = min_width.into();
        self
    }

    pub fn with_min_height(mut self, min_height: impl Into<super::Dimension>) -> Self {
        self.style.min_size.height = min_height.into();
        self
    }

    pub fn with_max_width(mut self, max_width: impl Into<super::Dimension>) -> Self {
        self.style.max_size.width = max_width.into();
        self
    }

    pub fn with_max_height(mut self, max_height: impl Into<super::Dimension>) -> Self {
        self.style.max_size.height = max_height.into();
        self
    }
}

impl Layout for FlexLayout {
    fn compute(
        &mut self,
        tree: &mut taffy::TaffyTree,
        font: Option<HFONT>,
    ) -> Result<taffy::NodeId> {
        let mut child_nodes = Vec::new();

        for item in &mut self.items {
            let node = match item {
                LayoutItem::Layout(layout) => layout.compute(tree, font),
                LayoutItem::Widget(widget) => widget.create_node(tree, font),
            }?;
            child_nodes.push(node);
        }

        let container = tree
            .new_with_children(self.style.clone(), &child_nodes)
            .unwrap();

        self.node_id = Some(container);

        Ok(container)
    }

    fn create_window(
        &mut self,
        parent: HWND,
        taffy: &taffy::TaffyTree,
        position: (i32, i32),
    ) -> Result<()> {
        let node_id = self.node_id.ok_or_else(|| {
            DialogError::InvalidOperation("Node ID not set for button".to_string())
        })?;
        let layout = taffy.layout(node_id)?;
        let x = position.0 + layout.location.x as i32;
        let y = position.1 + layout.location.y as i32;

        for item in &mut self.items {
            match item {
                LayoutItem::Widget(widget) => widget.create_window(parent, taffy, (x, y))?,
                LayoutItem::Layout(layout) => layout.create_window(parent, taffy, (x, y))?,
            };
        }
        Ok(())
    }

    fn handle_message(
        &mut self,
        msg: u32,
        wparam: windows::Win32::Foundation::WPARAM,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> Option<windows::Win32::Foundation::LRESULT> {
        // 子要素にメッセージを伝播
        for item in &mut self.items {
            let result = match item {
                LayoutItem::Widget(widget) => widget.handle_message(msg, wparam, lparam),
                LayoutItem::Layout(layout) => layout.handle_message(msg, wparam, lparam),
            };
            if let Some(result) = result {
                return Some(result);
            }
        }
        None
    }

    fn apply_font(&self, font: HFONT) {
        for item in &self.items {
            match item {
                LayoutItem::Widget(widget) => widget.set_font(font),
                LayoutItem::Layout(layout) => layout.apply_font(font),
            };
        }
    }
}
