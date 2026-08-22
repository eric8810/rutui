# rutui

A toolkit for building rich, interactive terminal UIs in Rust, built on [ratatui](https://ratatui.rs) and crossterm.

Extracted from a production-grade agentic coding CLI, `rutui` packages the reusable
building blocks of a polished terminal interface — theming, terminal-capability
detection, a block-style scrollable conversation view, a multi-line prompt editor,
generic UI widgets, and a keybinding registry — with **zero application-specific
business logic**.

## Crates

| Crate | What it gives you |
|---|---|
| [`rutui-theme`](rutui-theme) | Theming (5 built-in themes), terminal capability probing (kitty/sixel images, OSC 8 hyperlinks, tmux, keyboard protocols), color-capability quantization, appearance config |
| [`rutui-widgets`](rutui-widgets) | Generic UI parts: modal-window chrome, overlays, progress bars, the shortcuts bar |
| [`rutui-input`](rutui-input) | Key-shortcut abstraction, keyboard/modifier normalization, mouse scroll state machine, single-line editor, and a generic `ActionRegistry<A, C>` keybinding registry with 3-level bubbling |
| [`rutui-core`](rutui-core) | The block-style scrollback rendering pipeline: content blocks, scrolling, folding, text selection, search, sticky headers, timeline rail |
| [`rutui-prompt`](rutui-prompt) | The multi-line prompt editor: textarea wiring, paste/image chips with undo, ghost text, completion dropdown, fuzzy history search |
| [`rutui-markdown`](rutui-markdown) | Streaming terminal markdown renderer (pulldown-cmark + syntect highlighting) |
| [`rutui-foundations`](rutui-foundations) | Umbrella re-export of the lowest-level primitives (textarea, inline viewport, TTY-safe process spawning) |

Foundation primitives (also usable standalone):
- [`rutui-ratatui-textarea`](rutui-ratatui-textarea) — a multi-line text editor widget for ratatui
- [`rutui-ratatui-inline`](rutui-ratatui-inline) — an inline / scrollback-native viewport engine
- [`rutui-tty-utils`](rutui-tty-utils) — TTY-safe subprocess spawning

## Design

- **Business-agnostic**: every crate depends only on neutral crates (ratatui, crossterm,
  tokio, unicode-*, syntect, …). Application state, protocols, and product features are
  the caller's responsibility, injected via generics and `install_*` seams.
- **Elm-style data flow**: input → intent → synchronous state update → async effects.
  The keybinding registry (`rutui-input`) and the view components are decoupled from the
  state you drive them with.
- **Testable**: the presentation logic is terminal-independent and covered by snapshot
  and unit tests.

## Status

Early. The API surface is stabilizing as the extraction completes. Expect breaking
changes between minor versions.

## License

Apache-2.0
