//! Reusable prompt input widget stack (framework wave W5).
//!
//! Extracted from `xai-grok-pager` (`views/prompt_widget`, `views/history_search`,
//! `views/suggestion_controller`) into a business-free crate any terminal app
//! can reuse: a multi-line editor with paste chips, image chips, a generic
//! completion source, and background-thread history search.
//!
//! Module layout:
//! - [`prompt_widget`] — the editor core (`PromptWidget`): textarea, chip
//!   elements ([`PastedImage`] image chips, paste chips), ghost text
//!   rendering, shell-completion dropdown state, and per-terminal editing
//!   behavior. Business-specific overlays (`@`-file search, `/slash`
//!   commands, predicted prompts, telemetry) deliberately live OUTSIDE this
//!   crate — embed it in your own widget type and wrap the text-mutation
//!   entry points to hook your state machines (see the pager's
//!   `views::prompt_widget` shell for the reference pattern).
//! - [`suggestion_controller`] — ghost-text + completion-dropdown state
//!   machine with generation-based staleness. Data sources are injected via
//!   [`suggestion_controller::SuggestionConfig`] at construction; fetching
//!   is left to the caller, which dispatches the returned
//!   `SuggestionAction` intents (debounce expiry → fire the request with
//!   the generation token).
//! - [`history_search`] — background-thread nucleo fuzzy matching over
//!   prompt history with a lazy daemon thread.
//!
//! Dependency policy: this crate depends only on presentation/input
//! framework crates (`rutui-theme`, `rutui-input`,
//! `rutui-ratatui-textarea`) and generic libraries — never on
//! `xai-grok-*`/`agent-client-protocol`. Business effects (ACP requests,
//! telemetry, environment config) are injected by the embedding crate.

pub mod history_search;
pub mod prompt_widget;
pub mod suggestion_controller;
