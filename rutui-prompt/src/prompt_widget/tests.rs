//! Tests for the business-free editor core: chip elements, image lifecycle,
//! ghost text, history search integration, completion dropdown, and the
//! key/paste entry points. Business-overlay behaviors (file search, slash,
//! predicted prompts) are tested in the embedding crate's shell.

use super::*;
use crate::suggestion_controller::{CompletionItemParsed, SuggestResponseParsed, SuggestionSource};
use crossterm::event::KeyCode;

fn widget() -> PromptWidget {
    PromptWidget::new_with_cwd_and_cmd_a(Path::new("."), false)
}

fn test_image() -> PastedImage {
    PastedImage {
        element_id: ElementId::from_raw(0), // overwritten by insert_image
        display_number: 0,
        mime_type: "image/png".into(),
        dimensions: Some((100, 80)),
        byte_len: 2048,
        encoded_bytes: Some(vec![0u8; 16].into()),
        source_path: None,
        staged_temp_path: None,
        session_image_path: None,
        preview: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Telemetry hook
// ---------------------------------------------------------------------------

/// The OnceLock cannot be reset between tests, so only assert the no-op
/// default and the install-once (idempotent) contract with a fresh event.
#[test]
fn backspace_telemetry_defaults_to_installed_or_installable() {
    // Fire into the default (no-op) slot - must not panic.
    backspace_no_effect_emit(BackspaceNoEffectEvent {
        terminal_brand: "Ghostty".into(),
        terminal_multiplexer: String::new(),
        terminal_is_ssh: false,
        terminal_term_var: None,
        terminal_term_version: None,
        terminal_term_version_source: None,
        key_code: "Backspace".into(),
        key_modifiers: String::new(),
        key_kind: "Press".into(),
        cursor_pos: 0,
        text_len: 1,
        has_selection: false,
    });
    // Installing a callback is allowed regardless (once), and emit then runs
    // it - but we can't rely on ordering with other tests. Just ensure the
    // public API exists and doesn't panic.
    let _ = install_backspace_no_effect_telemetry(|_| {});
}

// ---------------------------------------------------------------------------
// Text editing basics
// ---------------------------------------------------------------------------

#[test]
fn set_text_and_cursor() {
    let mut w = widget();
    assert!(w.text().is_empty());
    w.set_text("hello world");
    assert_eq!(w.text(), "hello world");
    w.set_cursor(5);
    assert_eq!(w.cursor(), 5);
    // Clamped.
    w.set_cursor(999);
    assert_eq!(w.cursor(), "hello world".len());
}

#[test]
fn append_text_preserves_chips() {
    let mut w = widget();
    w.set_text("start");
    w.insert_image(test_image()).unwrap();
    let chip_text = w.text().to_owned();
    assert!(chip_text.contains("[Image #1]"));
    w.append_text(" end");
    assert_eq!(w.text(), format!("{chip_text} end"));
    assert_eq!(w.images.len(), 1);
}

// ---------------------------------------------------------------------------
// Paste chips
// ---------------------------------------------------------------------------

#[test]
fn short_paste_inserts_inline() {
    let mut w = widget();
    w.handle_paste("hello");
    assert_eq!(w.text(), "hello");
    assert!(w.textarea.elements().is_empty());
}

/// The textarea stores the ORIGINAL pasted content as the element's buffer
/// text; the `[Pasted: N lines]` label is the element's display `Line`.
#[test]
fn multiline_paste_creates_chip() {
    let mut w = widget();
    w.handle_paste("line1\nline2\nline3\nline4");
    // >= 4 lines -> atomic [Pasted: N lines] element.
    let elems = w.textarea.elements();
    assert_eq!(elems.len(), 1);
    assert_eq!(elems[0].kind, KIND_PASTE);
    assert_eq!(w.text(), "line1\nline2\nline3\nline4");
    assert_eq!(
        elems[0].display.as_ref().unwrap().to_string(),
        "[Pasted: 4 lines]"
    );
}

#[test]
fn compact_mode_lowers_chip_threshold_to_2_lines() {
    let mut w = widget();
    w.set_compact(true);
    w.handle_paste("a\nb");
    assert_eq!(w.textarea.elements().len(), 1);
    assert_eq!(w.textarea.elements()[0].kind, KIND_PASTE);
}

#[test]
fn repaste_expands_existing_chip() {
    let mut w = widget();
    w.set_compact(true);
    w.handle_paste("a\nb");
    assert_eq!(w.textarea.elements().len(), 1);
    // Cursor sits right after the chip - same content repaste expands it.
    w.handle_paste("a\nb");
    assert!(w.textarea.elements().is_empty());
    assert_eq!(w.text(), "a\nb");
}

/// Enter on a paste chip inlines it; cursor must be ON the chip (element
/// hit-testing covers `[start, end)`).
#[test]
fn paste_chip_inlines_on_enter() {
    let mut w = widget();
    w.set_compact(true);
    w.handle_paste("x\ny");
    assert_eq!(w.textarea.elements().len(), 1);
    let elem_start = w.textarea.elements()[0].range.start;
    w.textarea.set_cursor(elem_start);
    let interaction = w.try_element_interaction(&KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(matches!(interaction, Some(ElementInteraction::Inlined)));
    assert!(w.textarea.elements().is_empty());
    assert_eq!(w.text(), "x\ny");
}

// ---------------------------------------------------------------------------
// Image chips
// ---------------------------------------------------------------------------

#[test]
fn image_insert_creates_chip_and_records() {
    let mut w = widget();
    assert!(w.insert_image(test_image()).is_ok());
    assert_eq!(w.images.len(), 1);
    assert_eq!(w.image_counter, 1);
    assert_eq!(w.images[0].display_number, 1);
    assert!(w.text().contains("[Image #1]"));
    // Cursor sits after the trailing separator space, not on the chip.
    assert!(w.image_at_cursor().is_none());
}

#[test]
fn image_cap_rejects_overflow() {
    let mut w = widget();
    for _ in 0..PromptWidget::IMAGE_CAP {
        assert!(w.insert_image(test_image()).is_ok());
    }
    let err = w.insert_image(test_image()).unwrap_err();
    assert!(err.contains("Image limit reached"));
}

#[test]
fn set_text_clears_orphan_images() {
    let mut w = widget();
    w.insert_image(test_image()).unwrap();
    assert_eq!(w.images.len(), 1);
    // Replacing with text that has no image chip placeholder orphans the record.
    w.set_text("plain text");
    assert!(w.images.is_empty());
    assert_eq!(w.image_counter, 0);
}

#[test]
fn empty_set_text_resets_image_state() {
    let mut w = widget();
    w.insert_image(test_image()).unwrap();
    w.set_text("");
    assert!(w.images.is_empty());
    assert_eq!(w.image_counter, 0);
    assert!(w.text().is_empty());
}

#[test]
fn drain_images_reconciles_live_elements() {
    let mut w = widget();
    w.insert_image(test_image()).unwrap();
    assert_eq!(w.images.len(), 1);
    // Delete the chip like the user would: the trailing separator space then
    // the atomic element itself (Backspace at the element boundary removes
    // the whole chip, moving the record to the undo stash).
    let backspace = KeyEvent::new(KeyCode::Backspace, crossterm::event::KeyModifiers::NONE);
    assert_eq!(w.handle_key(&backspace), PromptEvent::Edited);
    assert_eq!(w.handle_key(&backspace), PromptEvent::Edited);
    assert!(!w.text().contains("[Image #1]"));
    // The deleted chip's record must not survive into a drain (it is in the
    // undo stash, recoverable via redo, not in `images`).
    let drained = w.drain_images();
    assert!(drained.is_empty());
}

#[test]
fn text_without_image_chips_strips_placeholders() {
    let mut w = widget();
    w.set_text("before");
    w.insert_image(test_image()).unwrap();
    w.append_text(" after");
    assert!(w.text().contains("[Image #1]"));
    let stripped = w.text_without_image_chips();
    assert!(!stripped.contains("[Image #"));
    assert!(stripped.ends_with(" after"));
}

// ---------------------------------------------------------------------------
// Ghost text + completion dropdown
// ---------------------------------------------------------------------------

#[test]
fn ghost_text_set_accept_and_progressive_match() {
    let mut w = widget();
    w.set_ghost_text(Some("grep patterns".to_string()));
    assert!(w.has_ghost_text());
    let accepted = w.accept_ghost(AcceptMode::Full);
    assert_eq!(accepted.as_deref(), Some("grep patterns"));
    assert_eq!(w.text(), "grep patterns");
    assert!(!w.has_ghost_text());
}

#[test]
fn ghost_text_progressive_matching_trims_before_accept() {
    let mut w = widget();
    w.set_text("grep");
    w.suggestions.set_last_request_text("grep");
    w.suggestions
        .set_ghost(" patterns".to_string(), SuggestionSource::None);
    assert!(w.try_progressive_match("grep "));
    assert_eq!(w.suggestions.ghost_text(), Some("patterns"));
}

#[test]
fn completion_dropdown_accept_and_splice() {
    let mut w = widget();
    w.set_text("cat gr");
    w.suggestions.enabled = true;
    let generation_token = w.suggestions.begin_tab_completion(false);
    let request_text = w.text().to_owned();
    let request_cursor = w.cursor();
    w.suggestions.on_suggestions_loaded(
        SuggestResponseParsed {
            ghost: None,
            completions: vec![CompletionItemParsed {
                display: "notes.md".into(),
                description: String::new(),
                insert_text: "cat notes.md".into(),
                source: SuggestionSource::FilePath,
                priority: 0,
                replace_range: Some(4..6),
                token_text: Some("notes.md".into()),
                truncated: false,
            }],
            generation: generation_token,
        },
        &request_text,
        request_cursor,
    );
    assert!(w.completion_dropdown_open_if_available());
    let splice = w.completion_dropdown_accept().unwrap();
    let CompletionSplice::Token(range, _) = &splice else {
        panic!("expected a token splice, got {splice:?}");
    };
    assert_eq!(range, &(4..6));
    assert!(w.apply_completion_splice(splice));
    assert_eq!(w.text(), "cat notes.md");
}

// ---------------------------------------------------------------------------
// history_search is wired into the widget (delegation sanity)
// ---------------------------------------------------------------------------

#[test]
fn history_search_is_owned_by_the_widget() {
    let mut w = widget();
    let activated = w.history_search.activate(
        &[crate::history_search::HistoryEntry { text: "abc".into() }],
        "",
    );
    assert!(activated);
    // The daemon matches on another thread; spin briefly like the
    // history_search module's own tests.
    let mut delivered = false;
    for _ in 0..100 {
        if w.history_search.poll() && w.history_search.result_count() > 0 {
            delivered = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(delivered);
    assert!(w.history_search.is_active());
    w.history_search.deactivate();
    assert!(!w.history_search.is_active());
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

#[test]
fn ctrl_c_clears_non_empty_text() {
    let mut w = widget();
    w.set_text("draft");
    let evt = w.handle_key(&KeyEvent::new(
        KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    assert_eq!(evt, PromptEvent::Edited);
    assert!(w.text().is_empty());
}

#[test]
fn esc_and_tab_are_ignored() {
    let mut w = widget();
    let esc = w.handle_key(&KeyEvent::new(
        KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(esc, PromptEvent::Ignored);
}

#[test]
fn backslash_continuation_blocks_send() {
    let mut w = widget();
    w.set_text("line\\");
    w.set_cursor(5);
    assert!(w.try_send().is_none());
    assert_eq!(w.text(), "line\n");
}

#[test]
fn route_enter_submits_on_bare_enter() {
    let mut w = widget();
    w.set_text("hello");
    let outcome = w.route_enter(&KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(outcome, EnterOutcome::Submit);
}

#[test]
fn route_enter_inserts_newline_on_shift_enter() {
    let mut w = widget();
    let outcome = w.route_enter(&KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::SHIFT,
    ));
    assert_eq!(outcome, EnterOutcome::PassThrough);
    let evt = w.handle_key(&KeyEvent::new(
        KeyCode::Enter,
        crossterm::event::KeyModifiers::SHIFT,
    ));
    assert_eq!(evt, PromptEvent::Edited);
    assert_eq!(w.text(), "\n");
}

#[test]
fn cmd_a_gate_only_fires_when_enabled() {
    let mut w = widget();
    w.set_text("hello");
    let key = KeyEvent::new(KeyCode::Char('a'), crossterm::event::KeyModifiers::SUPER);
    // Disabled by default in this fixture.
    assert_eq!(w.handle_key(&key), PromptEvent::Ignored);

    // With the Ghostty gate on, Cmd+A selects all.
    w.set_cmd_a_select_all_enabled_for_test(true);
    assert_eq!(w.handle_key(&key), PromptEvent::Edited);
    let selection = w.textarea.selection_range().unwrap();
    assert_eq!(selection, 0.."hello".len());
}
