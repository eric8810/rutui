//! The display-surface abstraction a shortcuts bar needs from a key type.
//!
//! The concrete `KeyShortcut` type and the `key!()` macro now live in exactly
//! one place: `rutui-input` (the input layer that owns key matching). This
//! module keeps the generic trait so a host application that binds the bar to
//! **its own** key type implements [`KeyHint`] for that type and instantiates
//! the generic [`crate::shortcuts_bar`] items with it. The
//! [`rutui_input::key::KeyShortcut`] bridge is implemented at the bottom of
//! this file (trait is local, type is foreign — legal).

use crossterm::event::KeyModifiers;

/// The display surface the shortcuts bar needs from a key type.
///
/// Implemented by [`rutui_input::key::KeyShortcut`]. A host application
/// that binds the bar to its **own** key type (e.g. one that lives in its
/// input layer) implements this trait for that type and instantiates the
/// generic [`crate::shortcuts_bar`] items with it.
pub trait KeyHint {
    /// Modifiers of the shortcut (used for shared-modifier compact joins).
    fn modifiers(&self) -> KeyModifiers;
    /// Full display string, e.g. `"Ctrl+Enter"`.
    fn display(&self) -> String;
    /// Key atom only (no modifier prefix), e.g. `"Enter"`.
    fn code_display(&self) -> String;
    /// Modifier prefix only, e.g. `"Ctrl+"`.
    fn modifiers_prefix(&self) -> String;
}

/// Bridge for the canonical shortcut type (owned by `rutui-input`).
///
/// The widgets crate depends on the input crate anyway (its shortcuts bar
/// defaults to `rutui_input::key::KeyShortcut`), so the impl stays here
/// next to the trait — no orphan-rule problem: `KeyHint` is a local trait,
/// `KeyShortcut` is a foreign type.
impl KeyHint for rutui_input::key::KeyShortcut {
    fn modifiers(&self) -> KeyModifiers {
        self.modifiers
    }

    fn display(&self) -> String {
        rutui_input::key::KeyShortcut::display(self)
    }

    fn code_display(&self) -> String {
        rutui_input::key::KeyShortcut::code_display(self)
    }

    fn modifiers_prefix(&self) -> String {
        rutui_input::key::KeyShortcut::modifiers_prefix(self)
    }
}