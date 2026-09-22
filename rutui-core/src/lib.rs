#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    unreachable_code,
    dead_code
)]
//! rutui-core — block-style conversation-history rendering pipeline.
//!
//! Extracted (W4) from `xai-grok-pager`'s `scrollback/` module: block types,
//! scrolling, folding, selection + copy, in-document search, sticky headers,
//! and the timeline rail, with zero business (Grok session / ACP) coupling.
//! Any terminal app can feed `ScrollbackState` with `RenderBlock`s and render
//! the pane; presentation primitives come from `rutui-theme`.
//!
//! Hosts, in addition to the `scrollback` tree itself:
//! - `context_info`   — pure wire data for `/context` snapshots (re-exported
//!   by xai-grok-shell so `xai_grok_shell::session::ContextInfo` still resolves)
//! - `diff`           — `DiffLine`/`DiffHunk`/patch text (re-exported by
//!   xai-grok-pager-diff; the ACP-side hunk builder lives there)
//! - `syntax`         — theme-keyed syntect cache used by edit/execute blocks
//! - `timeline`       — the turn rail (moved from the pager's `views/timeline.rs`)
//! - `bash_highlight` — bash command wrapping/highlighting for tool headers
//!   (inlined from the pager's `views/{permission_view,tasks_pane}.rs`);
//! - `line_editor`    — TEMPORARY inline copy of the pager's line editor for
//!   the search bar; to be replaced by `rutui-input` once W3 lands
//! - small pure-helper modules inlined from pager business code where the
//!   W4 decoupling pass hit couplings (see each module's header).

pub mod bash_highlight;
pub mod context_info;
pub mod diff;
pub mod error_display;
pub mod inline_media;
pub mod mcp_names;
pub mod md_style;
pub mod path_show;
pub mod scrollback;
pub mod skill_path;
pub mod subagent_meta;
pub mod syntax;
pub mod text_matcher;
pub mod timeline;
