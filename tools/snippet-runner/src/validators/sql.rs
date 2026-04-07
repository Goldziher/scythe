use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::SnippetValidator;

pub struct SqlValidator;

impl SnippetValidator for SqlValidator {
    fn language(&self) -> Language {
        Language::Sql
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        // SQL validation is a pass-through for now.
        // Future: integrate with scythe's own SQL parser.
        Ok((SnippetStatus::Pass, None))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Syntax
    }
}
