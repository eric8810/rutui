//! Skill-read detection helper.
//!
//! W4 decoupling: `skill_name_from_path` inlined from
//! `xai_grok_tools::implementations::skills::types` (pure path parsing).

/// Extract the skill name from a path if it points to a `SKILL.md` file.
///
/// Returns the parent directory name (e.g. `"/skills/deploy/SKILL.md"` →
/// `"deploy"`). Returns `None` for non-SKILL.md paths or bare `"SKILL.md"`
/// with no parent.
pub fn skill_name_from_path(path: &str) -> Option<&str> {
    let p = std::path::Path::new(path);
    if p.file_name()?.to_str()? == "SKILL.md" {
        p.parent()?.file_name()?.to_str()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_dir_from_skill_md_path() {
        assert_eq!(
            skill_name_from_path("/skills/deploy/SKILL.md"),
            Some("deploy")
        );
        assert_eq!(skill_name_from_path("deploy/SKILL.md"), Some("deploy"));
        assert_eq!(skill_name_from_path("SKILL.md"), None);
        assert_eq!(skill_name_from_path("/skills/deploy/README.md"), None);
    }
}
