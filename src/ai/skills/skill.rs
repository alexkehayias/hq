use crate::ai::skills::validation::{validate_skill_directory, validate_skill_name};
use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Regex to match YAML frontmatter (between --- delimiters)
/// Uses [\s\S]*? to match any character including newlines
static FRONTMATTER_RE: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new(r"^---\s*\n([\s\S]*?)\n---\s*\n([\s\S]*)$").unwrap());
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The frontmatter section of a SKILL.md file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    /// Required: The skill name (max 64 chars, lowercase alphanumeric + hyphens)
    pub name: String,

    /// Required: Description of what the skill does and when to use it (max 1024 chars)
    pub description: String,

    /// Optional: License name or reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// Optional: Environment requirements (max 500 chars)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,

    /// Optional: Arbitrary key-value metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,

    /// Optional: Space-delimited list of pre-approved tools (experimental)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
}

/// A complete skill with frontmatter and body content.
#[derive(Debug, Clone)]
pub struct Skill {
    /// The skill's directory path
    pub path: PathBuf,

    /// Parsed frontmatter metadata
    pub frontmatter: SkillFrontmatter,

    /// The markdown body content (instructions)
    pub body: String,
}

impl Skill {
    /// Load a skill from its directory.
    ///
    /// The directory must contain a `SKILL.md` file with YAML frontmatter
    /// followed by markdown content.
    pub fn load_from_directory(path: &Path) -> Result<Self> {
        // Validate the directory exists and contains SKILL.md
        validate_skill_directory(path)?;

        let skill_file = path.join("SKILL.md");
        let content = fs::read_to_string(&skill_file)?;

        Self::parse_from_content(path.to_path_buf(), &content)
    }

    /// Parse a skill from markdown content with YAML frontmatter.
    fn parse_from_content(path: PathBuf, content: &str) -> Result<Self> {
        // Parse YAML frontmatter (between --- delimiters)
        let captures = FRONTMATTER_RE
            .captures(content)
            .ok_or_else(|| anyhow!("Invalid SKILL.md: missing or malformed frontmatter"))?;

        let yaml_content = captures.get(1).unwrap().as_str();
        let body_content = captures.get(2).unwrap().as_str();

        // Parse YAML frontmatter
        let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml_content)
            .map_err(|e| anyhow!("Failed to parse frontmatter: {}", e))?;

        // Validate the skill name
        validate_skill_name(&frontmatter.name)?;

        // Validate description length (max 1024 characters)
        if frontmatter.description.is_empty() || frontmatter.description.len() > 1024 {
            return Err(anyhow!("Description must be between 1 and 1024 characters"));
        }

        // Validate compatibility length if provided (max 500 characters)
        if let Some(ref compat) = frontmatter.compatibility
            && !compat.is_empty()
            && compat.len() > 500
        {
            return Err(anyhow!("Compatibility must be at most 500 characters"));
        }

        // Validate that the directory name matches the skill name
        if let Some(dir_name) = path.file_name()
            && dir_name.to_string_lossy() != frontmatter.name
        {
            return Err(anyhow!(
                "Directory name '{}' does not match skill name '{}'",
                dir_name.to_string_lossy(),
                frontmatter.name
            ));
        }

        Ok(Self {
            path,
            frontmatter,
            body: body_content.trim().to_string(),
        })
    }

    /// Get the path to a file in the skill's optional directories.
    ///
    /// Supports: `scripts/`, `references/`, and `assets/`
    pub fn get_file_path(&self, relative_path: &str) -> Option<PathBuf> {
        let full_path = self.path.join(relative_path);
        if full_path.exists() && full_path.is_file() {
            Some(full_path)
        } else {
            None
        }
    }

    /// Read a file from the skill's optional directories.
    ///
    /// Returns `None` if the file doesn't exist or cannot be read.
    pub fn read_file(&self, relative_path: &str) -> Option<String> {
        self.get_file_path(relative_path)
            .and_then(|path| fs::read_to_string(path).ok())
    }

    /// Check if the skill has a specific optional directory.
    pub fn has_directory(&self, dir_name: &str) -> bool {
        let dir_path = self.path.join(dir_name);
        dir_path.exists() && dir_path.is_dir()
    }

    /// Get a summary for skill discovery (name + description only).
    pub fn summary(&self) -> SkillSummary {
        SkillSummary {
            name: self.frontmatter.name.clone(),
            description: self.frontmatter.description.clone(),
        }
    }

    /// Get the full content for skill activation (frontmatter + body).
    ///
    /// This is what should be available to the agent when a skill is activated.
    pub fn full_content(&self) -> String {
        format!(
            "# {}\n\n{}\n\n---\n\n{}",
            self.frontmatter.name, self.frontmatter.description, self.body
        )
    }
}

/// A lightweight summary for skill discovery (loaded at startup).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_skill() {
        let content = r#"---
name: pdf-processing
description: Extracts text and tables from PDF files, fills forms, merges documents. Use when working with PDFs.
license: Apache-2.0
metadata:
  author: example-org
  version: "1.0"
---

This skill helps you work with PDF files.

## Usage

Run the extraction script:
scripts/extract.py"#;

        let path = PathBuf::from("/skills/pdf-processing");
        let skill = Skill::parse_from_content(path, content).unwrap();

        assert_eq!(skill.frontmatter.name, "pdf-processing");
        assert_eq!(
            skill.frontmatter.description,
            "Extracts text and tables from PDF files, fills forms, merges documents. Use when working with PDFs."
        );
        assert_eq!(skill.frontmatter.license, Some("Apache-2.0".to_string()));
        assert_eq!(
            skill.frontmatter.metadata.as_ref().unwrap().get("author"),
            Some(&"example-org".to_string())
        );
        assert!(
            skill
                .body
                .contains("This skill helps you work with PDF files")
        );
    }

    #[test]
    fn test_parse_minimal_skill() {
        let content = r#"---
name: simple-skill
description: A simple skill for testing.
---

Just do it."#;

        let path = PathBuf::from("/skills/simple-skill");
        let skill = Skill::parse_from_content(path, content).unwrap();

        assert_eq!(skill.frontmatter.name, "simple-skill");
        assert!(skill.frontmatter.license.is_none());
        assert_eq!(skill.body, "Just do it.");
    }

    #[test]
    fn test_invalid_skill_name() {
        let content = r#"---
name: PDF-Processing
description: Invalid name with uppercase.
---

Content here."#;

        let path = PathBuf::from("/skills/PDF-Processing");
        assert!(Skill::parse_from_content(path, content).is_err());
    }

    #[test]
    fn test_empty_description() {
        let content = r#"---
name: test-skill
description: ""
---

Content here."#;

        let path = PathBuf::from("/skills/test-skill");
        assert!(Skill::parse_from_content(path, content).is_err());
    }

    #[test]
    fn test_description_too_long() {
        let content = format!(
            r#"---
name: test-skill
description: {}
---

Content here."#,
            "a".repeat(1025)
        );

        let path = PathBuf::from("/skills/test-skill");
        assert!(Skill::parse_from_content(path, &content).is_err());
    }

    #[test]
    fn test_mismatched_directory_name() {
        let content = r#"---
name: pdf-processing
description: A skill.
---

Content here."#;

        let path = PathBuf::from("/skills/different-name");
        assert!(Skill::parse_from_content(path, content).is_err());
    }
}
