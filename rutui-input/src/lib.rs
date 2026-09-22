//! Input handling primitives for any terminal app: key abstraction,
//! modifier-key normalization, mouse scroll normalization, single-line
//! editing, and a generic key-binding registry.
//!
//! Zero business dependencies: this crate links only presentation-primitives
//! ([`rutui_theme`]) and the textarea L0 ([`rutui_ratatui_textarea`]).

pub mod key;
pub mod keyboard_normalizer;
pub mod line_editor;
#[cfg(target_os = "macos")]
pub mod macos_modifiers;
pub mod mouse;
pub(crate) mod scroll_log;
pub mod terminal_support;

/// Generic key-binding registry: [`registry::ActionRegistry`],
/// [`registry::ActionDef`], the shortcuts-bar [`registry::HintItem`], and
/// the default [`registry::Category`] grouping vocabulary.
pub mod registry;

pub use keyboard_normalizer::{KeyboardNormalizer, ModifierState};
pub use registry::{ActionDef, ActionRegistry, Category, HintItem};
pub use terminal_support::{is_apple_terminal_newline_modifier_held, is_mod_enter};
