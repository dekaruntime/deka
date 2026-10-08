//! Shared wire-node preparation; inputs retain their original kind.
use crate::Node;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "wire", derive(serde::Serialize, serde::Deserialize))]
pub struct WireNode {
    #[cfg_attr(feature = "wire", serde(default))]
    pub tag: String,
    #[cfg_attr(feature = "wire", serde(default))]
    pub classes: String,
    /// Authored scalar attributes retained independently of renderer snapshots.
    #[cfg_attr(feature = "wire", serde(default))]
    pub attributes: std::collections::BTreeMap<String, String>,
    #[cfg_attr(feature = "wire", serde(default))]
    pub text: Option<String>,
    #[cfg_attr(feature = "wire", serde(default))]
    pub handler: Option<usize>,
    #[cfg_attr(feature = "wire", serde(default))]
    pub children: Vec<WireNode>,
}
impl WireNode {
    pub fn style(&self) -> Result<crate::Style> {
        let mut style = if self.text.is_some() {
            crate::Style::default()
        } else {
            // Inputs use the platform editor; their presentation container is
            // a div, while the retained store keeps the original input kind.
            crate::element_style(if matches!(self.tag.as_str(), "input" | "textarea") {
                "div"
            } else {
                &self.tag
            })?
        };
        if matches!(self.tag.as_str(), "input" | "textarea") {
            style.width = crate::Length::Px(240.);
            style.height = crate::Length::Px(if self.tag == "textarea" { 96. } else { 32. });
            style.clip = true;
        }
        crate::apply_classes(&mut style, &self.classes)?;
        Ok(style)
    }
    pub fn into_node(self, id: String) -> Result<Node> {
        let style = self.style()?;
        let children = self
            .children
            .into_iter()
            .enumerate()
            .map(|(i, c)| c.into_node(format!("{id}/{i}")))
            .collect::<Result<_>>()?;
        Ok(Node {
            id,
            style,
            text: self.text,
            on_click: self.handler,
            children,
        })
    }
}
