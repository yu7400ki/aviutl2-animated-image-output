use super::{Layout, LayoutItem, SizeValue};
use crate::widget::{CreateCtx, MeasureCtx, Widget};
use crate::{DialogError, Result};
use std::rc::Rc;

pub struct FlexLayout {
    style: taffy::Style,
    items: Vec<LayoutItem>,
    node_id: Option<taffy::NodeId>,
}

impl Default for FlexLayout {
    fn default() -> Self {
        Self::new()
    }
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
        Self::new().with_direction(taffy::FlexDirection::Column)
    }

    pub fn row() -> Self {
        Self::new().with_direction(taffy::FlexDirection::Row)
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
        self.items.push(LayoutItem::Widget(Rc::new(widget)));
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

    pub fn with_padding(self, padding: f32) -> Self {
        self.with_padding_rect(padding, padding, padding, padding)
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
    fn build(&mut self, tree: &mut taffy::TaffyTree, ctx: &MeasureCtx) -> Result<taffy::NodeId> {
        let mut child_nodes = Vec::new();

        for item in &mut self.items {
            let node = match item {
                LayoutItem::Layout(layout) => layout.build(tree, ctx),
                LayoutItem::Widget(widget) => widget.build_node(tree, ctx),
            }?;
            child_nodes.push(node);
        }

        let container = tree.new_with_children(self.style.clone(), &child_nodes)?;
        self.node_id = Some(container);
        Ok(container)
    }

    fn create(&self, ctx: &mut CreateCtx, offset: (f32, f32)) -> Result<()> {
        let node_id = self.node_id.ok_or_else(|| {
            DialogError::InvalidOperation("FlexLayout: layout has not been built".into())
        })?;
        let layout = ctx.tree().layout(node_id)?;
        let origin = (offset.0 + layout.location.x, offset.1 + layout.location.y);

        for item in &self.items {
            match item {
                LayoutItem::Widget(widget) => {
                    let ids = widget.create(ctx, origin)?;
                    ctx.register(Rc::clone(widget), ids);
                }
                LayoutItem::Layout(layout) => layout.create(ctx, origin)?,
            }
        }
        Ok(())
    }
}
