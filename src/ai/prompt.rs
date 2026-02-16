//! Reusable prompts using Handlebars for templating. Handlebars adds
//! additional security controls since it can't do much out of the box
//! without registering your own helpers. This is ideal since output
//! from LLMs should be considered untrusted and Handlebars forces you
//! to add only what you need.

use std::fmt;

use handlebars::{Handlebars, handlebars_helper};

// A simple `inc` helper for use with `each` and `@index` so that
// there can be natural number sequences when rendering (instead of
// starting at 0).
handlebars_helper!(inc: |v: i64| format!("{}", v + 1));

#[derive(Debug)]
pub enum Prompt {
    NoteSummary,
    UnreadEmails,
}

impl fmt::Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

// Implement the Into trait so that Prompt can be converted to an &str
impl From<Prompt> for String {
    fn from(item: Prompt) -> String {
        format!("{:?}", item)
    }
}

const NOTES_PROMPT: &str = r"
Summarize this context (CONTEXT) concisely. Always include a list of sources (SOURCES) with the title and file name if available.

CONTEXT:
{{context}}
";

const UNREAD_EMAILS_PROMPT: &str = r"
The following is a list of unread emails and their related email thread in reverse chronological order.

# Unread Emails
{{#each email_threads}}

## {{subject}}

**ID:** {{id}}
**From:** {{from}}
**To:** {{to}}
**Subject:** {{subject}}

{{#each messages}}
### Message {{inc @index}}

**From:** {{from}}
**To:** {{to}}
**Date:** {{received}}
**Subject:** {{subject}}
**Body:**
{{body}}

---

{{/each}}
{{/each}}
";

pub fn templates<'a>() -> Handlebars<'a> {
    let mut registry = Handlebars::new();
    registry.set_strict_mode(true);
    registry.register_helper("inc", Box::new(inc));
    registry
        .register_template_string(&Prompt::NoteSummary.to_string(), NOTES_PROMPT)
        .expect("Failed to register template");
    registry
        .register_template_string(&Prompt::UnreadEmails.to_string(), UNREAD_EMAILS_PROMPT)
        .expect("Failed to register template");
    registry
}
