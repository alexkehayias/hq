use regex::Regex;
use std::path::Path;

/// Error types for skill validation
#[derive(Debug, thiserror::Error)]
pub enum SkillValidationError {
    #[error("Invalid skill name: {0}")]
    InvalidName(String),

    #[error("Skill directory not found: {0}")]
    DirectoryNotFound(String),

    #[error("SKILL.md file not found in directory: {0}")]
    SkillFileNotFound(String),

    #[error("Invalid frontmatter: {0}")]
    InvalidFrontmatter(String),
}

/// Validates a skill name according to the Agent Skills specification.
///
/// # Constraints
/// - Must be 1-64 characters
/// - May only contain lowercase alphanumeric characters and hyphens (`a-z`, `0-9`, `-`)
/// - Must not start or end with a hyphen
/// - Must not contain consecutive hyphens (`--`)
pub fn validate_skill_name(name: &str) -> Result<(), SkillValidationError> {
    // Check length (1-64 characters)
    if name.is_empty() || name.len() > 64 {
        return Err(SkillValidationError::InvalidName(
            "Name must be between 1 and 64 characters".to_string(),
        ));
    }

    // Check that it only contains lowercase alphanumeric and hyphens
    let valid_chars = Regex::new(r"^[a-z0-9-]+$").unwrap();
    if !valid_chars.is_match(name) {
        return Err(SkillValidationError::InvalidName(
            "Name may only contain lowercase letters, numbers, and hyphens".to_string(),
        ));
    }

    // Must not start or end with a hyphen
    if name.starts_with('-') || name.ends_with('-') {
        return Err(SkillValidationError::InvalidName(
            "Name must not start or end with a hyphen".to_string(),
        ));
    }

    // Must not contain consecutive hyphens
    if name.contains("--") {
        return Err(SkillValidationError::InvalidName(
            "Name must not contain consecutive hyphens".to_string(),
        ));
    }

    Ok(())
}

/// Validates that a skill directory contains the required SKILL.md file.
pub fn validate_skill_directory(path: &Path) -> Result<(), SkillValidationError> {
    if !path.exists() || !path.is_dir() {
        return Err(SkillValidationError::DirectoryNotFound(
            path.display().to_string(),
        ));
    }

    let skill_file = path.join("SKILL.md");
    if !skill_file.exists() || !skill_file.is_file() {
        return Err(SkillValidationError::SkillFileNotFound(
            path.display().to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_skill_names() {
        let valid_names = vec![
            "pdf-processing",
            "data-analysis",
            "code-review",
            "a",
            "skill-name-123",
        ];

        for name in valid_names {
            assert!(
                validate_skill_name(name).is_ok(),
                "{} should be valid",
                name
            );
        }
    }

    #[test]
    fn test_invalid_skill_names() {
        let invalid_cases = vec![
            ("PDF-Processing", "uppercase"),
            ("-pdf", "starts with hyphen"),
            ("pdf-", "ends with hyphen"),
            ("pdf--processing", "consecutive hyphens"),
            ("", "empty"),
        ];

        for (name, reason) in invalid_cases {
            assert!(
                validate_skill_name(name).is_err(),
                "{} should be invalid ({})",
                name,
                reason
            );
        }
    }

    #[test]
    fn test_name_too_long() {
        let long_name = "a".repeat(65);
        assert!(validate_skill_name(&long_name).is_err());
    }
}
