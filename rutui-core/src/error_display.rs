//! User-facing formatting for terminal request / API errors.
//!
//! W4 decoupling: `WireErrorType` (display enum) and `banner_message` were
//! sunk here from `xai-grok-pager::app::error_display` — both are pure
//! formatting with no pager/ACP logic. The `disk_full` wire tag the shell
//! special-cases is inlined as a const (with the shell's value
//! `xai_grok_shell::extensions::notification::DISK_FULL_ERROR_TYPE`), so
//! this crate stays shell-free.

/// Wire `RetryState::Failed.error_type` values the pager understands: the
/// shell's `SamplingErrorKind::as_str` tags plus its special-cased tags
/// (`context_length`, `legacy_auth`, …). Unknown strings map to
/// [`WireErrorType::Other`] rather than being matched as raw `&str` at
/// call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireErrorType {
    Auth,
    AuthTransient,
    LegacyAuth,
    ContextLength,
    EncryptedContentMismatch,
    DiskFull,
    Api,
    Http,
    IdleTimeout,
    EmptyResponse,
    Serialization,
    RateLimited,
    MaxTokensTruncation,
    Other,
}

/// The shell's special-cased disk-full wire tag (value of
/// `xai_grok_shell::extensions::notification::DISK_FULL_ERROR_TYPE`),
/// inlined so this crate stays shell-free.
pub const DISK_FULL_ERROR_TYPE: &str = "disk_full";

/// Human-facing sentence for a full-disk condition (value of
/// `xai_grok_shell::extensions::notification::DISK_FULL_USER_MESSAGE`),
/// inlined for the same reason.
pub const DISK_FULL_USER_MESSAGE: &str = "Out of disk space. Free some space and try again.";

impl WireErrorType {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("auth") => Self::Auth,
            Some("auth_transient") => Self::AuthTransient,
            Some("legacy_auth") => Self::LegacyAuth,
            Some("context_length") => Self::ContextLength,
            Some("encrypted_content_mismatch") => Self::EncryptedContentMismatch,
            Some(s) if s == DISK_FULL_ERROR_TYPE => Self::DiskFull,
            Some("api") => Self::Api,
            Some("http") => Self::Http,
            Some("idle_timeout") => Self::IdleTimeout,
            Some("empty_response") => Self::EmptyResponse,
            Some("serialization") => Self::Serialization,
            Some("rate_limited") => Self::RateLimited,
            Some("max_tokens_truncation") => Self::MaxTokensTruncation,
            _ => Self::Other,
        }
    }
}

/// `Headline — detail` (headline alone when there is no detail). Shared with
/// the session-event block so the two renderings can't drift.
pub fn banner_message(headline: &str, detail: &str) -> String {
    if detail.is_empty() {
        headline.to_string()
    } else {
        format!("{headline} \u{2014} {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_and_unknown() {
        assert_eq!(WireErrorType::parse(Some("auth")), WireErrorType::Auth);
        assert_eq!(
            WireErrorType::parse(Some("auth_transient")),
            WireErrorType::AuthTransient
        );
        assert_eq!(
            WireErrorType::parse(Some("encrypted_content_mismatch")),
            WireErrorType::EncryptedContentMismatch
        );
        assert_eq!(
            WireErrorType::parse(Some("disk_full")),
            WireErrorType::DiskFull
        );
        assert_eq!(
            WireErrorType::parse(Some("rate_limited")),
            WireErrorType::RateLimited
        );
        assert_eq!(WireErrorType::parse(Some("nope")), WireErrorType::Other);
        assert_eq!(WireErrorType::parse(None), WireErrorType::Other);
    }

    #[test]
    fn banner_joins_with_dash_or_keeps_headline() {
        assert_eq!(
            banner_message("Server error (500)", ""),
            "Server error (500)"
        );
        assert_eq!(
            banner_message("Server error (500)", "boom"),
            "Server error (500) \u{2014} boom"
        );
    }
}
