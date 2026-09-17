//! Minimal diff-hunk data types + unified-patch text assembly.
//!
//! Sunk into the framework (W4) from `xai-grok-pager-diff`: scrollback's
//! edit blocks only need the neutral `DiffLine`/`DiffHunk` shapes (tags from
//! `similar`) and `diff_hunks_to_patch`. `xai-grok-pager-diff` keeps the
//! business-side construction (`build_diff_hunks` from
//! `SearchReplaceEditDetail`, `extract_edit_hunks` from ACP tool calls,
//! `stitch_overlapping_hunks`) and re-exports these types so both sides
//! share one definition.

use similar::ChangeTag;

/// One tagged diff line.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub text: String,
    pub lo: usize,
    pub ln: usize,
    pub tag: ChangeTag,
}

/// A hunk: a contiguous run of tagged diff lines.
pub type DiffHunk = Vec<DiffLine>;

/// Assemble hunks into unified-diff text (`--- a/path` / `+++ b/path`
/// with `@@ -old,count +new,count @@` headers). Mirrors the origin
/// `xai-grok-pager-diff::diff_hunks_to_patch` exactly.
pub fn diff_hunks_to_patch(path: &str, hunks: &[DiffHunk]) -> String {
    if hunks.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));

    for hunk in hunks {
        if hunk.is_empty() {
            continue;
        }

        // Compute hunk header: @@ -old_start,old_count +new_start,new_count @@
        let old_start = hunk
            .iter()
            .filter(|l| l.tag != ChangeTag::Insert)
            .map(|l| l.lo)
            .next()
            .unwrap_or(1);
        let new_start = hunk
            .iter()
            .filter(|l| l.tag != ChangeTag::Delete)
            .map(|l| l.ln)
            .next()
            .unwrap_or(1);
        let old_count = hunk.iter().filter(|l| l.tag != ChangeTag::Insert).count();
        let new_count = hunk.iter().filter(|l| l.tag != ChangeTag::Delete).count();

        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));

        for line in hunk {
            let prefix = match line.tag {
                ChangeTag::Equal => ' ',
                ChangeTag::Insert => '+',
                ChangeTag::Delete => '-',
            };
            let text = line.text.trim_end_matches(['\r', '\n']);
            out.push(prefix);
            out.push_str(text);
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar::ChangeTag;

    fn diff_line(text: &str, lo: usize, ln: usize, tag: ChangeTag) -> DiffLine {
        DiffLine {
            text: text.to_string(),
            lo,
            ln,
            tag,
        }
    }

    #[test]
    fn patch_from_hunks() {
        let hunks = vec![vec![
            diff_line("let x = 1;\n", 1, 1, ChangeTag::Delete),
            diff_line("let x = 2;\n", 2, 1, ChangeTag::Insert),
        ]];
        let patch = diff_hunks_to_patch("a.rs", &hunks);
        assert!(patch.starts_with("--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n"));
        // Header `---` (3), hunk header `@@ -1,1` (1), content prefix (1) = 5.
        assert_eq!(patch.matches('-').count(), 5);
        // Same shape for `+`: `+++`, `@@ ... +1,1`, `+let x = 2;`.
        assert_eq!(patch.matches('+').count(), 5);
    }

    #[test]
    fn empty_hunks_empty_patch() {
        assert_eq!(diff_hunks_to_patch("a.rs", &[]), "");
    }
}
