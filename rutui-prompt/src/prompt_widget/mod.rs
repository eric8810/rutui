//! Reusable prompt input widget (editor core, framework wave W5).
//!
//! [`PromptWidget`] is a text input component built on [`TextArea`] that handles
//! text editing, multiline input, paste chips, image chips, history search,
//! and shell-completion ghost text. It does NOT handle focus management
//! (Esc, Tab) or render chrome (accent lines, selection boxes) — those are the
//! caller's responsibility, making this widget reusable across agent view,
//! welcome screen, overlays, etc.
//!
//! ## What lives here vs. in the embedding crate
//!
//! This module is business-free by construction:
//!
//! - **In this crate:** textarea editing, chip elements (paste + image),
//!   image undo/reconcile, [`HistorySearchState`](crate::history_search),
//!   ghost text + completion dropdown state
//!   ([`SuggestionController`](crate::suggestion_controller)), and the
//!   generic key/mouse/paste entry points.
//! - **Left to the embedding crate:** `@`-file search overlays, `/slash`
//!   command state machines, predicted-prompt ghosts, per-key telemetry,
//!   and undo-tip / plan-nudge heuristics. The pager's
//!   `crate::views::prompt_widget` module is the reference "shell": a thin
//!   pager-local `PromptWidget` that embeds this editor via `Deref`, wraps
//!   the text-mutation entry points to refresh its business overlays, and
//!   paints its own overlay decorations after this widget's `draw`.
//!
//! ## Visual (as rendered by an embedding view with chrome)
//!
//! ```text
//!                                            ← top vpad (configurable)
//!  ❯ type here, text wraps                   ← prefix + TextArea
//!    continuation of long input...            ← TextArea continuation
//!  grok-3 · yolo                             ← info line (optional)
//! ```
//!
//! The accent line (┃) and selection box are rendered by the caller.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::StatefulWidgetRef;
use rutui_ratatui_textarea::{ElementId, ElementKind, TextArea, TextAreaState, TextElement};

use rutui_input::key;
use rutui_input::terminal_support::{is_apple_terminal_newline_modifier_held, is_mod_enter};
use rutui_theme::clipboard::{SystemClipboard, system_clipboard_get};
use rutui_theme::glyphs::PROMPT_ARROW_WIDTH;
use rutui_theme::prompt_images::{self, PastedImage};
use rutui_theme::render::SafeBuf;
use rutui_theme::render::{
    PreviewConfig, PreviewStyle, render_image_overlay, render_preview_overlay,
};
use rutui_theme::terminal::TerminalName;
use rutui_theme::theme::Theme;
use rutui_theme::{clipboard, render, terminal};

use crate::history_search::HistorySearchState;
use crate::suggestion_controller::{
    AcceptMode, CompletionSplice, SuggestionController, SuggestionSource,
};

/// Element kind tag for pasted multi-line content.
pub const KIND_PASTE: ElementKind = ElementKind(1);
/// Element kind tag for @-file references (owned by the embedding crate).
pub const KIND_FILE_REF: ElementKind = ElementKind(2);
/// Element kind tag for pasted image chips (`[Image #N]`).
pub const KIND_IMAGE: ElementKind = ElementKind(3);

/// Byte size at which a single-line paste is chipped (display only — not an offload threshold).
pub const PASTE_CHIP_DISPLAY_BYTES: usize = 10_000;

pub use rutui_theme::prompt_images::PROMPT_IMAGES_TRACING_TARGET;

/// What kind of element interaction occurred when pressing Enter on a chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementInteraction {
    /// Paste or file-ref element was inlined (expanded).
    Inlined,
    /// Image chip was activated (caller should open preview).
    ImagePreview,
}

/// What happened after the widget processed an input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptEvent {
    /// Text was modified (char inserted, deleted, paste, newline). Redraw needed.
    Edited,
    /// The widget didn't handle this event. Caller should handle it
    /// (Esc, Tab, Ctrl-D, etc.).
    Ignored,
}

/// Result of routing an Enter-family key event through a submittable prompt.
///
/// Used by [`PromptWidget::route_enter`] to centralize the behaviors that
/// every "bare Enter submits" context needs: Apple Terminal CoreGraphics
/// modifier rescue and backslash continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterOutcome {
    /// A newline was inserted (Apple Terminal modifier rescue or backslash
    /// continuation). The caller should return `InputOutcome::Changed`.
    NewlineInserted,
    /// Bare Enter with no active dropdown — the caller should perform its
    /// submit/save/advance action.
    Submit,
    /// Not an Enter key — the caller should fall through to
    /// `prompt.handle_key(key)`.
    PassThrough,
}

/// Whether the in-app `Cmd+A` "select all in prompt" handler is supported
/// for the given terminal brand.
///
/// Gated to **Ghostty only**. Every other macOS terminal either swallows
/// `Cmd+A` at the terminal layer (Apple Terminal, default iTerm2, default
/// Kitty / WezTerm) or has idiosyncratic in-terminal "Select All"
/// behaviour we don't want to fight. Ghostty users can opt in with a
/// one-line `keybind = cmd+a=unbind` in `~/.config/ghostty/config`, after
/// which this handler fires and selects all text inside the prompt
/// buffer.
///
/// Extracted as a free function so tests can verify the gate per-brand
/// without going through the global terminal context singleton.
#[doc(hidden)]
pub fn cmd_a_select_all_supported(brand: TerminalName) -> bool {
    matches!(brand, TerminalName::Ghostty)
}

/// Visual configuration for prompt rendering.
#[derive(Debug, Clone)]
pub struct PromptStyle {
    /// Whether the prompt is focused (affects prefix color, text dimming).
    pub focused: bool,
    /// Whether to show the ❯ prefix character.
    pub show_prefix: bool,
    /// Vertical padding above the text content.
    pub vpad_top: u16,
    /// Whether to render chrome (accent line + hpad + background fill).
    /// When true, the widget renders the full block layout (accent | hpad | content | hpad).
    /// When false, the widget renders only content (for use in overlays).
    pub chrome: bool,
    /// Layout config for chrome mode (accent + padding widths).
    /// Only used when `chrome` is true.
    pub chrome_pad_left: u16,
    pub chrome_pad_right: u16,
    /// Background surface for the prompt; see [`PromptBg`].
    pub bg: PromptBg,
    /// Override the accent line color. When `Some`, uses this color instead
    /// of the default `accent_user` / `gray_dim`. Used for plan mode (golden).
    pub accent_color_override: Option<ratatui::style::Color>,
    /// Override the border color (╭─╮│╰─╯). When `Some`, uses this color
    /// instead of `prompt_border_active` / `prompt_border`. Used for plan mode.
    pub border_color_override: Option<ratatui::style::Color>,
    /// Override the prefix character and its color.
    /// When `Some((str, color))`, replaces the default `❯` prefix.
    /// Used for bash mode (`"! "` in yellow).
    pub prefix_override: Option<(&'static str, ratatui::style::Color)>,
    /// Override the placeholder text shown when the textarea is empty (e.g. remember mode).
    /// When `Some(text)`, uses this instead of the default `"Build anything"`.
    pub placeholder_override: Option<&'static str>,
    /// The main composer hides the empty-textarea placeholder on focus; the feedback box keeps it visible.
    pub placeholder_when_focused: bool,
    /// Compact mode (currently unused for info_block sizing).
    pub compact: bool,
    /// Show the accent line (`┃`) on the left edge of the chrome.
    /// When false the line is hidden and its column reclaimed for content.
    pub show_accent_line: bool,
    /// Draw the prompt's border box (top `╭─╮` divider, side `│` borders, and
    /// bottom info divider). Only consulted when `chrome` is true. Defaults to
    /// `true` (the full-TUI boxed prompt); minimal mode sets it `false` for a
    /// cleaner, border-less input that still keeps the chrome padding.
    pub show_borders: bool,
    /// Session title inlined in the top border (right-aligned, 2-cell inset),
    /// styled like the bottom info line's model name.
    /// None (default) keeps the plain border. Set only by the agent view.
    pub title: Option<String>,
    /// Paint image-chip overlay into `overlay_area` (default true).
    pub image_preview: bool,
}

/// Background for the prompt widget.
///
/// Paste chips bake `theme.paste_bg` — a badge color tuned for the default
/// canvas — into their display `Line` at paste time, so the background says
/// what *kind* of surface the prompt sits on, not just its color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PromptBg {
    /// The standalone prompt's default fill (`theme.bg_base`).
    #[default]
    Default,
    /// Explicit canvas color for prompts rendered inline within another
    /// widget whose surface matches the main prompt's (dashboard dispatch
    /// box, peek reply). Chips keep their badge background.
    Canvas(ratatui::style::Color),
    /// Inline panel color (question freeform input, permission follow-up).
    /// Chip cells are repainted to blend into the panel.
    Panel(ratatui::style::Color),
}

impl PromptBg {
    /// Effective fill color; `default` is the standalone prompt's.
    pub fn color(self, default: ratatui::style::Color) -> ratatui::style::Color {
        match self {
            Self::Default => default,
            Self::Canvas(c) | Self::Panel(c) => c,
        }
    }
}

impl Default for PromptStyle {
    fn default() -> Self {
        Self {
            focused: true,
            show_prefix: true,
            vpad_top: 1,
            chrome: true,
            chrome_pad_left: 2,
            chrome_pad_right: 1,
            bg: PromptBg::Default,
            accent_color_override: None,
            border_color_override: None,
            prefix_override: None,
            placeholder_when_focused: false,
            placeholder_override: None,
            compact: false,
            show_accent_line: false,
            show_borders: true,
            title: None,
            image_preview: true,
        }
    }
}

impl PromptStyle {
    /// Style for overlays (no chrome, no vpad).
    pub fn overlay() -> Self {
        Self {
            chrome: false,
            vpad_top: 0,
            ..Default::default()
        }
    }

    /// Style for inline prompts in overlay panels (permission, plan approval,
    /// question). Focused, chromeless, no prefix, no vpad, bg_visual background.
    pub fn inline(bg: ratatui::style::Color) -> Self {
        Self {
            focused: true,
            show_prefix: false,
            vpad_top: 0,
            chrome: false,
            chrome_pad_left: 0,
            chrome_pad_right: 0,
            bg: PromptBg::Panel(bg),
            accent_color_override: None,
            border_color_override: None,
            prefix_override: None,
            placeholder_when_focused: false,
            placeholder_override: None,
            compact: false,
            show_accent_line: false,
            show_borders: false,
            title: None,
            image_preview: true,
        }
    }

    /// Info block height: rows reserved for the bottom divider line.
    pub fn info_block(&self, has_info: bool) -> u16 {
        if has_info { 1 } else { 0 } // bottom divider only
    }

    /// Mode-tinted accent: the override (e.g. plan mode) when set,
    /// else the focus-dependent default. Shared by the accent line and the
    /// prompt prefix so tints stay in lockstep.
    pub fn accent_color(&self, theme: &Theme) -> ratatui::style::Color {
        self.accent_color_override.unwrap_or(if self.focused {
            theme.accent_user
        } else {
            theme.gray_dim
        })
    }
}

/// A flag displayed in the prompt info line (e.g., "plan", "always-approve").
pub struct PromptFlag<'a> {
    /// Display text for the flag.
    pub text: &'a str,
    /// Optional color override. When `Some`, the flag renders in this color
    /// (blended toward bg for subtlety). When `None`, uses the default gray.
    pub color: Option<ratatui::style::Color>,
    /// Whether to render with bold modifier.
    pub bold: bool,
}

/// Optional info line rendered below the prompt text.
///
/// The default is blank: a caller that wants the bottom border without any info text passes it, and [`Self::is_blank`] then skips the text pass.
#[derive(Default)]
pub struct PromptInfo<'a> {
    /// Primary label to display on the info line (left side).
    pub model_name: &'a str,
    /// Flags to display on the left side, joined by " · " (e.g., "plan", "always-approve").
    pub flags: &'a [PromptFlag<'a>],
    /// Whether multiline mode is active (shown right-aligned).
    pub multiline: bool,
    /// Optional usage warning displayed right-aligned (e.g. "5% usage left").
    pub usage_warning: Option<&'a str>,
    /// When true the warning uses the yellow warning color (<=5% left);
    /// when false it uses dim grey text (5-10% left).
    pub usage_warning_critical: bool,
}

impl PromptInfo<'_> {
    pub fn is_blank(&self) -> bool {
        self.model_name.is_empty()
            && self.flags.is_empty()
            && !self.multiline
            && self.usage_warning.is_none()
    }
}

/// Live voice-capture overlay for the prompt.
///
/// Interim STT paints as muted italic ghost text (not in the textarea).
/// Finalized STT is real prompt content and stays editable while the mic is open.
/// Overlay presence (even with no interim) marks voice active for callers that
/// skip empty-state placeholders while capturing.
#[derive(Debug, Clone, Copy)]
pub struct VoicePromptOverlay<'a> {
    /// Latest interim transcript, if any.
    pub interim: Option<&'a str>,
    /// Theme accent associated with this overlay.
    pub color: ratatui::style::Color,
}

/// Greedy word-wrap the interim transcript into at most `max_rows` lines of
/// `max_w` columns, appending an ellipsis to the last line when truncated.
pub fn wrap_voice_interim(text: &str, max_w: usize, max_rows: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    if max_w == 0 || max_rows == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for word in text.split_whitespace() {
        let ww = UnicodeWidthStr::width(word);
        if !cur.is_empty() && cur_w + 1 + ww > max_w {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if cur.is_empty() {
            if ww > max_w {
                cur = render::line_utils::truncate_str(word, max_w);
                cur_w = UnicodeWidthStr::width(cur.as_str());
            } else {
                cur.push_str(word);
                cur_w = ww;
            }
        } else {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max_rows {
        lines.truncate(max_rows);
        if let Some(last) = lines.last_mut() {
            let trimmed = render::line_utils::truncate_str(last, max_w.saturating_sub(1));
            *last = format!("{trimmed}\u{2026}");
        }
    }
    lines
}

/// Result of rendering the prompt.
pub struct PromptRenderResult {
    /// Cursor position if the prompt wants a visible cursor.
    /// `None` when unfocused.
    pub cursor_pos: Option<(u16, u16)>,
    /// Terminal escape sequences to write after the ratatui cell flush
    /// (e.g. Kitty/iTerm2 inline image rendering for the image preview).
    pub post_flush_escapes: Option<terminal::overlay::Escapes>,
    /// Textarea render area from the last draw — needed by embedding crates
    /// that paint overlay decorations (slash highlights, ghost text) after
    /// this widget's draw, since those need the textarea's screen geometry
    /// and wrap state.
    pub ta_area: Rect,
}

// ---------------------------------------------------------------------------
// Backspace-no-effect telemetry hook (optional, default no-op)
// ---------------------------------------------------------------------------

/// Business-free description of a "Backspace/Delete produced no edit" event.
///
/// The framework never touches a telemetry crate. An embedding crate can
/// install a callback (once, globally) via
/// [`install_backspace_no_effect_telemetry`] and call
/// [`backspace_no_effect_emit`] from its key-handling wrapper; until then
/// emission is a cheap no-op.
#[derive(Debug, Clone)]
pub struct BackspaceNoEffectEvent {
    /// Debug rendering of the terminal name.
    pub terminal_brand: String,
    pub terminal_multiplexer: String,
    pub terminal_is_ssh: bool,
    pub terminal_term_var: Option<String>,
    pub terminal_term_version: Option<String>,
    pub terminal_term_version_source: Option<String>,
    pub key_code: String,
    pub key_modifiers: String,
    pub key_kind: String,
    pub cursor_pos: usize,
    pub text_len: usize,
    pub has_selection: bool,
}

/// Global (once) callback run by [`backspace_no_effect_emit`].
/// Installable by any embedding crate; defaults to no-op.
pub fn install_backspace_no_effect_telemetry(
    f: fn(BackspaceNoEffectEvent),
) -> Result<(), fn(BackspaceNoEffectEvent)> {
    BACKSPACE_NO_EFFECT_TELEMETRY.set(f)
}

static BACKSPACE_NO_EFFECT_TELEMETRY: std::sync::OnceLock<fn(BackspaceNoEffectEvent)> =
    std::sync::OnceLock::new();

/// Report a "deletion key had no effect" event to the installed callback
/// (no-op by default). Cheap when nothing is installed.
pub fn backspace_no_effect_emit(evt: BackspaceNoEffectEvent) {
    // Point of inefficiency: a OnceLock read — fine on this path (only fires
    // on an already-rare no-effect backspace).
    if let Some(f) = BACKSPACE_NO_EFFECT_TELEMETRY.get() {
        f(evt);
    }
}

/// Capture the telemetry snapshot for [`BackspaceNoEffectEvent`] — a thin
/// accessor so embeddings can log "event not wired to a callback" warnings.
pub fn backspace_no_effect_telemetry_installed() -> bool {
    BACKSPACE_NO_EFFECT_TELEMETRY.get().is_some()
}

// ---------------------------------------------------------------------------
// Editor core
// ---------------------------------------------------------------------------

/// Prefix display width (`"❯ "` or `"> "` — both 2 columns).
const PREFIX_WIDTH: u16 = PROMPT_ARROW_WIDTH;

/// Reusable text prompt component.
///
/// Handles text editing, multiline (Shift/Alt-Enter), submit (Enter),
/// and Ctrl-C (clear if non-empty). Delegates mouse events to TextArea
/// which tracks drag state internally (including drag-beyond-edge).
///
/// Does NOT handle: Esc, Tab (focus management), Ctrl-D (quit).
/// Does NOT render: accent line, selection box (caller's job).
///
/// This is the *editor core*: business overlays (`@`-file search, `/slash`
/// commands, predicted prompts) belong to the embedding crate, which wraps
/// this type (see module docs). Embeddings should wrap the text-mutation
/// entry points (`handle_key`, `handle_paste`, `handle_mouse`, `set_text`,
/// `append_text`, the completion apply/accept methods, `stash`/`restore`)
/// to refresh their own state machines after edits.
pub struct PromptWidget {
    pub textarea: TextArea,
    textarea_state: TextAreaState,
    /// Cached textarea render area from last draw (for mouse coordinate mapping).
    textarea_area: Rect,
    /// Prompt history panel state: search mode (`/history`) or
    /// browse mode (Up on an empty prompt).
    pub history_search: HistorySearchState,

    /// Shell command suggestion controller (ghost text + progressive matching).
    pub suggestions: SuggestionController,

    // -- Image paste state ---------------------------------------------------
    /// Images attached to the current prompt.
    pub images: Vec<PastedImage>,
    /// Images removed during undo that can be restored on redo.
    ///
    /// `pub` so embedding shells can move the whole stash across stash/
    /// restore cycles (see the pager's `StashedPrompt`).
    pub image_undo_stash: Vec<PastedImage>,
    /// Image element currently hovered by mouse hit-testing.
    hovered_image_element_id: Option<rutui_ratatui_textarea::ElementId>,
    /// Image shown immediately after insertion without moving the edit cursor.
    post_insert_image_preview: Option<(rutui_ratatui_textarea::ElementId, usize)>,
    /// Monotonic counter for image display numbering (1-based). Reset on
    /// prompt clear. Only increases within a single prompt lifetime.
    pub image_counter: usize,

    /// Compact mode lowers the paste-chip threshold from 4 lines to 2.
    compact: bool,

    /// Whether the in-app Cmd+A handler is enabled (gated to Ghostty).
    ///
    /// Initialised from `terminal_context().brand` at construction. Only
    /// Ghostty is supported because:
    /// - It's the only terminal in current use that can forward `Cmd+A`
    ///   without requiring the user to unbind on the terminal side
    ///   (`keybind = cmd+a=unbind`) — but that one terminal-side step
    ///   is well-documented and we want the binding to behave on Ghostty.
    /// - Other terminals either silently swallow the key (Apple
    ///   Terminal, default iTerm2) or have idiosyncratic in-terminal
    ///   "Select All" behaviour we don't want to fight.
    ///
    /// Embeddings without the theme terminal-context singleton can build with
    /// [`PromptWidget::new_with_cwd_gated`].
    cmd_a_select_all_enabled: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl PromptWidget {
    /// Test-only toggle for the Cmd+A gate (historical tests flip the
    /// private field directly; the field moved here with the editor core).
    pub fn set_cmd_a_select_all_enabled_for_test(&mut self, enabled: bool) {
        self.cmd_a_select_all_enabled = enabled;
    }

    /// Test-only hovered-image-element override.
    pub fn set_hovered_image_element_for_test(&mut self, id: Option<ElementId>) {
        self.hovered_image_element_id = id;
    }

    /// Test-only direct drive of the image/textarea reconcile pass.
    pub fn sync_images_for_test(&mut self) {
        self.sync_images_with_textarea();
    }

    /// Test-only backslash-continuation check (private in production).
    pub fn apply_backslash_continuation_for_test(&mut self) -> bool {
        self.apply_backslash_continuation()
    }

    /// Test-only paste-element preview probe.
    pub fn paste_element_for_preview_for_test(&self) -> Option<&str> {
        self.paste_element_for_preview()
    }

    /// Test-only image-preview probe.
    pub fn image_for_preview_for_test(&self) -> Option<&PastedImage> {
        self.image_for_preview()
    }

    /// Test-only paste-preview hint builder.
    pub fn paste_preview_hint_for_test(&self, theme: &Theme) -> Line<'static> {
        self.paste_preview_hint(theme)
    }
}

impl PromptWidget {
    /// Create a new empty prompt widget with system clipboard support.
    ///
    /// The `cwd` parameter is a compatibility hook for embeddings whose
    /// overlays want a base directory at construction; the editor core itself
    /// has no filesystem state.
    pub fn new_with_cwd(_cwd: &Path) -> Self {
        Self::new_with_cwd_and_cmd_a(
            _cwd,
            cmd_a_select_all_supported(terminal::terminal_context().brand),
        )
    }

    /// Like [`PromptWidget::new_with_cwd`], with an explicit Cmd+A gate.
    /// `cmd_a_select_all_enabled` mirrors the per-brand gate; embeddings that
    /// cannot touch `terminal_context()` compute it themselves.
    pub fn new_with_cwd_and_cmd_a(_cwd: &Path, cmd_a_select_all_enabled: bool) -> Self {
        let mut textarea = TextArea::new();
        textarea.set_clipboard_provider(Box::new(SystemClipboard));
        Self {
            textarea,
            textarea_state: TextAreaState::default(),
            textarea_area: Rect::default(),
            history_search: HistorySearchState::new(),
            suggestions: SuggestionController::new(),
            images: Vec::new(),
            image_undo_stash: Vec::new(),
            hovered_image_element_id: None,
            post_insert_image_preview: None,
            image_counter: 0,
            compact: false,
            cmd_a_select_all_enabled,
        }
    }

    /// Create a new empty prompt widget (no file search — for tests/fallback).
    pub fn new() -> Self {
        Self::new_with_cwd(Path::new("."))
    }

    /// Give the textarea a tab expansion width (embedding appearance glue).
    /// The pager's `AppView` calls this on hot-reload via its shell
    /// (`sync_tab_width_from_appearance`).
    pub fn set_tab_width(&mut self, width: u8) {
        self.textarea.set_tab_width(width);
    }

    /// Clear the transient image-chip preview state (hover + post-insert
    /// tracking). Embedding shells call this at the top of their key
    /// wrappers before handling keys that mutate the buffer outside the
    /// editor's own entry points.
    pub fn clear_element_preview_state(&mut self) {
        self.post_insert_image_preview = None;
        self.hovered_image_element_id = None;
    }

    pub fn set_compact(&mut self, compact: bool) {
        self.compact = compact;
    }

    /// Current compact flag (the derived render value fanned out by
    /// the embedding's appearance layer).
    pub fn compact(&self) -> bool {
        self.compact
    }

    /// Textarea render state from the last draw (for embeddings painting
    /// overlay decorations — e.g. span highlighting uses screen coords and
    /// the wrap state). `TextAreaState` is `Copy`.
    pub fn textarea_state(&self) -> TextAreaState {
        self.textarea_state
    }

    /// Set the ghost text for shell command suggestions.
    ///
    /// Source is set to [`SuggestionSource::None`]. Use
    /// `suggestions.set_ghost()` directly for source-aware callers.
    pub fn set_ghost_text(&mut self, text: Option<String>) {
        match text {
            Some(t) if !t.is_empty() => {
                self.suggestions.set_ghost(t, SuggestionSource::None);
            }
            _ => self.suggestions.clear_ghost(),
        }
    }

    /// Whether non-empty ghost text is set and would render.
    pub fn has_ghost_text(&self) -> bool {
        self.suggestions.has_ghost()
    }

    /// Accept ghost text (full or one word). Appends the accepted portion
    /// to the textarea. Returns `None` if no ghost is active or cursor is
    /// not at end-of-text. Embeddings should wrap this to refresh their
    /// `@`-completion context after the insert.
    pub fn accept_ghost(&mut self, mode: AcceptMode) -> Option<String> {
        if self.cursor() != self.text().len() {
            return None;
        }
        let accepted = self.suggestions.accept_ghost(mode)?;
        self.textarea.insert_str(&accepted);
        Some(accepted)
    }

    /// Dismiss ghost text without accepting it.
    pub fn clear_ghost(&mut self) {
        self.suggestions.clear_ghost();
    }

    /// Try progressive matching against the new text. Returns `true` if
    /// the ghost was trimmed (no network request needed).
    pub fn try_progressive_match(&mut self, new_text: &str) -> bool {
        self.suggestions.try_progressive_match(new_text)
    }

    // -- Completion dropdown ------------------------------------------------

    /// Whether the completion dropdown is currently open.
    pub fn completion_dropdown_open(&self) -> bool {
        self.suggestions.dropdown.open
    }

    /// Open the completion dropdown if items are available.
    pub fn completion_dropdown_open_if_available(&mut self) -> bool {
        if !self.suggestions.dropdown.items.is_empty() {
            self.suggestions.dropdown.open = true;
            true
        } else {
            false
        }
    }

    /// Close the completion dropdown.
    pub fn completion_dropdown_close(&mut self) {
        self.suggestions.dropdown.close();
    }

    /// Move the completion dropdown selection (Up = -1, Down = +1), wrapping.
    pub fn completion_dropdown_move(&mut self, delta: isize) {
        self.suggestions.dropdown.move_selection(delta);
    }

    /// Scroll the completion dropdown selection by `delta`, clamping at the
    /// ends (no wrap-around). Used for mouse-wheel and page-key scrolling.
    pub fn completion_dropdown_scroll(&mut self, delta: isize) {
        self.suggestions.dropdown.scroll_selection(delta);
    }

    /// Accept the selected completion, resolved against the current draft
    /// (stale generations refuse and close — see
    /// [`SuggestionController::accept_completion`]). Also clears ghost text
    /// since the accepted completion replaces it.
    pub fn completion_dropdown_accept(&mut self) -> Option<CompletionSplice> {
        let result = self.suggestions.accept_completion(self.textarea.text());
        if result.is_some() {
            self.suggestions.clear_ghost();
        }
        result
    }

    /// Write a resolved accept into the textarea. Returns whether text was
    /// written (`Stale` is a draft-preserving no-op); cursor lands after the
    /// insert. Embeddings should wrap this to refresh their completion
    /// context after `Token` writes.
    pub fn apply_completion_splice(&mut self, splice: CompletionSplice) -> bool {
        // Accepting rewrites the token — an unrelated highlight must not survive it.
        self.textarea.clear_selection();
        match splice {
            CompletionSplice::WholeLine(line) => {
                self.set_text(&line);
                self.set_cursor(line.len());
                true
            }
            CompletionSplice::Token(range, replacement) => {
                if self.completion_range_clips_element(&range) {
                    return false;
                }
                let start = range.start;
                self.textarea.replace_range(range, &replacement);
                self.textarea.set_cursor(start + replacement.len());
                true
            }
            CompletionSplice::Stale => false,
        }
    }

    /// Write a `TabAction::Fill` over the typed token (bash's first Tab).
    /// The pair was validated against this draft by `tab_decision` in the
    /// same key/landing event, so it applies directly; cursor lands after
    /// the fill. Returns whether the fill was written — a range clipping an
    /// atomic element is declined, and the caller must NOT treat a declined
    /// fill like typing (no re-fetch: that loop would spin on every Tab).
    pub fn apply_completion_fill(&mut self, range: std::ops::Range<usize>, fill: &str) -> bool {
        if self.completion_range_clips_element(&range) {
            return false;
        }
        let start = range.start;
        self.textarea.replace_range(range, fill);
        self.textarea.set_cursor(start + fill.len());
        true
    }

    /// Whether accepting the selected completion would splice a range that
    /// clips an atomic element — the write path would decline it AFTER the
    /// accept consumed the item, so callers probe first and degrade to
    /// opening the dropdown instead.
    pub fn completion_accept_would_clip_element(&self) -> bool {
        match self
            .suggestions
            .peek_completion_splice(self.textarea.text())
        {
            Some(CompletionSplice::Token(range, _)) => self.completion_range_clips_element(&range),
            _ => false,
        }
    }

    /// Whether a completion range overlaps an atomic prompt element (paste
    /// chip, file ref, image chip). Completion ranges are computed over the
    /// plain request text, but `TextArea::replace_range` expands any
    /// element overlap to the WHOLE element — accepting such a range would
    /// swallow the entire chip (paste data loss). Treated as a stale no-op.
    fn completion_range_clips_element(&self, range: &std::ops::Range<usize>) -> bool {
        self.textarea
            .elements()
            .iter()
            .any(|e| e.range.start < range.end && e.range.end > range.start)
    }

    /// Set the hovered completion dropdown item. Returns `true` if changed.
    pub fn set_completion_hovered(&mut self, index: Option<usize>) -> bool {
        let clamped = index.and_then(|i| {
            if i < self.suggestions.dropdown.items.len() {
                Some(i)
            } else {
                None
            }
        });
        let changed = clamped != self.suggestions.dropdown.hovered;
        self.suggestions.dropdown.hovered = clamped;
        changed
    }

    /// Select the hovered completion item (keyboard selection = hovered).
    pub fn select_completion_hovered(&mut self) -> bool {
        if let Some(idx) = self.suggestions.dropdown.hovered {
            self.suggestions.dropdown.selected = idx;
            true
        } else {
            false
        }
    }

    /// Get the current text.
    pub fn text(&self) -> &str {
        self.textarea.text()
    }

    /// Get the current cursor position (byte offset into text).
    pub fn cursor(&self) -> usize {
        self.textarea.cursor()
    }

    /// Clear the undo/redo history (see [`TextArea::clear_history`]).
    /// Pair with `set_text("")` when reusing the widget for a new
    /// logical context so an undo can't resurrect the previous draft.
    pub fn clear_history(&mut self) {
        self.textarea.clear_history();
    }

    /// Set the cursor position (byte offset), clamped to text length.
    pub fn set_cursor(&mut self, pos: usize) {
        self.post_insert_image_preview = None;
        self.textarea.set_cursor(pos);
    }

    /// Set the text content.
    ///
    /// When `text` is empty this is a full prompt reset: image-side state
    /// and the image counter are cleared so the next prompt starts fresh.
    ///
    /// **Image lifecycle contract.** Non-empty replacements that
    /// don't contain any `[Image #N]` placeholders also clear the
    /// image lists and counter, since orphan `PastedImage` records
    /// without matching chips can't be reached by the user. Embeddings
    /// that restore an in-flight prompt after `set_text` must also restore
    /// its chip elements and images (see [`Self::set_images`]). Overlay
    /// contexts should wrap this with their own stash/restore machinery.
    /// The `image_counter` is preserved when chips remain so numbering stays
    /// monotonic within a prompt lifetime.
    pub fn set_text(&mut self, text: &str) {
        self.post_insert_image_preview = None;
        self.hovered_image_element_id = None;
        self.textarea.set_text(text);
        // Suggestion state (ghost, dropdown items, in-flight fetches) is
        // tied to a SPECIFIC draft: every wholesale content swap — empty or
        // not — invalidates it, so nothing stale survives into (or lands
        // over) the new text.
        self.suggestions.invalidate_draft();
        if text.is_empty() {
            prompt_images::drain_and_cleanup(&mut self.images);
            prompt_images::drain_and_cleanup(&mut self.image_undo_stash);
            prompt_images::reset_counter(&mut self.image_counter);
        } else if !chip_placeholder_regex().is_match(text) {
            // Non-empty replacement with no chip placeholder match —
            // any retained `PastedImage` records are now orphan (no
            // chip in the buffer references them). Drop them to
            // keep `drain_images()` honest and to release encoded
            // bytes. Counter is reset since no chips survive.
            prompt_images::drain_and_cleanup(&mut self.images);
            prompt_images::drain_and_cleanup(&mut self.image_undo_stash);
            prompt_images::reset_counter(&mut self.image_counter);
        }
    }

    /// [`Self::set_text`] unless the buffer already holds exactly `text`.
    ///
    /// Skipping the no-op swap keeps chip elements, images, and undo history
    /// intact when a surface reloads an unchanged draft (the question view's
    /// freeform slots); any real content change takes the normal reset path.
    pub fn set_text_preserving(&mut self, text: &str) {
        if self.text() != text {
            self.set_text(text);
        }
    }

    /// Append plain text at the end without replacing existing chip elements.
    /// Embeddings should wrap this to refresh their `@`-completion context.
    pub fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.post_insert_image_preview = None;
        self.hovered_image_element_id = None;
        let end = self.textarea.text().len();
        self.set_cursor(end);
        self.textarea.insert_str(text);
    }

    /// How tall this widget wants to be, given the available width of the
    /// full prompt area (including chrome if enabled).
    ///
    /// Height = vpad_top + textarea rows + vpad_info(1) + info line.
    /// Clamped to max_height.
    pub fn desired_height(
        &self,
        area_width: u16,
        style: &PromptStyle,
        has_info: bool,
        max_height: u16,
    ) -> u16 {
        let content_width = self.content_width(area_width, style);
        let prefix_w = if style.show_prefix { PREFIX_WIDTH } else { 0 };
        let text_width = content_width.saturating_sub(prefix_w);
        // History browse live-populates the composer per selection move;
        // freeze the box at its pre-open one-row height so stepping through
        // entries of different heights doesn't resize the layout per
        // keypress. The first real edit detaches and the box resizes then.
        let text_height = if self.history_search.is_browse() {
            1
        } else {
            self.textarea.desired_height(text_width).max(1)
        };
        let vpad_top = style.vpad_top;
        let info_block = style.info_block(has_info);
        let total = vpad_top + text_height + info_block;
        let min = vpad_top + 1 + info_block;
        total.clamp(min.min(max_height), max_height)
    }

    /// Current vertical scroll offset of the textarea.
    pub fn scroll(&self) -> u16 {
        self.textarea_state.scroll
    }

    /// Set the vertical scroll offset (e.g., restore after collapsed render).
    pub fn set_scroll(&mut self, scroll: u16) {
        self.textarea_state.scroll = scroll;
    }

    /// Last rendered textarea area for mouse hit-testing.
    pub fn textarea_area(&self) -> Rect {
        self.textarea_area
    }

    /// Compute the content width inside the chrome (if any).
    fn content_width(&self, area_width: u16, style: &PromptStyle) -> u16 {
        if style.chrome {
            let accent_w = if style.show_accent_line { 1u16 } else { 0 };
            area_width.saturating_sub(accent_w + style.chrome_pad_left + style.chrome_pad_right)
        } else {
            area_width
        }
    }
}

impl PromptWidget {
    /// Handle a key event for text editing only.
    ///
    /// This does NOT handle send/submit — that's routed through the action
    /// registry and [`try_send()`]. The widget handles:
    /// - Newline insertion (Shift-Enter, Alt-Enter)
    /// - Clear (Ctrl-C with text)
    /// - All readline-style editing via `textarea.input()` (Ctrl-A/E/W/U/K/Y,
    ///   undo/redo, word movement, etc.)
    ///
    /// Embeddings that route other keys (file-search dropdown navigation,
    /// slash commands, viewer triggers) should wrap this method: handle
    /// their own keys first, delegate here for pass-through, then refresh
    /// their overlays when the returned event is [`PromptEvent::Edited`].
    pub fn handle_key(&mut self, key: &KeyEvent) -> PromptEvent {
        // Esc/Tab: drop the highlight but decline the press so it still cancels / switches focus.
        // Modified Esc has no structural consumer — it falls through to the
        // textarea catch-all, which clears the highlight WITH a repaint.
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
            || (key.code == KeyCode::Esc && key.modifiers.is_empty())
        {
            self.textarea.clear_selection();
            return PromptEvent::Ignored;
        }

        // ── Normal key handling ─────────────────────────────────────────

        // Newline: Shift/Alt+Enter, or Apple Terminal bare Enter with a
        // newline modifier held (CoreGraphics rescue inside is_mod_enter).
        if is_mod_enter(key) {
            self.insert_replacing_selection("\n");
            return PromptEvent::Edited;
        }

        // Clear: Ctrl-C (empty → Ignored so caller can handle)
        if key!('c', CONTROL).matches(key) {
            return if self.textarea.text().is_empty() {
                PromptEvent::Ignored
            } else {
                self.set_text("");
                PromptEvent::Edited
            };
        }

        // Ctrl-V / Cmd-V: intercept to route through handle_paste() for
        // element creation. Without this, textarea.input() would insert
        // multi-line clipboard content inline instead of creating a paste
        // element. Ghostty passes Cmd+V (SUPER) through as a key event
        // when the clipboard has no text content.
        if key::is_paste_key(key) {
            if let Some(text) = system_clipboard_get() {
                if !clipboard::clipboard_text_is_pasteable(Some(&text)) {
                    clipboard::log_paste_key_empty_host_clipboard("prompt_widget");
                }
                // handle_paste is the text mutation; embeddings wrap it.
                return self.handle_paste(&text);
            }
            clipboard::log_paste_key_empty_host_clipboard("prompt_widget");
            return PromptEvent::Ignored;
        }

        // Ctrl-Shift-V: inline paste — insert clipboard content directly
        // without creating a [Pasted: N lines] element.
        if key::is_inline_paste_key(key) {
            if let Some(text) = system_clipboard_get() {
                let text = normalize_cr(&text);
                if text.is_empty() {
                    clipboard::log_paste_key_empty_host_clipboard("prompt_widget_inline");
                    return PromptEvent::Ignored;
                }
                self.insert_replacing_selection(&text);
                return PromptEvent::Edited;
            }
            clipboard::log_paste_key_empty_host_clipboard("prompt_widget_inline");
            return PromptEvent::Ignored;
        }

        // Cmd+A (macOS, Ghostty only): select all text in the prompt
        // textarea.
        //
        // **Gated to Ghostty.** Every other macOS terminal binds
        // `Cmd+A` to its own in-terminal "Select All" by default, and
        // we don't want to compete with their built-in behaviour or
        // surprise users on terminals where the key doesn't even
        // arrive. Ghostty users can unbind it with a one-line config
        // (`keybind = cmd+a=unbind`) — after that, this handler fires
        // and selects the prompt buffer instead of the entire terminal
        // window. See `docs/user-guide/03-keyboard-shortcuts.md`.
        //
        // The gate is read from a struct field initialised from
        // `terminal_context().brand` at construction (see
        // `cmd_a_select_all_supported`). Tests in embedding crates
        // override the field via the test-support exposure.
        //
        // Image / paste chips are textarea elements, and
        // `selection_range()` expands selections to element boundaries
        // automatically, so a full-buffer selection naturally covers
        // every chip. Subsequent Backspace / Delete / typing falls
        // through to the textarea's existing selection-aware handlers
        // (which `delete_selection()` + `sync_images_with_textarea()`
        // keep consistent with `self.images`). The `[Image #N: /path]`
        // text in each chip means a follow-on Ctrl+X (cut) preserves
        // the file path through the system clipboard.
        //
        // Ctrl+A is intentionally NOT remapped: the textarea uses it
        // for the Emacs / readline "move cursor to beginning of line"
        // binding, which terminal users rely on.
        if self.cmd_a_select_all_enabled && key!('a', SUPER).matches(key) {
            let len = self.textarea.text().len();
            if len == 0 {
                return PromptEvent::Ignored;
            }
            self.textarea.set_selection(0, len);
            // Move the cursor to the end of the selection so the
            // visible caret matches the typical GUI "head at end"
            // convention after Cmd+A.
            self.textarea.set_cursor(len);
            return PromptEvent::Edited;
        }

        // Everything else: delegate to textarea.
        // Selection is rendered state: selection-only changes must report Edited or no frame is drawn.
        let old_text = self.textarea.text().to_owned();
        let old_cursor = self.textarea.cursor();
        let old_selection = self.textarea.selection_range();
        self.textarea.input(*key);
        let new_cursor = self.textarea.cursor();
        let new_selection = self.textarea.selection_range();
        let changed = self.textarea.text() != old_text
            || new_cursor != old_cursor
            || new_selection != old_selection;
        if changed {
            // Image chips only change with the text — skip the resync on
            // selection/cursor-only changes (held Shift+arrow extends).
            if self.textarea.text() != old_text {
                self.sync_images_with_textarea();
            }
            PromptEvent::Edited
        } else {
            PromptEvent::Ignored
        }
    }

    /// If the character immediately before the cursor is `\`, replace it
    /// with a newline and return `true` (continuation applied).  Returns
    /// `false` when no continuation is needed.
    fn apply_backslash_continuation(&mut self) -> bool {
        let cursor = self.textarea.cursor();
        if cursor > 0 && self.textarea.text().as_bytes().get(cursor - 1) == Some(&b'\\') {
            self.textarea.delete_backward(1);
            self.textarea.insert_str("\n");
            return true;
        }
        false
    }

    /// Shared Enter-routing for any context where bare Enter submits.
    ///
    /// Centralizes two of the three behaviors submittable prompts need
    /// (the third — accepting an open `@`-file dropdown — belongs to the
    /// embedding crate's wrapper, since the dropdown state lives there):
    /// 1. Apple Terminal CoreGraphics modifier rescue (Shift+Enter arrives
    ///    as bare Enter on terminals without Kitty protocol)
    /// 2. Backslash continuation (`\` + Enter → newline)
    ///
    /// Callers match on the result:
    /// - `NewlineInserted` → return `InputOutcome::Changed`
    /// - `Submit` → perform the context-specific submit action
    /// - `PassThrough` → fall through to `self.handle_key(key)`
    pub fn route_enter(&mut self, key: &KeyEvent) -> EnterOutcome {
        // Apple Terminal: Shift+Enter arrives as bare Enter because there's
        // no Kitty keyboard protocol.  Poll CoreGraphics for the real
        // modifier state — if Shift/Option/Cmd is physically held, insert
        // a newline instead of submitting.
        if key.code == KeyCode::Enter && is_apple_terminal_newline_modifier_held() {
            self.insert_replacing_selection("\n");
            return EnterOutcome::NewlineInserted;
        }

        // Backslash continuation: if the character before the cursor is `\`,
        // replace it with a newline.
        if key.code == KeyCode::Enter
            && key.modifiers.is_empty()
            && self.apply_backslash_continuation()
        {
            return EnterOutcome::NewlineInserted;
        }

        // Bare Enter → submit.
        if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            return EnterOutcome::Submit;
        }

        // Not Enter, or Enter with modifiers (Shift/Alt) — let handle_key()
        // process it (Shift+Enter inserts newline there).
        EnterOutcome::PassThrough
    }

    /// Attempt to send the prompt text.
    ///
    /// Called when the `SendPrompt` action is triggered (via action registry).
    /// Returns `Some(text)` if the prompt was submitted, `None` if rejected
    /// (empty text, backslash continuation).
    ///
    /// Handles:
    /// - Empty/whitespace rejection → returns None
    /// - Backslash continuation: trailing `\` before cursor → replaces with newline
    /// - On success: returns the text and clears the widget
    pub fn try_send(&mut self) -> Option<String> {
        if self.apply_backslash_continuation() {
            return None;
        }

        let text = self.textarea.text().to_string();

        // Reject empty/whitespace-only
        if text.trim().is_empty() {
            return None;
        }

        Some(text)
    }

    /// Whether the prompt is in a state where send would succeed.
    ///
    /// Used for dynamic hint visibility (show "Enter:send" only when sendable).
    pub fn can_send(&self) -> bool {
        let text = self.textarea.text();
        if text.trim().is_empty() {
            return false;
        }
        // Check backslash continuation — trailing `\` means send would do continuation, not send
        let cursor = self.textarea.cursor();
        if cursor > 0 && text.as_bytes().get(cursor - 1) == Some(&b'\\') {
            return false;
        }
        true
    }

    /// Handle a mouse event.
    ///
    /// Forwards to TextArea which handles click-to-place, drag-to-select,
    /// double/triple click, and element interactions. The caller should forward
    /// ALL mouse events (not just those in the prompt area) because TextArea
    /// tracks drag state internally and handles drag-beyond-edge.
    /// Embeddings should wrap this to refresh `@`-completion context after
    /// cursor-placing actions.
    pub fn handle_mouse(&mut self, mouse: &crossterm::event::MouseEvent) -> PromptEvent {
        use rutui_ratatui_textarea::{MouseAction, TextElementEventKind};

        self.post_insert_image_preview = None;
        let action = self
            .textarea
            .handle_mouse(*mouse, self.textarea_area, self.textarea_state);

        while let Some(event) = self.textarea.poll_element_event() {
            match event.kind {
                TextElementEventKind::HoverEnter => {
                    let is_image = self
                        .textarea
                        .elements()
                        .iter()
                        .any(|e| e.id == event.id && e.kind == KIND_IMAGE);
                    if is_image {
                        self.hovered_image_element_id = Some(event.id);
                    }
                }
                TextElementEventKind::HoverLeave => {
                    if self.hovered_image_element_id == Some(event.id) {
                        self.hovered_image_element_id = None;
                    }
                }
                TextElementEventKind::Click => {}
            }
        }

        match action {
            MouseAction::CursorPlaced
            | MouseAction::SelectionUpdated
            | MouseAction::SelectionFinished
            | MouseAction::Scrolled => PromptEvent::Edited,
            MouseAction::Nothing => PromptEvent::Edited,
        }
    }

    /// Handle a paste event.
    ///
    /// Short pastes are inserted inline.  At or above the chip threshold
    /// (4 lines, or 2 in compact mode), or larger than
    /// [`PASTE_CHIP_DISPLAY_BYTES`] bytes, an atomic `[Pasted: N lines]`
    /// element is created.  Trailing newlines in the inline path are
    /// preserved so that split batches (Windows) accumulate correctly.
    /// Repasting a chip's exact content while the cursor is on or right
    /// after it expands that chip instead of inserting a duplicate.
    pub fn handle_paste(&mut self, text: &str) -> PromptEvent {
        if text.is_empty() {
            return PromptEvent::Ignored;
        }
        self.post_insert_image_preview = None;

        let text = normalize_cr(text);
        let text = &text;
        let replacing_selection = self.textarea.selection_range().is_some();

        // Repaste-to-expand: "paste didn't do what I want? paste again."
        // Requires exact byte equality after the same canonicalization the
        // original insertion applied (normalize_cr above + the textarea's
        // tab expansion) — e.g. a trailing-newline difference is a
        // different paste and takes the normal path below.
        if !replacing_selection
            && let Some(elem) = self.paste_element_near_cursor()
            && self.textarea.text()[elem.range.clone()] == *self.textarea.expand_tabs(text)
        {
            let id = elem.id;
            self.expand_element(id);
            return PromptEvent::Edited;
        }

        if replacing_selection {
            self.textarea.begin_undo_group();
            self.textarea.delete_selection();
        }

        // Count actual content lines. `.lines()` treats a trailing newline as
        // optional, so "hello\n" counts as 1 line — exactly what we want.
        let line_count = text.lines().count();

        let chip_min_lines = if self.compact { 2 } else { 4 };
        // Chip when the paste is multi-line past the threshold, or large by byte
        // size. The label prefers size for large pastes (a 1 MB paste reads
        // better as "1.0 MB" than "N lines", regardless of line count); smaller
        // multi-line pastes keep the line count.
        let by_lines = line_count >= chip_min_lines;
        let by_bytes = text.len() > PASTE_CHIP_DISPLAY_BYTES;
        if by_lines || by_bytes {
            let display = if by_bytes {
                paste_chip_display_bytes(text.len())
            } else {
                paste_chip_display(line_count)
            };
            self.textarea
                .insert_element(text, KIND_PASTE, Some(display));
        } else {
            self.textarea.insert_str(text);
        }
        if replacing_selection {
            self.textarea.end_undo_group();
            self.sync_images_with_textarea();
        }

        PromptEvent::Edited
    }

    // ── Image chip support ──────────────────────────────────────────

    /// Maximum number of image chips allowed in a single prompt (v1).
    pub const IMAGE_CAP: usize = 10;

    /// Toast text shown when an image insertion is rejected because the
    /// prompt already holds [`Self::IMAGE_CAP`] images.
    #[allow(dead_code)]
    pub(crate) fn cap_reached_toast() -> String {
        format!("Image limit reached (max {})", Self::IMAGE_CAP)
    }

    /// Insert a pasted image as an atomic `[Image #N]` chip.
    ///
    /// Returns `Err` with a user-facing message if the image is rejected
    /// (cap reached or too small). The caller should show a toast.
    pub fn insert_image(&mut self, image: PastedImage) -> Result<(), String> {
        if self.images.len() >= Self::IMAGE_CAP {
            return Err(Self::cap_reached_toast());
        }
        // Backend APIs reject images with either side < 8 px.
        const MIN_SIDE: u32 = 8;
        if let Some((w, h)) = image.preview_dimensions()
            && (w < MIN_SIDE || h < MIN_SIDE)
        {
            return Err(format!(
                "Image too small ({w}×{h}). Must be at least {MIN_SIDE}×{MIN_SIDE} pixels."
            ));
        }

        self.image_counter += 1;
        let display_number = self.image_counter;
        let placeholder = prompt_images::display_text(display_number);
        let display_line = chip_line(format!("Image #{display_number}"));

        let buf_len_before = self.textarea.text().len();
        self.textarea.begin_undo_group();
        let element_id = self
            .textarea
            .insert_element(&placeholder, KIND_IMAGE, Some(display_line));
        self.textarea.insert_str(" ");
        self.textarea.end_undo_group();
        self.post_insert_image_preview = Some((element_id, self.textarea.cursor()));
        let buf_len_after = self.textarea.text().len();

        tracing::debug!(
            target: PROMPT_IMAGES_TRACING_TARGET,
            display_number,
            element_id = ?element_id,
            images_len_after = self.images.len() + 1,
            image_counter_after = self.image_counter,
            buf_len_before,
            buf_len_after,
            "prompt_widget: insert_image",
        );

        let mut img = image;
        img.element_id = element_id;
        img.display_number = display_number;
        self.images.push(img);

        Ok(())
    }

    /// Insert text replacing the selection; resyncs images (a selection can swallow chips).
    pub fn insert_replacing_selection(&mut self, text: &str) {
        let replaced_selection = self.textarea.selection_range().is_some();
        self.textarea.insert_str_replacing_selection(text);
        if replaced_selection {
            self.sync_images_with_textarea();
        }
    }

    fn sync_images_with_textarea(&mut self) {
        use std::collections::HashMap;
        use std::collections::hash_map::Entry;

        let live_image_elements: Vec<(ElementId, std::ops::Range<usize>)> = self
            .textarea
            .elements()
            .iter()
            .filter(|e| e.kind == KIND_IMAGE)
            .map(|e| (e.id, e.range.clone()))
            .collect();
        let stored_len_before = self.images.len();
        let counter_before = self.image_counter;
        let stash_len_before = self.image_undo_stash.len();

        // Primary lookup keyed by `ElementId` (unique by construction).
        // Keying by `display_number` would silently drop one record when two
        // images share a number; the warn branch below makes a future
        // duplicate-id regression visible instead of hiding it.
        let mut stored_by_id: HashMap<ElementId, PastedImage> =
            HashMap::with_capacity(self.images.len());
        for img in self.images.drain(..) {
            let id = img.element_id;
            let display_number = img.display_number;
            match stored_by_id.entry(id) {
                Entry::Vacant(v) => {
                    v.insert(img);
                }
                Entry::Occupied(_) => {
                    tracing::warn!(
                        target: PROMPT_IMAGES_TRACING_TARGET,
                        element_id = ?id,
                        display_number,
                        "sync_images_with_textarea: duplicate element_id \
                         in stored images — dropping later entry to \
                         preserve the first (invariant violation)",
                    );
                }
            }
        }

        // Fallback for elements restored by an undo/redo cycle: the textarea
        // may have re-issued a fresh `ElementId` for the restored chip, so an
        // `element_id` lookup misses. Keying by `display_number` is safe here
        // because the monotonic counter never recycles numbers within a
        // prompt lifetime; a collision-warn keeps any future regression visible.
        let mut stash_by_number: HashMap<usize, PastedImage> =
            HashMap::with_capacity(self.image_undo_stash.len());
        for img in self.image_undo_stash.drain(..) {
            let display_number = img.display_number;
            let element_id = img.element_id;
            match stash_by_number.entry(display_number) {
                Entry::Vacant(v) => {
                    v.insert(img);
                }
                Entry::Occupied(_) => {
                    tracing::warn!(
                        target: PROMPT_IMAGES_TRACING_TARGET,
                        display_number,
                        element_id = ?element_id,
                        "sync_images_with_textarea: duplicate display_number \
                         in undo stash — dropping later entry",
                    );
                }
            }
        }

        let mut synced = Vec::with_capacity(live_image_elements.len());

        for (id, range) in &live_image_elements {
            // Primary: match by element_id (collision-free, stable
            // for non-undo/redo edits).
            if let Some(mut img) = stored_by_id.remove(id) {
                // Refresh display_number from the live buffer text in
                // case the chip was renumbered externally (sanity).
                let parsed = parse_image_display_number(&self.textarea.text()[range.clone()])
                    .unwrap_or(img.display_number);
                img.element_id = *id;
                img.display_number = parsed;
                synced.push(img);
                continue;
            }
            // Fallback: redo path — match by display_number from the
            // stash, then rebind to the freshly-issued `element_id`.
            let display_number =
                parse_image_display_number(&self.textarea.text()[range.clone()]).unwrap_or(0);
            if let Some(mut img) = stash_by_number.remove(&display_number) {
                img.element_id = *id;
                img.display_number = display_number;
                synced.push(img);
            }
        }

        // Images whose elements are gone move to the undo stash so they
        // can be recovered if the user redoes. Explicit resets (`set_text`,
        // Ctrl-C) clear the stash separately.
        //
        // Cap the stash at `IMAGE_CAP * 2` so a long edit session
        // can't unboundedly grow it. Evicted entries get their temp
        // files cleaned up (mirrors what `clear()` does on submit).
        // Oldest-first eviction by `display_number` keeps the most
        // recent deletions recoverable, since redo typically reverses
        // the most recent undo.
        let mut new_stash: Vec<PastedImage> = stored_by_id
            .into_values()
            .chain(stash_by_number.into_values())
            .collect();
        let stash_cap = Self::IMAGE_CAP * 2;
        if new_stash.len() > stash_cap {
            // Oldest = lowest `display_number` (monotonic within a
            // prompt lifetime). Sort ascending, then drop the front.
            new_stash.sort_by_key(|img| img.display_number);
            let evict_count = new_stash.len() - stash_cap;
            for img in new_stash.drain(..evict_count) {
                // stash eviction
                prompt_images::cleanup_temp_file(&img);
            }
        }
        self.image_undo_stash = new_stash;
        self.images = synced;
        if self
            .hovered_image_element_id
            .is_some_and(|id| !self.images.iter().any(|img| img.element_id == id))
        {
            self.hovered_image_element_id = None;
        }
        if self
            .post_insert_image_preview
            .is_some_and(|(id, _)| !self.images.iter().any(|img| img.element_id == id))
        {
            self.post_insert_image_preview = None;
        }
        // Monotonic high-water mark within a prompt lifetime: the counter only
        // ever advances upward, so deleting a chip never lets a later insert
        // reuse a number that already appeared. The counter is reset only by
        // explicit prompt clears (`set_text("")`, Ctrl+C) which call
        // [`prompt_images::clear`].
        self.image_counter = self.image_counter.max(images_high_water(&self.images));

        // Only log when the stored count or stash size actually changed,
        // otherwise every post-paste keystroke would emit a debug line.
        let stash_len_after = self.image_undo_stash.len();
        let stored_changed = self.images.len() != stored_len_before;
        let stash_changed = stash_len_after != stash_len_before;
        if stored_changed || stash_changed {
            let stashed_numbers: Vec<usize> = self
                .image_undo_stash
                .iter()
                .map(|img| img.display_number)
                .collect();
            tracing::debug!(
                target: PROMPT_IMAGES_TRACING_TARGET,
                live_count = live_image_elements.len(),
                stored_len_before,
                synced_len = self.images.len(),
                counter_before,
                counter_after = self.image_counter,
                stash_len_before,
                stash_len_after,
                ?stashed_numbers,
                "prompt_widget: sync_images_with_textarea",
            );
        }
    }

    /// Look up the image record for the element the cursor is currently on.
    ///
    /// Uses `element_at_cursor()` which matches when `cursor_pos` falls
    /// within `[range.start, range.end)`. For atomic elements the cursor
    /// sits at `range.start` when the user navigates onto the chip (LEFT
    /// arrow jumps there). Positions at or past `range.end` (the trailing
    /// separator space and beyond) are intentionally excluded so the
    /// overlay dismisses as soon as the cursor leaves the chip.
    pub fn image_at_cursor(&self) -> Option<&PastedImage> {
        let elem = self.textarea.element_at_cursor()?;
        if elem.kind != KIND_IMAGE {
            return None;
        }
        self.images.iter().find(|img| img.element_id == elem.id)
    }

    /// Image element the cursor is on, or exactly at its right edge.
    ///
    /// An on-chip match of any kind wins (the chip under the cursor owns
    /// the position), so a cursor sitting on a non-image chip yields
    /// `None` even when an image chip ends at the cursor.
    fn image_element_near_cursor(&self) -> Option<&rutui_ratatui_textarea::TextElement> {
        if let Some(elem) = self.textarea.element_at_cursor() {
            return (elem.kind == KIND_IMAGE).then_some(elem);
        }
        let cursor = self.textarea.cursor();
        // Right edge only — not the spacer after (`range.end + 1`).
        self.textarea
            .elements()
            .iter()
            .filter(|e| e.kind == KIND_IMAGE)
            .find(|e| e.range.end == cursor)
    }

    /// Image record for the preview overlay: cursor, hover, or fresh insertion.
    fn image_for_preview(&self) -> Option<&PastedImage> {
        if let Some(elem) = self.image_element_near_cursor() {
            return self.images.iter().find(|img| img.element_id == elem.id);
        }
        if let Some(id) = self.hovered_image_element_id {
            return self.images.iter().find(|img| img.element_id == id);
        }
        let (id, cursor) = self.post_insert_image_preview?;
        if self.cursor() != cursor {
            return None;
        }
        self.images.iter().find(|img| img.element_id == id)
    }

    /// Drain all prompt-side images for submission.
    ///
    /// Reconciles against live `TextArea` elements first so deleted chips
    /// are never included. Returns the drained images; the prompt-side
    /// storage is left empty.
    pub fn drain_images(&mut self) -> Vec<PastedImage> {
        self.post_insert_image_preview = None;
        let live_ids: std::collections::HashSet<_> = self
            .textarea
            .elements()
            .iter()
            .filter(|e| e.kind == KIND_IMAGE)
            .map(|e| e.id)
            .collect();
        prompt_images::reconcile(&mut self.images, &live_ids);
        std::mem::take(&mut self.images)
    }

    /// Buffer text with `[Image #N]` chip placeholders removed, for
    /// text-only surfaces (e.g. question/permission feedback) that must
    /// not leak image tokens onto the wire.
    pub fn text_without_image_chips(&self) -> String {
        let text = self.textarea.text();
        let mut out = String::with_capacity(text.len());
        let mut prev_end = 0usize;
        for elem in self.textarea.elements() {
            if elem.kind != KIND_IMAGE {
                continue;
            }
            out.push_str(&text[prev_end..elem.range.start]);
            prev_end = elem.range.end;
        }
        out.push_str(&text[prev_end..]);
        out
    }

    /// Replace the prompt-side image list and restore `image_counter`.
    /// The caller must restore chip elements beforehand to re-register the
    /// `KIND_IMAGE` textarea elements (with stored byte ranges) so
    /// hover/click behaviour works.
    ///
    /// **Pairing**: each `PastedImage` is bound to the live textarea
    /// element whose chip text parses to the same `display_number`.
    /// Identity-based pairing is robust against the two ways the
    /// `images` vector and the textarea element list can diverge:
    ///
    /// 1. `textarea.elements()` is sorted by buffer position
    ///    (`TextArea::add_element` sorts on every insert), whereas
    ///    `self.images` is populated by `insert_image::push` in
    ///    chronological insertion order. Inserting a second image at
    ///    the start of the buffer (cursor-at-Home) is enough to make
    ///    a positional zip mis-bind both records.
    /// 2. Two chips with identical placeholder widths (e.g.
    ///    `[Image #1]` and `[Image #2]`, both 10 bytes) would
    ///    collide under a byte-length match.
    ///
    /// `display_number` is unique within a prompt lifetime (the
    /// monotonic counter never recycles within one prompt), so the
    /// identity lookup is collision-free.
    ///
    /// Excess `images` past `IMAGE_CAP` are dropped with a warn:
    /// defence-in-depth for sessions that somehow accumulated more
    /// than the cap.
    pub fn set_images(&mut self, mut images: Vec<PastedImage>) {
        if images.len() > Self::IMAGE_CAP {
            tracing::warn!(
                target: PROMPT_IMAGES_TRACING_TARGET,
                count = images.len(),
                cap = Self::IMAGE_CAP,
                "set_images: input exceeds IMAGE_CAP; truncating",
            );
            images.truncate(Self::IMAGE_CAP);
        }

        // Build a (display_number → ElementId) lookup over the live
        // KIND_IMAGE elements. Linear scan (cap ≤ 10 in practice).
        //
        // Defence in depth: the monotonic-counter contract
        // guarantees `display_number` is unique within a prompt
        // lifetime, so duplicates here imply a contract violation
        // upstream (e.g. a future refactor that recycles numbers).
        // Surface a `tracing::warn!` with the existing image
        // tracing target so the regression is visible without
        // crashing the TUI; the lookup below still picks the first
        // match so the visible binding is at least deterministic.
        let buf = self.textarea.text();
        let mut by_number: Vec<(usize, ElementId)> = Vec::new();
        for elem in self.textarea.elements() {
            if elem.kind != KIND_IMAGE {
                continue;
            }
            if let Some(dn) = parse_image_display_number(&buf[elem.range.clone()]) {
                if by_number.iter().any(|(seen_dn, _)| *seen_dn == dn) {
                    tracing::warn!(
                        target: PROMPT_IMAGES_TRACING_TARGET,
                        display_number = dn,
                        element_id = ?elem.id,
                        "set_images: duplicate display_number across live KIND_IMAGE \
                         elements — monotonic-counter contract violated; first match \
                         wins the binding",
                    );
                    continue;
                }
                by_number.push((dn, elem.id));
            }
        }

        for img in &mut images {
            if let Some(&(_, eid)) = by_number.iter().find(|(dn, _)| *dn == img.display_number) {
                img.element_id = eid;
            } else {
                tracing::warn!(
                    target: PROMPT_IMAGES_TRACING_TARGET,
                    display_number = img.display_number,
                    "set_images: no live element matched display_number; \
                     PastedImage remains dangling until the next sync",
                );
            }
        }
        // Align with the monotonic contract in `sync_images_with_textarea`:
        // the counter only ever advances upward within a prompt lifetime.
        self.image_counter = self.image_counter.max(images_high_water(&images));
        self.images = images;
    }

    /// Expose the underlying textarea for element access.
    pub fn textarea(&self) -> &rutui_ratatui_textarea::TextArea {
        &self.textarea
    }

    /// Buffer text of `elem` if it is a paste chip (`KIND_PASTE`).
    fn paste_text(&self, elem: &TextElement) -> Option<&str> {
        (elem.kind == KIND_PASTE).then(|| &self.textarea.text()[elem.range.clone()])
    }

    /// Check if the cursor is currently on a paste element (strict on-chip
    /// match, mirroring the Enter-to-expand targeting in
    /// [`Self::try_element_interaction`]).
    ///
    /// Returns the element's buffer text if so.
    pub fn paste_element_at_cursor(&self) -> Option<&str> {
        self.paste_text(self.textarea.element_at_cursor()?)
    }

    /// Paste element the cursor is on or immediately to the right of —
    /// where [`TextArea::insert_element`] leaves it right after a paste.
    ///
    /// An on-chip match of any kind wins (the chip under the cursor owns
    /// the position), so a cursor sitting on a non-paste chip yields
    /// `None` even when a paste chip ends at the cursor.
    fn paste_element_near_cursor(&self) -> Option<&TextElement> {
        let elem = self.textarea.element_at_cursor().or_else(|| {
            let cursor = self.textarea.cursor();
            self.textarea
                .elements()
                .iter()
                .find(|e| e.range.end == cursor)
        })?;
        (elem.kind == KIND_PASTE).then_some(elem)
    }

    /// Paste element text for the preview overlay.
    ///
    /// Shows the moment a chip is created (cursor right after it) as well
    /// as with the cursor on it. Display-only: Enter handling keeps the
    /// strict on-chip match, so Enter right after a chip keeps its normal
    /// behavior (submit or newline) instead of expanding the chip.
    fn paste_element_for_preview(&self) -> Option<&str> {
        self.paste_text(self.paste_element_near_cursor()?)
    }

    /// Expand hint for the paste preview overlay, honest per position:
    /// ON the chip Enter expands it; right after the chip Enter submits,
    /// so the affordance advertised there is pasting the content again.
    /// Double-click expands from either position (a click moves the
    /// cursor onto the chip first).
    fn paste_preview_hint(&self, theme: &Theme) -> Line<'static> {
        let dim = Style::default().fg(theme.gray);
        // Chord deliberately deviates from the tips' text_secondary to a
        // real accent: oscura/rosepine alias text_secondary and gray to
        // the box's own content/border colors, so only an accent pops.
        let chord = Style::default()
            .fg(theme.fuzzy_accent)
            .add_modifier(Modifier::BOLD);
        let action = if self.paste_element_at_cursor().is_some() {
            "enter"
        } else {
            "paste again"
        };
        Line::from(vec![
            Span::styled(action, chord),
            Span::styled(" or ", dim),
            Span::styled("double-click", chord),
            Span::styled(" to expand", dim),
        ])
    }

    /// Inline (expand) element `id` into plain buffer text. A single
    /// undoable step (see [`TextArea::inline_element`]). Embeddings wrap
    /// the public expand entry points to refresh their `@`-completion
    /// context afterwards.
    fn expand_element(&mut self, id: ElementId) {
        self.textarea.inline_element(id);
    }

    /// Expand the paste chip under the cursor (mouse double-click path).
    ///
    /// Strict on-chip match: a cursor merely adjacent to a chip (e.g.
    /// right after it) must not expand it. Returns `true` if a paste
    /// chip was expanded.
    pub fn expand_paste_element_at_cursor(&mut self) -> bool {
        let Some(elem) = self.textarea.element_at_cursor() else {
            return false;
        };
        if elem.kind != KIND_PASTE {
            return false;
        }
        let id = elem.id;
        self.expand_element(id);
        true
    }

    /// Try to interact with an element at the cursor.
    ///
    /// - Enter on a paste element → inline (expand) it.
    /// - Enter on an image chip → signal the caller to open preview
    ///   (does NOT inline the placeholder text).
    ///
    /// Returns `Some(interaction)` if handled, `None` otherwise.
    pub fn try_element_interaction(&mut self, key: &KeyEvent) -> Option<ElementInteraction> {
        let elem = self.textarea.element_at_cursor()?;
        if !key!(Enter).matches(key) {
            return None;
        }
        match elem.kind {
            k if k == KIND_PASTE => {
                let id = elem.id;
                self.expand_element(id);
                Some(ElementInteraction::Inlined)
            }
            k if k == KIND_IMAGE => Some(ElementInteraction::ImagePreview),
            _ => None,
        }
    }
}

impl PromptWidget {
    /// Render the prompt widget.
    ///
    /// When `style.chrome` is true, renders the full block layout:
    /// `accent(┃) | hpad | content | hpad` with background fill.
    /// When false, renders only the content (prefix + textarea + info).
    ///
    /// Embeddings paint their own overlay decorations after this call
    /// (slash command highlights, ghost text) using
    /// [`PromptRenderResult::ta_area`] + [`PromptWidget::textarea_state`].
    pub fn draw(
        &mut self,
        buf: &mut Buffer,
        area: Rect,
        overlay_area: Option<Rect>,
        style: &PromptStyle,
        info: Option<&PromptInfo>,
        voice: Option<VoicePromptOverlay>,
    ) -> PromptRenderResult {
        if area.height == 0 || area.width < 4 {
            return PromptRenderResult {
                cursor_pos: None,
                post_flush_escapes: None,
                ta_area: Rect::default(),
            };
        }

        let theme = Theme::current();
        let bg = style.bg.color(theme.bg_base);

        let border_color = style.border_color_override.unwrap_or(if style.focused {
            theme.prompt_border_active
        } else {
            theme.prompt_border
        });

        // Fill entire area with fg + bg so every cell has RGB colors (needed for blending)
        buf.set_style(area, Style::default().fg(theme.text_primary).bg(bg));

        // Compute content area (inside chrome if enabled).
        let content_area = if style.chrome {
            let accent_w = if style.show_accent_line { 1u16 } else { 0 };
            if style.show_accent_line {
                let accent_color = style.accent_color(&theme);
                for y in area.y..area.y + area.height {
                    if let Some(cell) = buf.cell_mut((area.x, y)) {
                        cell.set_symbol(rutui_theme::glyphs::accent_bar());
                        cell.set_style(Style::default().fg(accent_color).bg(bg));
                    }
                }
            }
            Rect {
                x: area.x + accent_w + style.chrome_pad_left,
                y: area.y,
                width: area
                    .width
                    .saturating_sub(accent_w + style.chrome_pad_left + style.chrome_pad_right),
                height: area.height,
            }
        } else {
            area
        };

        // Split content: vpad_top + text + info_block
        let vpad_top = style.vpad_top;
        let info_block = style.info_block(info.is_some());
        let chunks = Layout::vertical([
            Constraint::Length(vpad_top),
            Constraint::Min(1),
            Constraint::Length(info_block),
        ])
        .split(content_area);

        let text_area_rect = chunks[1];

        // Top divider: ╭──────────╮
        if vpad_top > 0 && style.chrome && style.show_borders {
            let div_style = Style::default().fg(border_color).bg(bg);
            let div_y = chunks[0].y;
            let left_x = area.x;
            let right_x = area.x + area.width.saturating_sub(1);
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, div_y)) {
                    let ch = if x == left_x {
                        '\u{256d}' // ╭
                    } else if x == right_x {
                        '\u{256e}' // ╮
                    } else {
                        '\u{2500}' // ─
                    };
                    cell.set_char(ch);
                    cell.set_style(div_style);
                }
            }

            // Session title inlined in the divider (` title `, right-aligned
            // ending 2 cells before ╮) in the shared chrome-caption style;
            // the pad spaces blank the adjacent `─`.
            if let Some(title) = style
                .title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                // Corners plus 2-cell insets on both sides stay plain border.
                let max_w = area.width.saturating_sub(6);
                if max_w >= 6 {
                    let label = format!(" {title} ");
                    let trunc = render::line_utils::truncate_str(&label, max_w as usize);
                    let label_w = unicode_width::UnicodeWidthStr::width(trunc.as_str()) as u16;
                    let x = area.x + area.width.saturating_sub(3 + label_w);
                    buf.set_string(
                        x,
                        div_y,
                        &trunc,
                        Self::chrome_caption_style(bg, &theme, style.focused),
                    );
                }
            }
        }

        // Render prefix on first text row: search icon when history search is active, else ❯.
        let prefix_w = if style.show_prefix { PREFIX_WIDTH } else { 0 };
        if style.show_prefix && text_area_rect.width > PREFIX_WIDTH {
            let (prefix_str, accent_color) =
                if self.history_search.is_active() && !self.history_search.is_browse() {
                    // Search-mode indicator — highest prefix priority. Browse
                    // mode keeps the normal prefix: the composer holds the
                    // populated entry (a real draft), not a filter query.
                    ("? ", theme.accent_user)
                } else if let Some((p, c)) = style.prefix_override {
                    // Caller override (e.g., bash mode `! ` in yellow).
                    (p, c)
                } else {
                    (
                        rutui_theme::glyphs::prompt_arrow(),
                        style.accent_color(&theme),
                    )
                };
            buf.set_string(
                text_area_rect.x,
                text_area_rect.y,
                prefix_str,
                Style::default().fg(accent_color).bg(bg),
            );
        }

        // TextArea content
        let ta_area = Rect {
            x: text_area_rect.x + prefix_w,
            y: text_area_rect.y,
            width: text_area_rect.width.saturating_sub(prefix_w),
            height: text_area_rect.height,
        };
        self.textarea_area = ta_area;

        (&self.textarea).render_ref(ta_area, buf, &mut self.textarea_state);

        // Chip bg remap (see `PromptBg::Panel`): chip `Line`s bake in
        // `paste_bg` at paste time and the same element can render on
        // multiple surfaces, so restyle at paint time.
        if matches!(style.bg, PromptBg::Panel(_)) && bg != theme.paste_bg {
            for y in ta_area.top()..ta_area.bottom() {
                for x in ta_area.left()..ta_area.right() {
                    if let Some(cell) = buf.cell_mut((x, y))
                        && cell.bg == theme.paste_bg
                    {
                        cell.bg = bg;
                    }
                }
            }
        }

        // Interim STT: muted italic overlay (not in the textarea). Finalized
        // text remains the real, editable draft.
        let voice_interim_shown = if let Some(v) = voice
            && let Some(interim) = v.interim.filter(|t| !t.trim().is_empty())
            && ta_area.width > 0
            && ta_area.height > 0
        {
            let interim_fg =
                render::color::blend_color(bg, theme.text_secondary, 0.7).unwrap_or(theme.gray);
            let interim_style = Style::default()
                .fg(interim_fg)
                .bg(bg)
                .add_modifier(Modifier::ITALIC);
            if self.textarea.text().is_empty() {
                let lines =
                    wrap_voice_interim(interim, ta_area.width as usize, ta_area.height as usize);
                for (i, line) in lines.iter().enumerate() {
                    buf.set_string(ta_area.x, ta_area.y + i as u16, line, interim_style);
                }
            } else {
                // Ghost suffix after the finalized draft (not at the caret).
                let end = self.textarea.text().len();
                if let Some((start_x, row_y)) =
                    self.textarea
                        .screen_position_of(end, ta_area, self.textarea_state)
                {
                    let display = format!(" {interim}");
                    let avail = (ta_area.x + ta_area.width).saturating_sub(start_x) as usize;
                    if avail > 0 {
                        let truncated = render::line_utils::truncate_str(&display, avail);
                        buf.set_string(start_x, row_y, &truncated, interim_style);
                    }
                }
            }
            true
        } else {
            false
        };

        // Placeholder text when empty (unfocused, or opted in while focused).
        if self.textarea.text().is_empty()
            && ta_area.width > 0
            && (!style.focused || style.placeholder_when_focused)
            && !voice_interim_shown
        {
            let placeholder = style.placeholder_override.unwrap_or("Build anything");
            // `set_string` clips at the buffer edge, not at the textarea, so a placeholder longer than the box would paint over its border.
            let truncated = render::line_utils::truncate_str(placeholder, ta_area.width as usize);
            buf.set_string(
                ta_area.x,
                ta_area.y,
                &truncated,
                Style::default().fg(theme.gray).bg(bg),
            );
        }

        // Side borders: │ on left and right of each text row.
        if style.chrome && style.show_borders && area.width >= 2 {
            let div_style = Style::default().fg(border_color).bg(bg);
            let left_x = area.x;
            let right_x = area.x + area.width.saturating_sub(1);
            for y in text_area_rect.y..text_area_rect.y + text_area_rect.height {
                if let Some(cell) = buf.cell_mut((left_x, y)) {
                    cell.set_char('\u{2502}'); // │
                    cell.set_style(div_style);
                }
                if let Some(cell) = buf.cell_mut((right_x, y)) {
                    cell.set_char('\u{2502}'); // │
                    cell.set_style(div_style);
                }
            }
        }

        // Bottom divider: ╰──────────grok-3 · flags──╯
        // Guard on actual allocated height, not requested `info_block`: during
        // resize the layout may squeeze the info block to 0 rows, leaving
        // chunks[2].y past the buffer boundary.
        if info_block > 0 && style.chrome && style.show_borders && chunks[2].height > 0 {
            let div_y = chunks[2].y;
            let div_style = Style::default().fg(border_color).bg(bg);
            let left_x = area.x;
            let right_x = area.x + area.width.saturating_sub(1);
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, div_y)) {
                    let ch = if x == left_x {
                        '\u{2570}' // ╰
                    } else if x == right_x {
                        '\u{256f}' // ╯
                    } else {
                        '\u{2500}' // ─
                    };
                    cell.set_char(ch);
                    cell.set_style(div_style);
                }
            }
            // A blank info line still writes its padding spaces, which would punch holes in the divider it sits on.
            if let Some(info) = info.filter(|i| !i.is_blank()) {
                let info_rect = Rect {
                    x: content_area.x,
                    y: div_y,
                    width: content_area.width,
                    height: 1,
                };
                self.render_info_line(buf, info_rect, info, bg, &theme, style.focused);
            }
        }

        // Unfocused dimming: blend fg toward bg (bg already precomputed above).
        // Skip when the bg is overridden — the prompt is inline in another widget.
        if !style.focused && style.bg == PromptBg::Default {
            // Dim only the content inside the box (skip all border chars).
            let dim_area = Rect {
                x: area.x + 1,
                y: content_area.y + vpad_top,
                width: area.width.saturating_sub(2),
                height: content_area.height.saturating_sub(vpad_top + info_block),
            };
            render::color::blend_area(buf, dim_area, Some((bg, 0.66)), None);
        }

        // Finalized draft stays editable during voice; hide the caret only when
        // the box is empty and interim is standing in for it.
        let hide_caret_for_empty_interim = self.textarea.text().is_empty()
            && voice.is_some_and(|v| v.interim.is_some_and(|t| !t.trim().is_empty()));
        let cursor_pos = if style.focused && !hide_caret_for_empty_interim {
            self.textarea
                .cursor_pos_with_state(ta_area, self.textarea_state)
        } else {
            None
        };

        // Paste preview overlay: show when cursor is on or right after a paste element
        if style.focused
            && let Some(overlay) = overlay_area
            && let Some(paste_text) = self.paste_element_for_preview()
        {
            let preview_style = PreviewStyle::new(theme.paste_bg, theme.paste_fg, theme.paste_dim);
            let config = PreviewConfig {
                hint: Some(self.paste_preview_hint(&theme)),
                ..Default::default()
            };
            render_preview_overlay(buf, overlay, paste_text, preview_style, config);
        }

        let mut post_flush_escapes = None;
        let preview_image = if style.image_preview && style.focused && overlay_area.is_some() {
            self.image_for_preview()
        } else {
            None
        };

        if let Some(image) = preview_image {
            let Some(overlay) = overlay_area else {
                return PromptRenderResult {
                    cursor_pos,
                    post_flush_escapes: None,
                    ta_area,
                };
            };
            post_flush_escapes = render_image_overlay(
                buf,
                overlay,
                image,
                theme.paste_bg,
                theme.paste_fg,
                theme.paste_dim,
            );
        }

        // Clear ID 1 whenever this frame did not replace it.
        if post_flush_escapes.is_none() {
            post_flush_escapes = terminal::overlay::clear();
        }

        PromptRenderResult {
            cursor_pos,
            post_flush_escapes,
            ta_area,
        }
    }

    /// Caption style for text embedded in the prompt's border chrome — the
    /// info line's model name on `╰─╯` and the session title on `╭─╮`:
    /// dimmed secondary text over the prompt bg, fading further when
    /// unfocused, so both borders read as one chrome.
    fn chrome_caption_style(bg: ratatui::style::Color, theme: &Theme, focused: bool) -> Style {
        let opacity = if focused { 0.6 } else { 0.4 };
        let fg =
            render::color::blend_color(bg, theme.text_secondary, opacity).unwrap_or(theme.gray);
        Style::default().fg(fg).bg(bg)
    }

    /// Render the info line: left-aligned `model_name · flag1 · flag2`, right-aligned `multiline`.
    ///
    /// `pub` so embedding widgets (e.g. a dashboard dispatch box with its
    /// own chrome) can paint an identical model + mode indicator on their
    /// bottom border. Does not read `self`.
    pub fn render_info_line(
        &self,
        buf: &mut Buffer,
        area: Rect,
        info: &PromptInfo,
        bg: ratatui::style::Color,
        theme: &Theme,
        focused: bool,
    ) {
        if area.height == 0 {
            return;
        }

        // When the prompt is unfocused, fade info-line content further toward
        // bg so it follows the same focused/unfocused dimming as the prompt
        // border. Model name and flag color use a higher opacity than the
        // separator so they remain readable.
        let sep_opacity = if focused { 1.0 } else { 0.6 };
        let flag_opacity = if focused { 0.75 } else { 0.5 };

        let model_style = Self::chrome_caption_style(bg, theme, focused);
        let sep_fg = if focused {
            theme.gray_dim
        } else {
            render::color::blend_color(bg, theme.gray_dim, sep_opacity).unwrap_or(theme.gray_dim)
        };
        let sep_style = Style::default().fg(sep_fg).bg(bg);
        let flag_style = Style::default().fg(theme.gray).bg(bg);

        // Left side: model name + flags. Wrap with leading/trailing spaces
        // so the rendered cells adjacent to the corner borders (╰ / ╯) are
        // blanked out instead of showing the underlying `─` glyphs from the
        // bottom-border fill — giving 1 cell of visual padding on each side.
        let pad_style = Style::default().bg(bg);
        let mut left_spans = vec![Span::styled(" ", pad_style)];
        if let Some(warning) = info.usage_warning {
            let fg = if info.usage_warning_critical {
                theme.warning
            } else {
                sep_fg
            };
            let warning_style = Style::default().fg(fg).bg(bg);
            left_spans.push(Span::styled(warning.to_owned(), warning_style));
            left_spans.push(Span::styled(" · ", sep_style));
        }
        left_spans.push(Span::styled(info.model_name, model_style));
        for flag in info.flags {
            left_spans.push(Span::styled(" · ", sep_style));
            let mut style = if let Some(color) = flag.color {
                if flag.bold {
                    // Bold flags use full color for visibility.
                    Style::default().fg(color).bg(bg)
                } else {
                    let dimmed =
                        render::color::blend_color(bg, color, flag_opacity).unwrap_or(theme.gray);
                    Style::default().fg(dimmed).bg(bg)
                }
            } else if flag.bold {
                Style::default().fg(theme.text_primary).bg(bg)
            } else if focused {
                flag_style
            } else {
                let dimmed =
                    render::color::blend_color(bg, theme.gray, flag_opacity).unwrap_or(theme.gray);
                Style::default().fg(dimmed).bg(bg)
            };
            if flag.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            left_spans.push(Span::styled(flag.text, style));
        }
        // Trailing pad mirrors the leading pad above.
        left_spans.push(Span::styled(" ", pad_style));

        // Build right-side spans: "multiline" indicator.
        let mut right_spans: Vec<Span<'static>> = Vec::new();
        if info.multiline {
            right_spans.push(Span::styled("multiline", flag_style));
        }

        if !right_spans.is_empty() {
            // Pad the right side so it doesn't sit flush against the ╯ corner.
            right_spans.push(Span::styled(" ", pad_style));
            let right_line = Line::from(right_spans);
            let right_w = right_line.width() as u16;
            let left_line = Line::from(left_spans);
            let left_w = (left_line.width() as u16).min(area.width.saturating_sub(right_w + 1));
            // Right-align both parts: [left][gap][right]
            let total_w = left_w + 1 + right_w; // 1 for gap
            let x = area.x + area.width.saturating_sub(total_w);
            buf.set_line_safe(x, area.y, &left_line, left_w);
            let rx = area.x + area.width.saturating_sub(right_w);
            buf.set_line_safe(rx, area.y, &right_line, right_w);
        } else {
            // Right-align: clamp to area width so text doesn't overflow borders.
            let line = Line::from(left_spans);
            let text_w = (line.width() as u16).min(area.width);
            let x = area.x + area.width.saturating_sub(text_w);
            buf.set_line_safe(x, area.y, &line, text_w);
        }
    }
}

impl Default for PromptWidget {
    fn default() -> Self {
        Self::new()
    }
}

// ── Element display helpers ────────────────────────────────────────────

/// Extract the display number from an `[Image #N]` or
/// `[Image #N: <path>]` chip placeholder. Returns `None` when the
/// input doesn't match either form.
///
/// Path-suffix support is required because chips with a `source_path`
/// (file-path pastes / Finder Cmd+C) appear in the buffer as
/// `[Image #3: /path/to/file.png]`. Without it, the undo/redo
/// recovery path in `sync_images_with_textarea` falls back to
/// `stash_by_number.remove(0)` and never finds the original entry.
///
/// Permissive parsing: a literal space after `#` (e.g. `[Image # 1]`)
/// is tolerated via `trim()`. The canonical emitter never produces
/// that form; the permissiveness is intentional defence in depth.
fn parse_image_display_number(placeholder: &str) -> Option<usize> {
    let inner = placeholder.strip_prefix("[Image #")?.strip_suffix(']')?;
    // Split at the first `:` (path-suffix form) or use the whole string.
    let number_str = inner.split_once(':').map(|(n, _)| n).unwrap_or(inner);
    number_str.trim().parse().ok()
}

/// Regex matching a canonical image chip placeholder anywhere in a
/// buffer string: `[Image #<digits>]` or `[Image #<digits>:`.
///
/// Used by `set_text` to detect whether a non-empty replacement
/// preserves any chips. A literal-substring scan
/// (`text.contains("[Image #")`) has two foot-guns:
/// - false positive on the user pasting prose like `"[Image #" as a
///   placeholder convention`,
/// - false negative on near-matches like `[Image#1]` (missing space)
///   that the parser would reject anyway.
///
/// The regex pins to the canonical emitter format so the heuristic
/// matches what `display_text(...)` actually produces.
fn chip_placeholder_regex() -> &'static regex::Regex {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\[Image #\d+(?::|\])").unwrap());
    &RE
}

/// Build a `[<label>]` chip line with the shared chip styling
/// (dim brackets, paste foreground/background).
fn chip_line(label: String) -> Line<'static> {
    let theme = Theme::current();
    let bracket = Style::default().fg(theme.paste_dim).bg(theme.paste_bg);
    let text = Style::default().fg(theme.paste_fg).bg(theme.paste_bg);
    Line::from(vec![
        Span::styled("[", bracket),
        Span::styled(label, text),
        Span::styled("]", bracket),
    ])
}

/// Normalize bare `\r` to `\n`, leaving `\r\n` pairs intact.
///
/// Some terminals send bare `\r` for line breaks in bracketed-paste content.
/// Rust's `str::lines()` only splits on `\n` and `\r\n`, so without this
/// normalization multi-line pastes would be treated as a single line.
pub fn normalize_cr(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() != Some(&'\n') {
            s.push('\n');
        } else {
            s.push(c);
        }
    }
    s
}

/// Build the styled display `Line` for a paste element chip.
///
/// Renders as: `[Pasted: N lines]`
fn paste_chip_display(line_count: usize) -> Line<'static> {
    chip_line(format!(
        "Pasted: {} line{}",
        line_count,
        if line_count != 1 { "s" } else { "" }
    ))
}

/// Display `Line` for a byte-triggered paste chip, e.g. `[Pasted: 12 KB]` or
/// `[Pasted: 1.0 MB]` (size, not a line count). Decimal (1000-based) units.
pub fn paste_chip_display_bytes(byte_len: usize) -> Line<'static> {
    let size = if byte_len >= 1_000_000 {
        format!("{:.1} MB", byte_len as f64 / 1_000_000.0)
    } else if byte_len >= 1000 {
        format!("{} KB", byte_len / 1000)
    } else {
        format!("{byte_len} bytes")
    };
    chip_line(format!("Pasted: {size}"))
}

/// Highest `display_number` present in `images`, or `0` when the slice is empty.
fn images_high_water(images: &[PastedImage]) -> usize {
    images.iter().map(|i| i.display_number).max().unwrap_or(0)
}

#[cfg(test)]
mod tests;
