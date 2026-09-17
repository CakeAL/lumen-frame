//! 页面组合时使用的领域组件。
//!
//! 组件不拥有 `AppView` 的业务真值；它们只呈现状态，或保存控件自身必需的交互状态。

pub(super) mod field;
pub(super) mod gainmap_preview;
pub(super) mod inspector;
pub(super) mod preview;
pub(super) mod queue;
pub(super) mod text_section;
pub(super) mod title_bar;
