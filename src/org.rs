use std::fmt;

/// Builder for constructing org-mode documents.
///
/// A `Document` has document-level properties and keywords
/// (`#+TITLE`, `#+FILETAGS`) followed by a sequence of headlines.
///
/// # Example
///
/// ```
/// let doc = hq::org::Document::builder()
///     .property("ID", "doc-uuid")
///     .title("Project Name")
///     .filetags("project")
///     .headline(
///         hq::org::Headline::builder()
///             .level(1)
///             .status("TODO")
///             .title("First task")
///             .property("ID", "task-uuid")
///             .body("Details here")
///             .build(),
///     )
///     .build();
/// ```
#[derive(Clone, Debug)]
pub struct Document {
    properties: Vec<(String, String)>,
    title: Option<String>,
    filetags: Option<String>,
    headlines: Vec<Headline>,
}

/// A single org-mode headline with optional TODO status, property drawer, and body.
///
/// # Example
///
/// ```
/// let h = hq::org::Headline::builder()
///     .level(1)
///     .status("DONE")
///     .title("Finished task")
///     .property("ID", "task-uuid")
///     .body("Completed work")
///     .build();
/// ```
#[derive(Clone, Debug)]
pub struct Headline {
    level: usize,
    status: Option<String>,
    title: String,
    properties: Vec<(String, String)>,
    body: Option<String>,
}

// ---------------------------------------------------------------------------
// Document builder
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DocumentBuilder {
    properties: Vec<(String, String)>,
    title: Option<String>,
    filetags: Option<String>,
    headlines: Vec<Headline>,
}

impl DocumentBuilder {
    /// Add a document-level property (e.g. `("ID", "some-uuid")`).
    pub fn property(mut self, key: &str, value: &str) -> Self {
        self.properties.push((key.to_string(), value.to_string()));
        self
    }

    /// Set the `#+TITLE` keyword value.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    /// Set the `#+FILETAGS` keyword value (space-separated tags).
    pub fn filetags(mut self, tags: &str) -> Self {
        self.filetags = Some(tags.to_string());
        self
    }

    /// Append a headline to the document.
    pub fn headline(mut self, headline: Headline) -> Self {
        self.headlines.push(headline);
        self
    }

    /// Consume the builder and produce a `Document`.
    pub fn build(self) -> Document {
        Document {
            properties: self.properties,
            title: self.title,
            filetags: self.filetags,
            headlines: self.headlines,
        }
    }
}

// ---------------------------------------------------------------------------
// Headline builder
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct HeadlineBuilder {
    level: Option<usize>,
    status: Option<String>,
    title: Option<String>,
    properties: Vec<(String, String)>,
    body: Option<String>,
}

impl HeadlineBuilder {
    /// Set the heading level (1 = top-level `*`, 2 = `**`, etc.).
    pub fn level(mut self, level: usize) -> Self {
        self.level = Some(level);
        self
    }

    /// Set the TODO status keyword (e.g. `"TODO"`, `"DONE"`).
    pub fn status(mut self, status: &str) -> Self {
        self.status = Some(status.to_string());
        self
    }

    /// Set the headline title text.
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    /// Add a headline-level property (e.g. `("ID", "some-uuid")`).
    pub fn property(mut self, key: &str, value: &str) -> Self {
        self.properties.push((key.to_string(), value.to_string()));
        self
    }

    /// Set the body text for the headline.
    pub fn body(mut self, body: &str) -> Self {
        self.body = Some(body.to_string());
        self
    }

    /// Consume the builder and produce a `Headline`.
    pub fn build(self) -> Headline {
        Headline {
            level: self.level.unwrap_or(1),
            status: self.status,
            title: self.title.unwrap_or_default(),
            properties: self.properties,
            body: self.body.filter(|b| !b.is_empty()),
        }
    }
}

// ---------------------------------------------------------------------------
// Display implementations
// ---------------------------------------------------------------------------

fn write_property_drawer(f: &mut fmt::Formatter<'_>, props: &[(String, String)]) -> fmt::Result {
    if props.is_empty() {
        return Ok(());
    }
    writeln!(f, ":PROPERTIES:")?;
    for (key, value) in props {
        writeln!(f, ":{key}:       {value}")?;
    }
    writeln!(f, ":END:")
}

impl fmt::Display for Headline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Headline marker
        let stars = "*".repeat(self.level);
        write!(f, "{stars}")?;
        if let Some(status) = &self.status {
            write!(f, " {status}")?;
        }
        writeln!(f, " {}", self.title)?;

        // Property drawer (must come before body in org-mode)
        write_property_drawer(f, &self.properties)?;

        // Body
        if let Some(body) = &self.body {
            writeln!(f, "{body}")?;
        }

        Ok(())
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Document-level property drawer
        write_property_drawer(f, &self.properties)?;

        // Keywords
        if let Some(title) = &self.title {
            writeln!(f, "#+TITLE: {title}")?;
        }
        if let Some(tags) = &self.filetags {
            writeln!(f, "#+FILETAGS: {tags}")?;
        }

        // Blank line before first headline
        if !self.headlines.is_empty() {
            writeln!(f)?;
        }

        // Headlines separated by blank lines
        let mut iter = self.headlines.iter();
        if let Some(first) = iter.next() {
            write!(f, "{first}")?;
        }
        for headline in iter {
            writeln!(f)?; // blank line between headlines
            write!(f, "{headline}")?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience accessors
// ---------------------------------------------------------------------------

impl Document {
    /// Create a new `DocumentBuilder`.
    pub fn builder() -> DocumentBuilder {
        DocumentBuilder::default()
    }
}

impl Headline {
    /// Create a new `HeadlineBuilder`.
    pub fn builder() -> HeadlineBuilder {
        HeadlineBuilder::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_document() {
        let doc = Document::builder().build();
        assert_eq!(doc.to_string(), "");
    }

    #[test]
    fn test_document_with_title_only() {
        let doc = Document::builder().title("Test").build();
        assert_eq!(doc.to_string(), "#+TITLE: Test\n");
    }

    #[test]
    fn test_document_with_properties_and_title() {
        let doc = Document::builder()
            .property("ID", "abc-123")
            .title("My Doc")
            .filetags("project")
            .build();
        let expected = "\
:PROPERTIES:
:ID:       abc-123
:END:
#+TITLE: My Doc
#+FILETAGS: project
";
        assert_eq!(doc.to_string(), expected);
    }

    #[test]
    fn test_headline_minimal() {
        let h = Headline::builder().level(1).title("Task").build();
        assert_eq!(h.to_string(), "* Task\n");
    }

    #[test]
    fn test_headline_with_status() {
        let h = Headline::builder()
            .level(1)
            .status("TODO")
            .title("Buy milk")
            .build();
        assert_eq!(h.to_string(), "* TODO Buy milk\n");
    }

    #[test]
    fn test_headline_with_properties() {
        let h = Headline::builder()
            .level(1)
            .status("TODO")
            .title("Buy milk")
            .property("ID", "uuid-123")
            .build();
        let expected = "\
* TODO Buy milk
:PROPERTIES:
:ID:       uuid-123
:END:
";
        assert_eq!(h.to_string(), expected);
    }

    #[test]
    fn test_headline_with_body() {
        let h = Headline::builder()
            .level(1)
            .status("TODO")
            .title("Buy milk")
            .property("ID", "uuid-123")
            .body("Milk, eggs, bread")
            .build();
        let expected = "\
* TODO Buy milk
:PROPERTIES:
:ID:       uuid-123
:END:
Milk, eggs, bread
";
        assert_eq!(h.to_string(), expected);
    }

    #[test]
    fn test_full_document() {
        let doc = Document::builder()
            .property("ID", "project-uuid")
            .title("Sprint 12")
            .filetags("project")
            .headline(
                Headline::builder()
                    .level(1)
                    .status("TODO")
                    .title("Fix login")
                    .property("ID", "task-1")
                    .body("Investigate redirect issue")
                    .build(),
            )
            .headline(
                Headline::builder()
                    .level(1)
                    .status("DONE")
                    .title("Setup CI")
                    .property("ID", "task-2")
                    .build(),
            )
            .build();
        let expected = "\
:PROPERTIES:
:ID:       project-uuid
:END:
#+TITLE: Sprint 12
#+FILETAGS: project

* TODO Fix login
:PROPERTIES:
:ID:       task-1
:END:
Investigate redirect issue

* DONE Setup CI
:PROPERTIES:
:ID:       task-2
:END:
";
        assert_eq!(doc.to_string(), expected);
    }

    #[test]
    fn test_headline_level_2() {
        let h = Headline::builder()
            .level(2)
            .status("TODO")
            .title("Sub task")
            .build();
        assert_eq!(h.to_string(), "** TODO Sub task\n");
    }

    #[test]
    fn test_headline_empty_body_not_included() {
        let h = Headline::builder()
            .level(1)
            .title("Task")
            .body("")
            .build();
        assert_eq!(h.to_string(), "* Task\n");
    }
}
