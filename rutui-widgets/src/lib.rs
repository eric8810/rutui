//! `rutui-widgets` — business-agnostic terminal UI widgets.
//!
//! Extracted (W2) from `xai-grok-pager/src/views/`. Reusable UI chrome for
//! any terminal app: modal window chrome, generic confirmation dialogs,
//! overlays, list overlays, progress bars, the shortcuts bar, and
//! theme-agnostic debug HUDs.
//!
//! Zero dependency on `xai-grok-*` business crates. In-repo dependencies are
//! `rutui-theme` (theme / render / appearance / glyphs) and `rutui-input`
//! (the single `KeyShortcut` definition the shortcuts bar binds by default).
//!
//! Crate layout:
//!
//! - [`key`] — the [`key::KeyHint`] display trait (generic over the bar's key
//!   type) plus the bridge impl for [`rutui_input::key::KeyShortcut`]. The
//!   concrete `KeyShortcut` type and `key!` macro live in `rutui-input`.
//! - [`shortcuts_bar`] — `HintItem` / `ShortcutsBar` / `PendingHint` /
//!   `CompactConfig`, generic over a [`key::KeyHint`]-implementing key type.
//! - [`modal_window`] — shared popup window chrome (border, title, tabs,
//!   footer shortcuts, input routing) with the `EMBEDDED` global switch.
//! - [`overlay`] / [`overlay_list`] — overlay pane state machine and the
//!   shared prompt-area list overlay.
//! - [`progress_bar`] — 1/8-cell Unicode block progress bar.
//! - [`debug_style`] / [`fps_hud`] — theme-agnostic debug overlay chrome
//!   and the release-safe FPS readout.
//!
//! Note (W2): `modal.rs`'s generic `ModalConfirmation<R>` / `ModalOption<R>`
//! stayed in the pager: its business constructors (`edit_confirm()`, …) are
//! inherent impls on `ModalConfirmation<BusinessResult>` invoked from the
//! pager's `app/` test code, and inherent impls on a foreign type are not
//! allowed (E0116) — extracting the types would require touching `app/`,
//! which is out of scope for this wave.

pub mod debug_style;
pub mod fps_hud;
pub mod key;
pub mod modal_window;
pub mod overlay;
pub mod overlay_list;
pub mod progress_bar;
pub mod shortcuts_bar;