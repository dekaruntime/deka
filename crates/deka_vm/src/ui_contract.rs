//! Source attributes shared by checking/lowering and the retained UI adapter.
pub(crate) const SCALAR_ATTRIBUTES: &[&str] = &[
    "id",
    deka_native_ir::CLASS_ATTRIBUTE,
    "value",
    "placeholder",
];
#[cfg(feature = "compiler")]
pub(crate) const EVENT_ATTRIBUTES: &[&str] = &["onClick", "onInput", "onKeyDown"];
