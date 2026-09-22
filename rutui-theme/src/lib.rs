//! `rutui-theme` — a terminal presentation layer for any terminal app.
//!
//! Extracted (W1) from `xai-grok-pager-render` and cleansed of Grok / ACP
//! dependencies. Owns the render pipeline (theme, terminal probing, appearance
//! tuning, markdown *style*, clipboard decision layer, …); the Grok side
//! installs host adapters (config loader, platform clipboard ops, markdown
//! renderer, telemetry bridging) at startup through the `*::install_*` seams.

pub mod appearance;
pub mod clipboard;
pub mod effective_config;
pub mod gboom;
pub mod glyphs;
pub mod host;
pub mod link_opener;
pub mod modal_window_state;
pub mod prompt_images;
pub mod render;
pub mod stderr;
pub mod terminal;
pub mod theme;
pub mod util;
