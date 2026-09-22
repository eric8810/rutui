//! Theme-aware markdown style in the shape `rutui-markdown` renders.
//!
//! W4 decoupling: moved from the pager's `theme.rs` adapter module. The
//! framework (`rutui-theme`) owns the `MarkdownStyle` *structure*; this
//! module converts it field-for-field into `rutui_markdown`'s identical
//! shape for the real renderer.

use rutui_theme::theme::md_style::MarkdownStyle;

/// Convert the framework's markdown style to `rutui_markdown`'s
/// identical shape (field-for-field, no behavior change).
pub fn into_grok_markdown_style(style: MarkdownStyle) -> rutui_markdown::MarkdownStyle {
    rutui_markdown::MarkdownStyle {
        heading_inner: style.heading_inner,
        heading_outer: style.heading_outer,
        strong_inner: style.strong_inner,
        strong_outer: style.strong_outer,
        emphasis_inner: style.emphasis_inner,
        emphasis_outer: style.emphasis_outer,
        strikethrough_inner: style.strikethrough_inner,
        strikethrough_outer: style.strikethrough_outer,
        inline_code_inner: style.inline_code_inner,
        inline_code_outer: style.inline_code_outer,
        blockquote_outer: style.blockquote_outer,
        task_checked: style.task_checked,
        task_unchecked: style.task_unchecked,
        list_item: style.list_item,
        rule: style.rule,
        link_outer: style.link_outer,
        link_text: style.link_text,
        link_url: style.link_url,
        link_title: style.link_title,
        code_outer: style.code_outer,
        code_language: style.code_language,
        code_untagged: style.code_untagged,
        code_background: style.code_background,
        table_outer: style.table_outer,
        text: style.text,
        math: style.math,
    }
}

/// Theme-aware markdown style in the shape `rutui_markdown` renders.
pub fn style() -> rutui_markdown::MarkdownStyle {
    into_grok_markdown_style(rutui_theme::theme::md_style::style())
}
