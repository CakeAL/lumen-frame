//! 文字格式操作与选区语义。桌面工具栏和命令接口使用同一实现。
use crate::watermark::{
    TextParams, TextStyleChange,
    rich_text::{apply_style, selection_range},
};
use std::ops::Range;

pub fn format_line(
    text: &str,
    style: &mut TextParams,
    selection: Range<usize>,
    change: TextStyleChange,
) {
    let selected = selection_range(text, selection);
    let range = if selected.is_empty() {
        0..text.len()
    } else {
        selected.clone()
    };
    let mut span_change = change.clone();
    if selected.is_empty() {
        match change {
            TextStyleChange::Font(font) => {
                style.font = font.unwrap_or_default();
                span_change = TextStyleChange::Font(None);
            }
            TextStyleChange::Color(color) => {
                style.color = color;
                span_change = TextStyleChange::Color(None);
            }
            TextStyleChange::Bold(value) => style.bold = value,
            TextStyleChange::Italic(value) => style.italic = value,
            TextStyleChange::Underline(value) => style.underline = value,
            TextStyleChange::Reset => {
                style.bold = false;
                style.italic = false;
                style.underline = false;
                style.color = None;
                style.font.clear();
            }
        }
    }
    apply_style(text, &mut style.spans, range, span_change);
}
