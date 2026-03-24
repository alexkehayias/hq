use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ai::skills::{
    skill::{Skill, SkillSummary},
    validation::validate_skill_directory,
};

/// Registry for managing and discovering Agent Skills.
///
/// The registry loads skill summaries at startup (for efficient discovery)
/// and can load full skill content on demand.
#[derive(Debug, Clone)]
pub struct SkillRegistry {
    /// The base directory containing skill subdirectories.
    dir_path: PathBuf,
    /// Lightweight summaries for all skills (loaded at startup)
    summaries: HashMap<String, SkillSummary>,
}

impl SkillRegistry {
    /// Create a new registry with the given skills directory path.
    pub fn new<P: AsRef<Path>>(dir_path: P) -> Result<Self> {
        let dir_path = PathBuf::from(dir_path.as_ref());
        let mut registry = Self {
            dir_path,
            summaries: HashMap::new(),
        };

        // Load all skill summaries
        registry.load_summaries()?;

        Ok(registry)
    }

    /// Load summaries of all skills from the configured directory.
    ///
    /// This only loads metadata (name + description) for efficient discovery.
    /// Full skill content is loaded on demand when a skill is activated.
    fn load_summaries(&mut self) -> Result<()> {
        let dir_path = &self.dir_path;

        if !dir_path.is_dir() {
            return Err(anyhow!(
                "Skills path '{}' is not a directory",
                dir_path.display()
            ));
        }

        // Iterate through subdirectories
        for entry in fs::read_dir(dir_path)? {
            let entry = entry?;
            let path = entry.path();

            // Skip non-directories
            if !path.is_dir() {
                continue;
            }

            // Validate and load the skill
            match validate_skill_directory(&path) {
                Ok(()) => {
                    // Load just the summary (name + description)
                    if let Ok(skill) = Skill::load_from_directory(&path) {
                        let summary = skill.summary();
                        self.summaries.insert(summary.name.clone(), summary);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Skipping invalid skill directory '{}': {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        tracing::info!(
            "Loaded {} skill summaries from '{}'",
            self.summaries.len(),
            dir_path.display()
        );

        Ok(())
    }

    /// Reload all skill summaries from the configured directory.
    ///
    /// This is useful for picking up new skills without restarting the application.
    pub fn reload(&mut self) -> Result<()> {
        self.summaries.clear();
        self.load_summaries()
    }

    /// Get a summary for all available skills.
    ///
    /// Returns a vector of skill summaries sorted by name.
    pub fn list_skills(&self) -> Vec<SkillSummary> {
        let mut summaries: Vec<_> = self.summaries.values().cloned().collect();
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        summaries
    }

    /// Check if a skill with the given name exists.
    pub fn has_skill(&self, name: &str) -> bool {
        self.summaries.contains_key(name)
    }

    /// Get the summary for a specific skill by name.
    pub fn get_summary(&self, name: &str) -> Option<&SkillSummary> {
        self.summaries.get(name)
    }

    /// Load the full content of a skill by name.
    ///
    /// This loads the complete SKILL.md including frontmatter and body content.
    pub fn load_skill(&self, name: &str) -> Result<Skill> {
        let skill_path = self.dir_path.join(name);

        if !skill_path.exists() || !skill_path.is_dir() {
            return Err(anyhow!("Skill '{}' not found", name));
        }

        Skill::load_from_directory(&skill_path)
    }

    /// Search for skills by keyword in their name or description.
    ///
    /// Returns matching skill summaries sorted by relevance (exact name match first,
    /// then name contains, then description contains).
    pub fn search(&self, query: &str) -> Vec<SkillSummary> {
        let query_lower = query.to_lowercase();

        let mut exact_name_matches: Vec<SkillSummary> = Vec::new();
        let mut name_contains: Vec<SkillSummary> = Vec::new();
        let mut description_contains: Vec<SkillSummary> = Vec::new();

        for summary in self.summaries.values() {
            let name_lower = summary.name.to_lowercase();
            let description_lower = summary.description.to_lowercase();

            if name_lower == query_lower {
                exact_name_matches.push(summary.clone());
            } else if name_lower.contains(&query_lower) {
                name_contains.push(summary.clone());
            } else if description_lower.contains(&query_lower) {
                description_contains.push(summary.clone());
            }
        }

        // Sort each group alphabetically by name
        exact_name_matches.sort_by(|a, b| a.name.cmp(&b.name));
        name_contains.sort_by(|a, b| a.name.cmp(&b.name));
        description_contains.sort_by(|a, b| a.name.cmp(&b.name));

        // Concatenate with priority: exact name > name contains > description contains
        let mut results = Vec::new();
        results.extend(exact_name_matches);
        results.extend(name_contains);
        results.extend(description_contains);

        results
    }

    /// Get the number of loaded skills.
    pub fn count(&self) -> usize {
        self.summaries.len()
    }

    /// Get the skills directory path.
    pub fn dir_path(&self) -> &Path {
        &self.dir_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a test skill directory
    fn create_test_skill(base_dir: &Path, name: &str, description: &str) -> PathBuf {
        let skill_dir = base_dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();

        let skill_content = format!(
            r#"---
name: {}
description: {}
---

This is the body of {}.
"#,
            name, description, name
        );

        fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();

        skill_dir
    }

    #[test]
    fn test_registry_loads_skills() {
        let temp = TempDir::new().unwrap();
        create_test_skill(temp.path(), "pdf-processing", "Process PDF files");
        create_test_skill(temp.path(), "code-review", "Review code");
        let registry = SkillRegistry::new(temp.path()).unwrap();

        assert_eq!(registry.count(), 2);
        assert!(registry.has_skill("pdf-processing"));
        assert!(registry.has_skill("code-review"));
    }

    #[test]
    fn test_registry_search() {
        let temp = TempDir::new().unwrap();
        create_test_skill(temp.path(), "pdf-processing", "Process PDF files");
        create_test_skill(temp.path(), "code-review", "Review code for bugs");

        let registry = SkillRegistry::new(temp.path()).unwrap();

        // Exact name match
        let results = registry.search("pdf-processing");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "pdf-processing");

        // Partial name match
        let results = registry.search("pdf");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "pdf-processing");

        // Description match
        let results = registry.search("bugs");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "code-review");

        // No match
        let results = registry.search("nonexistent");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_registry_load_full_skill() {
        let temp = TempDir::new().unwrap();
        create_test_skill(temp.path(), "test-skill", "A test skill");
        let registry = SkillRegistry::new(temp.path()).unwrap();

        let skill = registry.load_skill("test-skill").unwrap();
        assert_eq!(skill.frontmatter.name, "test-skill");
        assert_eq!(skill.frontmatter.description, "A test skill");
        assert!(skill.body.contains("This is the body of test-skill"));
    }

    #[test]
    fn test_registry_reload() {
        let temp = TempDir::new().unwrap();
        create_test_skill(temp.path(), "skill-one", "First skill");
        let mut registry = SkillRegistry::new(temp.path()).unwrap();

        assert_eq!(registry.count(), 1);

        // Add a new skill
        create_test_skill(temp.path(), "skill-two", "Second skill");

        // Reload should pick up the new skill
        registry.reload().unwrap();
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_registry_empty_directory() {
        let temp = TempDir::new().unwrap();
        let registry = SkillRegistry::new(temp.path()).unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_registry_nonexistent_directory() {
        let temp = TempDir::new().unwrap();
        let non_existent = temp.path().join("nonexistent");
        let result = SkillRegistry::new(&non_existent);

        assert!(result.is_err());
    }
}
