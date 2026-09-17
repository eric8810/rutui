//! Pure context-window usage data for `/context`-style snapshots.
//!
//! Sunk into the framework (W4) from `xai-grok-shell::session` —
//! `ContextInfo`, `TokenUsageCategory` and `count_detail` are pure
//! data/formatting with zero session logic. `xai-grok-shell` re-exports
//! these so every existing `xai_grok_shell::session::ContextInfo` path
//! keeps resolving to the same type.
//!
//! The token math (bytes/4 estimate, usage percent, free tokens) is inlined
//! verbatim from `xai-token-estimation` (pure heuristics, no business deps;
//! mirrors that crate exactly so displays cannot drift).

/// Bytes per token under the rough character-based heuristic (copy of
/// `xai_token_estimation::BYTES_PER_TOKEN`).
const BYTES_PER_TOKEN: u64 = 4;

/// Serde default for the auto-compact threshold (historical default of 85,
/// mirrored from the shell's `util::config`).
pub const DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT: u8 = 85;

/// Bytes/4 estimate of a string's token count (copy of
/// `xai_token_estimation::estimate_tokens`).
#[inline]
pub fn estimate_tokens(s: &str) -> u64 {
    (s.len() as u64) / BYTES_PER_TOKEN
}

/// Usage percentage rounded to `u8`, clamped to `100` (copy of
/// `xai_token_estimation::usage_percentage_u8` — `0.0` when `total == 0`).
#[inline]
pub fn usage_percentage_u8(used: u64, total: u64) -> u8 {
    if total == 0 {
        0
    } else {
        (((used as f64) / (total as f64) * 100.0).min(100.0)).round() as u8
    }
}

/// `total - used`, saturating at zero (copy of
/// `xai_token_estimation::free_tokens`).
#[inline]
pub fn free_tokens(total: u64, used: u64) -> u64 {
    total.saturating_sub(used)
}

/// Itemized token usage for one context category, shown as an
/// informational row in `/context`, e.g. the skills listing or the
/// MCP server listing.
///
/// (Doc comment preserved from the origin `xai-grok-shell` copy.)
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenUsageCategory {
    /// Display label, e.g. `"Skills"` or `"MCP servers"`.
    pub label: String,
    /// Estimated tokens this category costs in context.
    pub tokens: u64,
    /// Short supporting detail. By convention a count followed by a
    /// noun, e.g. `"21 skills"`; the pager right-aligns the leading count
    /// across rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl TokenUsageCategory {
    /// Row for the skills listing. `text` is the canonical render from
    /// the host's skill manager snapshot.
    pub fn skills_listing(text: &str, skill_count: usize) -> Self {
        Self {
            label: "Skills".to_string(),
            tokens: estimate_tokens(text),
            detail: Some(count_detail(skill_count as u64, "skill")),
        }
    }

    /// Row for the MCP server announcement. `text` is the full reminder
    /// body for the current server set.
    pub fn mcp_servers(text: &str, server_count: usize) -> Self {
        Self {
            label: "MCP servers".to_string(),
            tokens: estimate_tokens(text),
            detail: Some(count_detail(server_count as u64, "server")),
        }
    }
}

/// Formats a count with a naively pluralized noun: `"1 skill"`, `"21 skills"`.
pub fn count_detail(count: u64, noun: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{suffix}")
}

/// Context usage breakdown for session info.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContextInfo {
    pub used: u64,
    pub total: u64,
    pub system_prompt_tokens: u64,
    pub tool_definitions_count: u64,
    pub tool_definitions_tokens: u64,
    pub compaction_count: u64,
    pub turn_count: u64,
    pub tool_call_count: u64,
    /// Total conversation items (system + user + assistant + tool responses).
    pub message_count: u64,
    /// Bytes/4 estimate of all non-system conversation items.
    pub message_tokens: u64,
    pub free_tokens: u64,
    pub usage_pct: u8,
    /// The resolved auto-compact threshold percent (0-100) for the active model
    /// at the time this snapshot was captured. Comes from the 6-tier resolution
    /// (env > user per-model > user global > GB per-model > GB global > 85).
    /// Used by the TUI `/context` view so the displayed "Auto-compact at X%"
    /// always matches the actual trigger (e.g. 65 for grok-build in remote
    /// settings).
    #[serde(default = "default_auto_compact_threshold")]
    pub auto_compact_threshold_percent: u8,
    /// Itemized usage rows (skills listing, MCP server listing). Empty on
    /// partial snapshots.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub usage_categories: Vec<TokenUsageCategory>,
}

/// Historical default for `auto_compact_threshold_percent`: the bottom tier
/// of the 6-tier resolution (env > user per-model > user global > GB
/// per-model > GB global > 85). Kept in sync with the field-level serde
/// default so `ContextInfo::default()` and old JSON snapshots agree.
impl Default for ContextInfo {
    fn default() -> Self {
        Self {
            used: 0,
            total: 0,
            system_prompt_tokens: 0,
            tool_definitions_count: 0,
            tool_definitions_tokens: 0,
            compaction_count: 0,
            turn_count: 0,
            tool_call_count: 0,
            message_count: 0,
            message_tokens: 0,
            free_tokens: 0,
            usage_pct: 0,
            auto_compact_threshold_percent: DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT,
            usage_categories: Vec::new(),
        }
    }
}

impl ContextInfo {
    /// Partial snapshot from a notification carrying only used + total.
    /// Breakdown fields default to zero until the next full ContextInfo update.
    pub fn from_notification(used: u64, total: u64) -> Self {
        Self {
            used,
            total,
            usage_pct: usage_percentage_u8(used, total),
            free_tokens: free_tokens(total, used),
            auto_compact_threshold_percent: DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT,
            ..Self::default()
        }
    }
}

/// Serde default for the new threshold field (keeps old snapshots / partials
/// deserializing without error and gives the historical default of 85).
fn default_auto_compact_threshold() -> u8 {
    DEFAULT_AUTO_COMPACT_THRESHOLD_PERCENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_detail_pluralizes() {
        assert_eq!(count_detail(1, "skill"), "1 skill");
        assert_eq!(count_detail(21, "skill"), "21 skills");
    }

    #[test]
    fn from_notification_sets_derived_bits() {
        let c = ContextInfo::from_notification(50_000, 200_000);
        assert_eq!(c.used, 50_000);
        assert_eq!(c.total, 200_000);
        assert_eq!(c.usage_pct, 25);
        assert_eq!(c.free_tokens, 150_000);
        assert_eq!(c.auto_compact_threshold_percent, 85);
    }

    #[test]
    fn from_notification_zero_total_no_panic() {
        let c = ContextInfo::from_notification(100, 0);
        assert_eq!(c.usage_pct, 0);
        assert_eq!(c.free_tokens, 0);
    }

    #[test]
    fn serde_roundtrip_defaults() {
        let json = serde_json::to_string(&ContextInfo::default()).unwrap();
        let back: ContextInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.used, 0);
        assert_eq!(back.total, 0);
        assert_eq!(back.usage_pct, 0);
        assert_eq!(back.auto_compact_threshold_percent, 85);
        assert!(back.usage_categories.is_empty());
    }
}
