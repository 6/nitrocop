use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/MagicCommentFormat enforces separator style plus directive capitalization
/// on leading magic comments.
///
/// Investigation findings (2026-03-30):
/// - FN root cause: this cop only checked `_` vs `-` separators, so directives like
///   `# Encoding: utf-8` were missed under RuboCop's default
///   `DirectiveCapitalization: lowercase` setting.
/// - Fix: combine separator and capitalization checks into the directive offense so
///   `Encoding` now reports `Prefer lower snake case for magic comments.` without
///   changing the existing separator matches.
///
/// Investigation findings (2026-04-06):
/// - FN root cause (kebab_case variant): files with UTF-8 BOM (`\u{feff}`) prefix
///   were not detected as magic comments because the BOM prefix caused the line to
///   not start with `#`. This caused FNs for files like `# frozen_string_literal: true`
///   with `EnforcedStyle: kebab_case`.
/// - Fix: strip UTF-8 BOM from each line before checking if it starts with `#`.
///   The BOM is now removed before processing the line content.
pub struct MagicCommentFormat;

const MAGIC_COMMENT_DIRECTIVES: &[&str] = &[
    "frozen_string_literal",
    "frozen-string-literal",
    "encoding",
    "shareable_constant_value",
    "shareable-constant-value",
    "typed",
    "warn_indent",
    "warn-indent",
];

impl MagicCommentFormat {
    /// UTF-8 BOM character that may precede magic comments in some files.
    const UTF8_BOM: &str = "\u{feff}";

    fn strip_bom(s: &str) -> &str {
        s.strip_prefix(Self::UTF8_BOM).unwrap_or(s)
    }

    fn directive_capitalization(config: &CopConfig) -> Option<&str> {
        match config.options.get("DirectiveCapitalization") {
            Some(value) => value.as_str(),
            None => Some("lowercase"),
        }
    }

    fn is_magic_comment_directive(word: &str) -> bool {
        let normalized = word.replace(['-', '_'], "_").to_lowercase();
        MAGIC_COMMENT_DIRECTIVES
            .iter()
            .any(|&d| d.replace('-', "_").to_lowercase() == normalized)
    }

    fn has_underscores(s: &str) -> bool {
        s.contains('_')
    }

    fn has_dashes(s: &str) -> bool {
        s.contains('-')
    }

    fn wrong_capitalization(text: &str, expected: Option<&str>) -> bool {
        match expected {
            Some("lowercase") => text != text.to_lowercase(),
            Some("uppercase") => text != text.to_uppercase(),
            _ => false,
        }
    }

    fn expected_style(style: &str, directive_capitalization: Option<&str>) -> Option<String> {
        let mut parts = Vec::new();

        match directive_capitalization {
            Some("lowercase") => parts.push("lower"),
            Some("uppercase") => parts.push("upper"),
            _ => {}
        }

        match style {
            "snake_case" => parts.push("snake"),
            "kebab_case" => parts.push("kebab"),
            _ => return None,
        }

        Some(parts.join(" "))
    }
}

impl Cop for MagicCommentFormat {
    fn name(&self) -> &'static str {
        "Style/MagicCommentFormat"
    }

    fn check_lines(
        &self,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let lines: Vec<&str> = source
            .lines()
            .filter_map(|l| std::str::from_utf8(l).ok())
            .collect();
        let style = config.get_str("EnforcedStyle", "snake_case");
        let directive_capitalization = Self::directive_capitalization(config);
        let _value_capitalization = config.get_str("ValueCapitalization", "");

        // Only check lines before the first code statement
        for (i, line) in lines.iter().enumerate() {
            let line = Self::strip_bom(line);
            let trimmed = line.trim();

            // Stop at first non-comment, non-blank line
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                break;
            }

            if !trimmed.starts_with('#') {
                continue;
            }

            let content = &trimmed[1..].trim_start();

            // Handle emacs-style: # -*- key: value; key: value -*-
            let is_emacs = content.starts_with("-*-");

            if is_emacs {
                // Parse emacs-style directives
                let inner = content
                    .trim_start_matches("-*-")
                    .trim_end_matches("-*-")
                    .trim();
                for part in inner.split(';') {
                    let part = part.trim();
                    if let Some(colon_pos) = part.find(':') {
                        let directive = part[..colon_pos].trim();
                        if Self::is_magic_comment_directive(directive) {
                            if let Some(diagnostic) = self.check_directive_style(
                                source,
                                i,
                                line,
                                directive,
                                style,
                                directive_capitalization,
                            ) {
                                diagnostics.push(diagnostic);
                            }
                        }
                    }
                }
            } else {
                // Standard style: # directive: value
                if let Some(colon_pos) = content.find(':') {
                    let directive = content[..colon_pos].trim();
                    if Self::is_magic_comment_directive(directive) {
                        if let Some(diagnostic) = self.check_directive_style(
                            source,
                            i,
                            line,
                            directive,
                            style,
                            directive_capitalization,
                        ) {
                            diagnostics.push(diagnostic);
                        }
                    }
                }
            }
        }
    }
}

impl MagicCommentFormat {
    fn check_directive_style(
        &self,
        source: &SourceFile,
        line_idx: usize,
        line: &str,
        directive: &str,
        style: &str,
        directive_capitalization: Option<&str>,
    ) -> Option<Diagnostic> {
        let wrong_separator = match style {
            "snake_case" => Self::has_dashes(directive),
            "kebab_case" => Self::has_underscores(directive),
            _ => false,
        };
        let wrong_capitalization = Self::wrong_capitalization(directive, directive_capitalization);

        if wrong_separator || wrong_capitalization {
            // Find the directive position in the line
            if let Some(pos) = line.find(directive) {
                let line_num = line_idx + 1;
                let expected_style = Self::expected_style(style, directive_capitalization)?;
                let msg = format!("Prefer {expected_style} case for magic comments.");
                return Some(self.diagnostic(source, line_num, pos, msg));
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(MagicCommentFormat, "cops/style/magic_comment_format");

    fn kebab_case_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("kebab_case".to_string()),
        );
        options.insert(
            "DirectiveCapitalization".to_string(),
            serde_yml::Value::String("lowercase".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_kebab_case() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &MagicCommentFormat,
            include_bytes!(
                "../../../tests/fixtures/cops/style/magic_comment_format/kebab_case_offense.rb"
            ),
            kebab_case_config(),
        );
    }

    #[test]
    fn no_offense_kebab_case() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &MagicCommentFormat,
            include_bytes!(
                "../../../tests/fixtures/cops/style/magic_comment_format/kebab_case_no_offense.rb"
            ),
            kebab_case_config(),
        );
    }
}
