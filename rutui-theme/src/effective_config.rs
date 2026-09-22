//! Injected access to the host application's effective configuration.
//!
//! The framework reads a handful of `[ui]`-style keys (`theme`,
//! `auto_dark_theme`, `auto_light_theme`, `compact_mode`, …) through this
//! seam. The host application installs the real layered-config loader at
//! startup (the pager installs `xai_grok_config`'s loader); until then reads
//! return `None` and callers fall back to their defaults.

use std::sync::OnceLock;

/// Loader returning the host's effective configuration as a parsed TOML
/// value, or `None` when no configuration is available / an error occurs.
pub type EffectiveConfigLoader = fn() -> Option<toml::Value>;

static LOADER: OnceLock<EffectiveConfigLoader> = OnceLock::new();

/// Install the effective-config loader. Idempotent — the first call wins.
pub fn install(loader: EffectiveConfigLoader) {
    let _ = LOADER.set(loader);
}

/// The host's effective configuration, if the loader is installed and
/// succeeded.
pub fn load() -> Option<toml::Value> {
    LOADER.get().and_then(|loader| loader())
}
