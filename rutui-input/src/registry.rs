//! Generic action registry — a key-binding registry that any terminal app
//! can instantiate with its own action-id type `A`, context type `C`, and
//! (optionally) category type `K` (defaulting to this crate's
//! [`Category`]).
//!
//! Three consumers (as in the pager this was extracted from):
//! - **Shortcuts bar**: `registry.hints(contexts)` → filtered, prioritized hints
//! - **Command palette**: `registry.all()` → fuzzy searchable list
//! - **Key dispatch**: `registry.lookup(key, context)` → action to execute
//!
//! ## Input bubbling
//!
//! Each layer in the input chain does an **exact context match**:
//! 1. Pane level: `lookup(key, <pane context>)`
//! 2. Agent level: `lookup(key, <agent context>)`
//! 3. Global level: `lookup(key, <always context>)`
//!
//! The bubbling is explicit in the calling app's code, not hidden in
//! `context_matches`.

use std::borrow::Cow;

use crossterm::event::KeyEvent;

use crate::key::KeyShortcut;

/// Shortcuts-bar hint item produced by this registry.
///
/// The generic shortcuts-bar widget type (moved here from `rutui-widgets`
/// when `KeyShortcut` was unified centrally in this crate: the widgets crate
/// re-exports it as `rutui_widgets::shortcuts_bar::HintItem`, so the pager's
/// `crate::views::shortcuts_bar::HintItem` alias resolves to exactly this type
/// and `def.hint()` output flows into its bar unchanged).
#[derive(Debug, Clone)]
pub struct HintItem<K = crate::key::KeyShortcut> {
    /// Keys to display. Multiple keys are shown joined with "/" (e.g., j/k).
    pub keys: Vec<K>,
    /// Short label for the bottom bar (e.g., "send", "nav", "cancel").
    pub label: Cow<'static, str>,
    /// Optional custom key text for single-key bar paint (and cheatsheet).
    /// Multi-key bar paint (`keys.len() > 1`) ignores this and builds from
    /// structured keys (shared-mod compact or full-form join).
    pub custom_display: Option<&'static str>,
    /// Longer description for the all-shortcuts cheatsheet (e.g.,
    /// "Send prompt to agent"). When `None`, falls back to `label`.
    pub description: Option<Cow<'static, str>>,
    /// When true, the hint survives compact-mode truncation — it is always
    /// rendered regardless of `max_visible`. Use for hints that should be
    /// discoverable in every scrollback context (e.g. nav, turn, mode).
    pub pinned: bool,
}

impl<K> HintItem<K> {
    /// Single-key hint.
    pub fn new(key: K, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            keys: vec![key],
            label: label.into(),
            custom_display: None,
            description: None,
            pinned: false,
        }
    }

    /// Paired-key hint (e.g., j/k for nav, h/l for turn).
    pub fn paired(a: K, b: K, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            keys: vec![a, b],
            label: label.into(),
            custom_display: None,
            description: None,
            pinned: false,
        }
    }

    /// Mark this hint as pinned — it will always be shown in the compact
    /// shortcuts bar, even when the hint list exceeds `max_visible`.
    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }
}

/// Action category vocabulary (for grouping in a command palette).
///
/// The pager's original enum, moved here as the *default* `K` for
/// [`ActionDef`] / [`ActionRegistry`] so apps with their own category model
/// can substitute it, while apps without one get a sensible grouping set for
/// free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    GettingStarted,
    Input,
    ConversationNav,
    ConversationAction,
    Panels,
    Session,
    /// Agent Dashboard shortcuts.
    Dashboard,
}

/// A registered action definition, generic over:
/// - `A`: the app's action-id type (e.g. the pager's `ActionId`)
/// - `C`: the app's context type (e.g. the pager's `When`)
/// - `K`: the app's category type (defaults to this crate's [`Category`])
#[derive(Debug, Clone)]
pub struct ActionDef<A, C, K = Category> {
    pub id: A,
    /// Short label for shortcuts bar: "Send", "Quit", "nav"
    pub label: &'static str,
    /// Longer description for command palette
    pub description: &'static str,
    /// Optional man-style help for the shortcuts cheatsheet detail/expand UI.
    /// Consumers should fall back to `description` when this is `None`.
    pub long_help: Option<&'static str>,
    /// Default key binding
    pub default_key: KeyShortcut,
    /// Optional second key binding (e.g., j/k both shown as "j/k:nav")
    pub alt_keys: Vec<KeyShortcut>,
    /// Category for grouping
    pub category: K,
    /// When this action is available (the app's context type)
    pub context: C,
    /// Priority for shortcuts bar. None = don't show. Some(0) = highest priority.
    pub hint_priority: Option<u8>,
    /// Combined display for shortcuts bar (e.g., "j/k" for SelectNext+SelectPrev pair).
    /// If set, overrides `default_key.display()`.
    pub hint_key_display: Option<&'static str>,
    /// If true, requires double-press (1000ms TTL) to execute.
    /// The first press sets a pending action; the second press confirms.
    pub requires_confirmation: bool,
}

impl<A, C, K> ActionDef<A, C, K> {
    /// Convert this action def into a [`HintItem`] for the shortcuts bar.
    ///
    /// Uses `default_key` only. For paired hints (j/k, h/l), the view should
    /// use [`HintItem::paired`] with keys from two related action defs.
    pub fn hint(&self) -> HintItem {
        let mut item = HintItem::new(self.default_key, self.label);
        item.custom_display = self.hint_key_display;
        item.description = Some(Cow::Borrowed(self.description));
        item
    }
}

/// Registry of all actions. Single source of truth.
///
/// Generic over the app's action-id type `A` and context type `C` (and the
/// category type `K`, defaulted to [`Category`]).
#[derive(Debug, Clone)]
pub struct ActionRegistry<A, C, K = Category> {
    actions: Vec<ActionDef<A, C, K>>,
}

impl<A, C, K> ActionRegistry<A, C, K> {
    /// Create a registry with the given action definitions.
    pub fn new(actions: Vec<ActionDef<A, C, K>>) -> Self {
        Self { actions }
    }

    /// True when the send-now (interject) chord should act or be advertised:
    /// the turn is running and there is something to send. `has_payload` is
    /// true for non-empty composer text, editing a queued row, or a visible
    /// queued follow-up. Idle or no payload remains a no-op.
    pub fn interjection_possible(turn_running: bool, has_payload: bool) -> bool {
        turn_running && has_payload
    }

    /// Get all actions (for command palette).
    pub fn all(&self) -> &[ActionDef<A, C, K>] {
        &self.actions
    }
}

impl<A, C, K> ActionRegistry<A, C, K>
where
    A: PartialEq,
{
    /// Find an action definition by ID.
    pub fn find(&self, id: A) -> Option<&ActionDef<A, C, K>> {
        self.actions.iter().find(|d| d.id == id)
    }

    /// Whether `event` matches `id`'s default or any alt, ignoring `C`.
    /// Used for cross-pane chords that share an action's key set (e.g. queue
    /// force-interject uses the same keys as `InterjectPrompt`).
    pub fn matches_id(&self, id: A, event: &KeyEvent) -> bool {
        let Some(def) = self.find(id) else {
            return false;
        };
        def.default_key.matches(event) || def.alt_keys.iter().any(|k| k.matches(event))
    }

    /// Get the current key binding for an action.
    pub fn key_for(&self, id: A) -> Option<KeyShortcut> {
        self.find(id).map(|def| def.default_key)
    }
}

impl<A, C, K> ActionRegistry<A, C, K>
where
    A: PartialEq,
    C: PartialEq,
{
    /// Get the effective hint key for an action, accounting for vim mode.
    ///
    /// In non-vim mode, bare-letter bindings on the app's letter-gated
    /// context (the pager's `ScrollbackFocused`) are suppressed. This returns
    /// the first non-letter alt key instead (e.g. arrow keys), so hints show
    /// a key that actually works.
    pub fn key_for_mode(&self, id: A, vim_mode: bool, letter_gated_context: C) -> Option<KeyShortcut> {
        let def = self.find(id)?;
        if !vim_mode
            && def.context == letter_gated_context
            && def.default_key.is_letter_or_shift_letter()
        {
            def.alt_keys
                .iter()
                .find(|k| !k.is_letter_or_shift_letter())
                .copied()
        } else {
            Some(def.default_key)
        }
    }
}

impl<A, C, K> ActionRegistry<A, C, K>
where
    A: Copy + PartialEq,
    C: PartialEq,
{
    /// Look up an action by key event and current context.
    ///
    /// Uses **exact** context matching — each layer in the input chain
    /// calls this with its own context level.
    ///
    /// Peek/probe only: does **not** emit telemetry.
    pub fn lookup(&self, event: &KeyEvent, context: C) -> Option<A> {
        for def in &self.actions {
            if def.context != context {
                continue;
            }
            if def.default_key.matches(event) {
                return Some(def.id);
            }
            if def.alt_keys.iter().any(|alt| alt.matches(event)) {
                return Some(def.id);
            }
        }
        None
    }

    /// Look up an action like [`Self::lookup`] but optionally suppress
    /// bare-letter (or `Shift+letter`) bindings when `vim_mode == false`,
    /// for contexts where those letters double as text-input keys.
    ///
    /// `letter_gated` is the app's policy decision for `context`: `true` for
    /// contexts where a bare letter is also a typeable input key (the pager
    /// passes `true` for its `ScrollbackFocused` and `DashboardFocused`, where
    /// a text prompt must receive `j`/`k` when vim mode is off). Arrow / Tab /
    /// Esc / Space / PgUp / PgDn / `?` and all `Ctrl+letter` shortcuts always
    /// resolve — they come in as either the action's `default_key` (e.g.
    /// `PageUp`, `Esc`) or `alt_keys` (arrows on nav actions). Only the
    /// bare-letter primary or alt is gated; arrow `alt_keys` still match.
    pub fn lookup_with_mode(
        &self,
        event: &KeyEvent,
        context: C,
        vim_mode: bool,
        letter_gated: bool,
    ) -> Option<A> {
        for def in &self.actions {
            if def.context != context {
                continue;
            }
            let suppress_default =
                !vim_mode && letter_gated && def.default_key.is_letter_or_shift_letter();
            if !suppress_default && def.default_key.matches(event) {
                return Some(def.id);
            }
            // Alt keys: when vim_mode is off, also suppress any alt key
            // that is itself a bare letter (e.g. the `j`/`k` alts on the
            // dashboard's SelectNext / SelectPrev). Non-letter alts
            // (arrows, Tab, Space) always match.
            for alt in &def.alt_keys {
                if !vim_mode && letter_gated && alt.is_letter_or_shift_letter() {
                    continue;
                }
                if alt.matches(event) {
                    return Some(def.id);
                }
            }
        }
        None
    }

    /// Get hints for the shortcuts bar, filtered by contexts and sorted by priority.
    ///
    /// Pass multiple contexts to collect hints from all applicable levels
    /// (e.g. the pager's `&[ScrollbackFocused, AgentScreen, Always]`).
    pub fn hints(&self, contexts: &[C]) -> Vec<&ActionDef<A, C, K>> {
        let mut hints: Vec<&ActionDef<A, C, K>> = self
            .actions
            .iter()
            .filter(|def| def.hint_priority.is_some() && contexts.contains(&def.context))
            .collect();
        hints.sort_by_key(|def| def.hint_priority.unwrap_or(255));
        hints
    }

    /// Get hint items for the shortcuts bar, filtered by contexts and sorted by priority.
    ///
    /// Convenience method that converts `ActionDef`s to [`HintItem`]s.
    pub fn hint_items(&self, contexts: &[C]) -> Vec<HintItem> {
        self.hints(contexts).iter().map(|def| def.hint()).collect()
    }
}