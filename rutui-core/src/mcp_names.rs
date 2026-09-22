//! MCP tool-name display formatting.
//!
//! W4 decoupling: `MCP_TOOL_NAME_DELIMITER` + `mcp_titleize_segment`
//! inlined from `xai_grok_workspace::permission` (pure string formatting;
//! the canonical delimiter const lives in `xai-grok-workspace-types`, value
//! `"__"`).

/// Delimiter used to qualify MCP tool names as `"<server>__<tool>"`.
pub const MCP_TOOL_NAME_DELIMITER: &str = "__";

/// Pretty-format a single MCP server- or tool-name segment for display:
/// split on `'_'`, title-case each word, join with spaces. Leaves
/// non-underscore characters (camelCase, hyphens) intact, so
/// `"list_issues"` → `"List Issues"`, `"grok_com_notion"` →
/// `"Grok Com Notion"`, and `"getMyTaskList"` → `"GetMyTaskList"`.
pub fn mcp_titleize_segment(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delimiter_is_double_underscore() {
        assert_eq!(MCP_TOOL_NAME_DELIMITER, "__");
    }

    #[test]
    fn titleize_snake_case() {
        assert_eq!(mcp_titleize_segment("list_issues"), "List Issues");
        assert_eq!(mcp_titleize_segment("grok_com_notion"), "Grok Com Notion");
        assert_eq!(mcp_titleize_segment("getMyTaskList"), "GetMyTaskList");
    }
}
