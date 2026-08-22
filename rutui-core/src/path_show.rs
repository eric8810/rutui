//! Display-path abbreviation helpers.
//!
//! W4 decoupling: `abbreviate_path` was a pager `util` helper that walked
//! `xai_grok_config::grok_home()`. This neutral copy uses the same
//! `$GROK_HOME`-or-`<home>/.grok` resolution (see
//! `scrollback::blocks::tool::memory_search::grok_home_default`) and `$HOME`,
//! without the config crate.

use std::path::Path;

/// Resolve the grok home for display: `$GROK_HOME` verbatim when set, else
/// `<os home>/.grok`. Mirrors `xai_grok_config::grok_home()`'s env semantics
/// (skips the config crate's dunce canonicalization / create_dir_all).
fn grok_home_default() -> std::path::PathBuf {
    if let Some(env) = std::env::var_os("GROK_HOME").filter(|v| !v.is_empty()) {
        return std::path::PathBuf::from(env);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".grok")
}

/// Abbreviate an absolute path for display: prefer the grok home
/// (`~/.grok/...`), then `~/...`, then the path unchanged.
pub fn abbreviate_path(path: &str) -> std::borrow::Cow<'_, str> {
    let path_buf = Path::new(path);
    let grok = grok_home_default();
    if let Ok(rest) = path_buf.strip_prefix(&grok) {
        if rest.as_os_str().is_empty() {
            return std::borrow::Cow::Owned("~/.grok".to_string());
        }
        return std::borrow::Cow::Owned(format!("~/.grok/{}", rest.display()));
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
        && let Some(rest) = path.strip_prefix(&home)
    {
        if rest.is_empty() {
            return std::borrow::Cow::Borrowed("~");
        }
        if rest.starts_with('/') {
            return std::borrow::Cow::Owned(format!("~{rest}"));
        }
    }
    std::borrow::Cow::Borrowed(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_relative_abbreviation() {
        match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => {
                let p = format!("{home}/project/file.rs");
                assert_eq!(abbreviate_path(&p), format!("~/project/file.rs"));
                assert_eq!(abbreviate_path(&home), "~");
            }
            _ => {
                assert_eq!(abbreviate_path("/else/where"), "/else/where");
            }
        }
    }
}