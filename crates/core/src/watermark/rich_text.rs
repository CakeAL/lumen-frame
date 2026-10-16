//! 富文本样式锚定模板的 UTF-8 字节范围；字段作为整体，解析后继承其样式。

use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
}

/// 可序列化的局部覆盖。未设置的属性继承文字行，不覆盖行级默认值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    #[serde(flatten)]
    pub style: TextStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TextStyleChange {
    Font(Option<String>),
    Color(Option<[u8; 3]>),
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Reset,
}

pub fn style_at(spans: &[TextSpan], offset: usize) -> TextStyle {
    let mut style = TextStyle::default();
    for span in spans
        .iter()
        .filter(|span| span.start <= offset && offset < span.end)
    {
        if span.style.font.is_some() {
            style.font = span.style.font.clone();
        }
        if span.style.color.is_some() {
            style.color = span.style.color;
        }
        if span.style.bold.is_some() {
            style.bold = span.style.bold;
        }
        if span.style.italic.is_some() {
            style.italic = span.style.italic;
        }
        if span.style.underline.is_some() {
            style.underline = span.style.underline;
        }
    }
    style
}

fn push_span(spans: &mut Vec<TextSpan>, start: usize, end: usize, style: TextStyle) {
    if start >= end || style == TextStyle::default() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.end == start
        && last.style == style
    {
        last.end = end;
    } else {
        spans.push(TextSpan { start, end, style });
    }
}

/// 输入组件也采用字节范围。先裁到字符边界，再扩展任何触及的模板字段。
pub fn selection_range(text: &str, range: Range<usize>) -> Range<usize> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end += 1;
    }
    if start >= end {
        return start..start;
    }
    static TOKENS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\{[^{}]+\}").unwrap());
    for token in TOKENS.find_iter(text) {
        if token.start() < end && token.end() > start {
            start = start.min(token.start());
            end = end.max(token.end());
        }
    }
    start..end
}

pub(crate) fn apply_style(
    text: &str,
    spans: &mut Vec<TextSpan>,
    range: Range<usize>,
    change: TextStyleChange,
) {
    let range = selection_range(text, range);
    if range.is_empty() {
        return;
    }
    let mut updated = Vec::new();
    for (offset, ch) in text.char_indices() {
        let mut style = style_at(spans, offset);
        if range.contains(&offset) {
            match &change {
                TextStyleChange::Font(font) => style.font = font.clone(),
                TextStyleChange::Color(color) => style.color = *color,
                TextStyleChange::Bold(bold) => style.bold = Some(*bold),
                TextStyleChange::Italic(italic) => style.italic = Some(*italic),
                TextStyleChange::Underline(underline) => style.underline = Some(*underline),
                TextStyleChange::Reset => style = TextStyle::default(),
            }
        }
        push_span(&mut updated, offset, offset + ch.len_utf8(), style);
    }
    *spans = updated;
}

/// 保留未修改字符的样式；替换内容继承被替换选区起点，插入继承左侧字符。
/// 比较时以 Unicode 字符为单位，不能把中文或 emoji 截在半个 UTF-8 编码中。
pub fn rebase_spans(old: &str, new: &str, spans: &mut Vec<TextSpan>) {
    if old == new {
        return;
    }
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    let inherited = if prefix < old_end {
        style_at(spans, prefix)
    } else {
        old[..prefix]
            .char_indices()
            .last()
            .map(|(offset, _)| style_at(spans, offset))
            .unwrap_or_default()
    };
    let mut updated = Vec::new();
    for (offset, ch) in new.char_indices() {
        let style = if offset < prefix {
            style_at(spans, offset)
        } else if offset >= new_end {
            style_at(spans, old_end + offset - new_end)
        } else {
            inherited.clone()
        };
        push_span(&mut updated, offset, offset + ch.len_utf8(), style);
    }
    *spans = updated;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_styles_preserve_other_properties_and_unicode() {
        let text = "中文🤔ABC";
        let mut spans = vec![];
        apply_style(
            text,
            &mut spans,
            0..10,
            TextStyleChange::Color(Some([255, 0, 0])),
        );
        apply_style(
            text,
            &mut spans,
            6..11,
            TextStyleChange::Font(Some("Menlo".into())),
        );
        assert_eq!(style_at(&spans, 6).color, Some([255, 0, 0]));
        assert_eq!(style_at(&spans, 6).font.as_deref(), Some("Menlo"));
        assert_eq!(style_at(&spans, 10).color, None);
        apply_style(text, &mut spans, 6..10, TextStyleChange::Reset);
        assert_eq!(style_at(&spans, 6), TextStyle::default());
        assert!(
            spans
                .iter()
                .all(|s| text.is_char_boundary(s.start) && text.is_char_boundary(s.end))
        );
    }
    #[test]
    fn edits_move_styles_without_recoloring_neighbors() {
        let mut spans = vec![];
        apply_style(
            "甲乙丙",
            &mut spans,
            3..6,
            TextStyleChange::Color(Some([1, 2, 3])),
        );
        rebase_spans("甲乙丙", "前甲乙丙", &mut spans);
        assert_eq!(spans[0].start, 6);
        rebase_spans("前甲乙丙", "前甲🤔丙", &mut spans);
        assert_eq!(spans[0].start, 6);
        assert_eq!(spans[0].end, 10);
        rebase_spans("前甲🤔丙", "前甲丙", &mut spans);
        assert!(spans.is_empty());
    }
    #[test]
    fn template_fields_are_atomic_and_legacy_params_default_to_plain() {
        assert_eq!(selection_range("ISO{ISO}mm", 5..6), 3..8);
        let params: super::super::TextParams = toml::from_str("size = 0.03").unwrap();
        assert!(params.spans.is_empty());
    }
}
