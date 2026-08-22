//! Subagent metadata formatting.
//!
//! W4 decoupling: `format_subagent_meta` (with its `dedup_persona_role` /
//! `join_meta_parts` helpers) moved here from the pager's
//! `app::subagent` — a pure `(persona · role · model)` label formatter.

/// Collapse `(persona, role)` to a single label when both refer to the same
/// title. Comparison is case-insensitive after trimming surrounding
/// whitespace. (Copied verbatim from the pager's `app::subagent`.)
///
/// Behavior:
/// - Either side that is `Some(s)` where `s.trim().is_empty()` is treated as
///   `None` first, so a stray empty/whitespace-only string never sneaks into
///   the joined output as a leading separator.
/// - Both present and titles match -> returns `(Some(persona), None)` so
///   callers render only the persona once.
/// - Both present and titles differ -> returns the inputs unchanged.
/// - Either or both absent -> returns the inputs unchanged.
fn dedup_persona_role<'a, 'b>(
    persona: Option<&'a str>,
    role: Option<&'b str>,
) -> (Option<&'a str>, Option<&'b str>) {
    let persona = persona.filter(|s| !s.trim().is_empty());
    let role = role.filter(|s| !s.trim().is_empty());
    match (persona, role) {
        (Some(p), Some(r)) if p.trim().eq_ignore_ascii_case(r.trim()) => (Some(p), None),
        _ => (persona, role),
    }
}

/// Join non-empty parts with ` · `.
fn join_meta_parts(parts: &[Option<&str>]) -> String {
    let non_empty: Vec<&str> = parts.iter().copied().flatten().collect();
    if non_empty.is_empty() {
        String::new()
    } else {
        non_empty.join(" \u{00b7} ")
    }
}

/// ` (persona · role · model)` parenthetical for a subagent line; empty
/// string when nothing is set.
pub fn format_subagent_meta(
    persona: Option<&str>,
    role: Option<&str>,
    model: Option<&str>,
) -> String {
    let (persona, role) = dedup_persona_role(persona, role);
    let bare = join_meta_parts(&[persona, role, model]);
    if bare.is_empty() {
        bare
    } else {
        format!(" ({bare})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_empty_output() {
        assert_eq!(format_subagent_meta(None, None, None), "");
        assert_eq!(format_subagent_meta(Some(""), Some("  "), None), "");
    }

    #[test]
    fn all_three_joined() {
        assert_eq!(
            format_subagent_meta(Some("researcher"), Some("analyst"), Some("grok-3")),
            " (researcher \u{00b7} analyst \u{00b7} grok-3)"
        );
    }

    #[test]
    fn persona_and_role_dedupe() {
        assert_eq!(
            format_subagent_meta(Some("Researcher"), Some("researcher"), Some("grok-3")),
            " (Researcher \u{00b7} grok-3)"
        );
    }

    #[test]
    fn missing_optional_parts_omitted() {
        assert_eq!(format_subagent_meta(Some("researcher"), None, Some("grok-3")), " (researcher \u{00b7} grok-3)");
    }
}