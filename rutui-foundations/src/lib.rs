//! TUI foundations — 框架最底层的、业务无关的构建块。
//!
//! 这是一个「伞 crate」（umbrella crate）：把三个已经干净的 L0 原子 crate
//! 汇总成统一入口，供上层框架 crate（theme / widgets / core）以及最终
//! 应用使用。
//!
//! ## 包含什么
//!
//! | 子模块     | 来源 crate           | 作用 |
//! |-----------|----------------------|------|
//! | [`textarea`] | `rutui-ratatui-textarea` | 多行文本编辑器（TextArea / TextAreaState / 元素 chips） |
//! | [`inline`]   | `rutui-ratatui-inline`   | inline / scrollback 视口引擎（`Terminal::insert_before`、viewport 尺寸） |
//! | [`tty`]      | `rutui-tty-utils`        | TTY 安全的子进程派生（脱离控制终端、进程组生命周期） |
//!
//! 三者只依赖 ratatui / crossterm / tokio / unicode 等中性 crate，
//! **不含任何 `xai-grok-*` 业务依赖**，可独立拷到任意终端项目。
//!
//! ## 可移植性约定
//!
//! 源码内部一律用本地名（`textarea` / `inline` / `tty_utils`），`xai-` 前缀
//! 只出现在 `Cargo.toml`。拷走时重命名 manifest 即可，源码零改动。
//!
//! ## 使用
//!
//! ```rust,ignore
//! use rutui_foundations::textarea::TextArea;
//! use rutui_foundations::tty;
//! ```

// 直接透传三个原子 crate 的完整公共 API。外部 name 已在 Cargo.toml 改成
// 本地名，因此这里用本地名 re-export。
pub use inline;
pub use textarea;

/// TTY 安全工具（`tty_utils` crate）。重导出为 `tty`，语义更顺口。
pub mod tty {
    pub use tty_utils::*;
}

// 常用的编辑器类型在顶层再露一手，省去深挖子模块。
pub use textarea::{TextArea, TextAreaState};
pub use tty_utils::{detach_command, pager_env};
