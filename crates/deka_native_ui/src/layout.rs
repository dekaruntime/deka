use crate::{Align, Edges, Justify, Length, Node, Style, text};
use fontdue::Font;
use taffy::Overflow;
use taffy::prelude::*;

pub(crate) struct TextMeasure {
    pub text: String,
    pub size: f32,
    pub nowrap: bool,
}
pub(crate) struct Item<'a> {
    pub node: &'a Node,
    pub layout: NodeId,
    pub color: u32,
    pub font_size: f32,
    pub nowrap: bool,
    pub children: Vec<Item<'a>>,
}
fn dimension(value: Length) -> Dimension {
    match value {
        Length::Auto => Dimension::auto(),
        Length::Px(n) => Dimension::length(n),
        Length::Percent(n) => Dimension::percent(n),
    }
}
fn align(value: Align) -> AlignItems {
    match value {
        Align::Start => AlignItems::FlexStart,
        Align::Center => AlignItems::Center,
        Align::End => AlignItems::FlexEnd,
        Align::Stretch => AlignItems::Stretch,
    }
}
fn justify(value: Justify) -> JustifyContent {
    match value {
        Justify::Start => JustifyContent::FlexStart,
        Justify::Center => JustifyContent::Center,
        Justify::End => JustifyContent::FlexEnd,
        Justify::Between => JustifyContent::SpaceBetween,
        Justify::Around => JustifyContent::SpaceAround,
        Justify::Evenly => JustifyContent::SpaceEvenly,
    }
}
fn edges<T: FromLength>(s: Edges) -> taffy::Rect<T> {
    taffy::Rect {
        top: T::from_length(s.top),
        right: T::from_length(s.right),
        bottom: T::from_length(s.bottom),
        left: T::from_length(s.left),
    }
}
fn style(s: &Style) -> taffy::Style {
    taffy::Style {
        display: Display::Flex,
        flex_direction: if s.row {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        },
        flex_wrap: if s.wrap {
            FlexWrap::Wrap
        } else {
            FlexWrap::NoWrap
        },
        flex_grow: s.grow,
        flex_shrink: s.shrink,
        align_items: Some(align(s.align)),
        align_self: s.align_self.map(align),
        justify_content: Some(justify(s.justify)),
        align_content: Some(AlignContent::FlexStart),
        size: Size {
            width: dimension(s.width),
            height: dimension(s.height),
        },
        min_size: Size {
            width: dimension(s.min_width),
            height: dimension(s.min_height),
        },
        max_size: Size {
            width: dimension(s.max_width),
            height: dimension(s.max_height),
        },
        padding: edges(s.padding),
        margin: edges(s.margin),
        gap: Size {
            width: LengthPercentage::length(s.gap_x),
            height: LengthPercentage::length(s.gap_y),
        },
        overflow: taffy::Point {
            x: if s.clip {
                Overflow::Clip
            } else {
                Overflow::Visible
            },
            y: if s.clip {
                Overflow::Clip
            } else {
                Overflow::Visible
            },
        },
        ..Default::default()
    }
}
pub(crate) fn tree<'a>(
    node: &'a Node,
    color: u32,
    size: f32,
    nowrap: bool,
    tree: &mut TaffyTree<TextMeasure>,
) -> Item<'a> {
    let s = &node.style;
    let color = s.color.unwrap_or(color);
    let size = s.font_size.unwrap_or(size);
    let nowrap = s.nowrap.unwrap_or(nowrap);
    let children: Vec<_> = node
        .children
        .iter()
        .map(|n| self::tree(n, color, size, nowrap, tree))
        .collect();
    let layout = if let Some(text) = &node.text {
        tree.new_leaf_with_context(
            style(s),
            TextMeasure {
                text: text.clone(),
                size,
                nowrap,
            },
        )
        .expect("text layout")
    } else {
        tree.new_with_children(
            style(s),
            &children.iter().map(|c| c.layout).collect::<Vec<_>>(),
        )
        .expect("element layout")
    };
    Item {
        node,
        layout,
        color,
        font_size: size,
        nowrap,
        children,
    }
}
pub(crate) fn measure(
    font: &Font,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<&mut TextMeasure>,
) -> Size<f32> {
    let Some(c) = context else {
        return Size::ZERO;
    };
    let width = if c.nowrap {
        None
    } else {
        known.width.or(match available.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent => Some(text::min_width(font, &c.text, c.size)),
            AvailableSpace::MaxContent => None,
        })
    };
    let layout = text::layout(font, &c.text, c.size, width);
    Size {
        width: known
            .width
            .unwrap_or_else(|| text::width(font, &layout, c.size)),
        height: known.height.unwrap_or_else(|| layout.height()),
    }
}

/// Adjacent bare text fragments are one run, including interpolation boundaries.
/// Element boundaries remain real boxes; this is not HTML inline formatting.
pub(crate) fn normalize(node: &Node) -> Node {
    let mut result = node.clone();
    result.children.clear();
    for child in &node.children {
        let child = normalize(child);
        if let Some(last) = result.children.last_mut()
            && last.text.is_some()
            && child.text.is_some()
            && last.on_click.is_none()
            && child.on_click.is_none()
            && last.style == child.style
            && last.children.is_empty()
            && child.children.is_empty()
        {
            last.text
                .as_mut()
                .unwrap()
                .push_str(child.text.as_ref().unwrap());
        } else {
            result.children.push(child);
        }
    }
    result
}
