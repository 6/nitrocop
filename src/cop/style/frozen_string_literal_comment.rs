use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Matches RuboCop's leading magic-comment scan more closely in encoded files.
///
/// This cop previously bailed out on any non-UTF-8 byte or any NUL byte in the file, which hid
/// real offenses when the header was plain ASCII but the body used EUC-JP/ISO-8859 bytes, a
/// gemspec stub embedded a later NUL, or the whole file was UTF-16LE without a BOM. The fix keeps
/// the top-of-file scan byte-oriented so those headers still produce offenses, while preserving the
/// early return for truly undecodable files that do not advertise an encoding in the leading lines.
pub struct FrozenStringLiteralComment;

impl Cop for FrozenStringLiteralComment {
    fn name(&self) -> &'static str {
        "Style/FrozenStringLiteralComment"
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/*.arb"]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_lines(
        &self,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "always");

        let lines: Vec<&[u8]> = source.lines().collect();
        let has_null_bytes = source.as_bytes().contains(&0x00);

        // RuboCop flags UTF-16LE files without a BOM, but not the BE variant.
        if looks_like_utf16be_without_bom(source.as_bytes()) {
            return;
        }

        // Preserve the old early return only for truly undecodable files that lack a leading
        // encoding directive. RuboCop still flags files whose magic comments are plain ASCII even
        // when later bytes are non-UTF-8 or UTF-16/NUL-padded.
        if !has_null_bytes
            && std::str::from_utf8(source.as_bytes()).is_err()
            && !has_leading_encoding_comment(&lines)
        {
            return;
        }

        if enforced_style == "never" {
            // Flag the presence of frozen_string_literal comment as unnecessary
            for (i, line) in lines.iter().enumerate() {
                if is_frozen_string_literal_comment(line) {
                    let mut diag = self.diagnostic(
                        source,
                        i + 1,
                        0,
                        "Unnecessary frozen string literal comment.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        // Delete the entire line including its newline
                        if let Some(start) = source.line_col_to_offset(i + 1, 0) {
                            let end = source
                                .line_col_to_offset(i + 2, 0)
                                .unwrap_or(source.as_bytes().len());
                            corr.push(crate::correction::Correction {
                                start,
                                end,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                    }
                    diagnostics.push(diag);
                }
            }
            return;
        }

        // Skip empty files — RuboCop returns early when there are no tokens.
        // Lint/EmptyFile handles these instead.
        let has_content = lines
            .iter()
            .any(|line| first_non_padding_byte(line).is_some());
        if !has_content {
            return;
        }

        // RuboCop skips bare data files that start with `__END__`, but it still flags files
        // that have leading comments before `__END__`.
        if starts_with_end_data_only(&lines) {
            return;
        }

        let mut idx = 0;

        // Skip shebang
        if idx < lines.len() && starts_with_shebang(lines[idx]) {
            idx += 1;
        }

        // Skip blank lines after shebang (RuboCop scans all lines before the first
        // non-comment token, so blank lines don't break the search)
        while idx < lines.len() && is_blank_line(lines[idx]) {
            idx += 1;
        }

        // Skip encoding comment, but check if it also contains frozen_string_literal
        // (Emacs-style: # -*- encoding: utf-8; frozen_string_literal: true -*-)
        if idx < lines.len() && is_encoding_comment(lines[idx]) {
            if is_frozen_string_literal_comment(lines[idx]) {
                if enforced_style == "always_true" && !is_frozen_string_literal_true(lines[idx]) {
                    diagnostics.push(self.diagnostic(
                        source,
                        idx + 1,
                        0,
                        "Frozen string literal comment must be set to `true`.".to_string(),
                    ));
                }
                return;
            }
            idx += 1;
        }

        // Remember where to insert the comment (after shebang/encoding)
        let insert_after_line = idx; // 0-indexed line number

        // Scan leading comment and blank lines for the frozen_string_literal magic comment.
        // RuboCop's `leading_comment_lines` returns all lines before the first non-comment
        // token — blank lines are included since they don't produce tokens.
        while idx < lines.len() && is_comment_or_blank_line(lines[idx]) {
            if is_frozen_string_literal_comment(lines[idx]) {
                if enforced_style == "always_true" {
                    // Must be set to true specifically
                    if !is_frozen_string_literal_true(lines[idx]) {
                        let mut diag = self.diagnostic(
                            source,
                            idx + 1,
                            0,
                            "Frozen string literal comment must be set to `true`.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            // Replace the entire line with the correct comment
                            if let Some(start) = source.line_col_to_offset(idx + 1, 0) {
                                let end = source
                                    .line_col_to_offset(idx + 2, 0)
                                    .unwrap_or(source.as_bytes().len());
                                corr.push(crate::correction::Correction {
                                    start,
                                    end,
                                    replacement: "# frozen_string_literal: true\n".to_string(),
                                    cop_name: self.name(),
                                    cop_index: 0,
                                });
                                diag.corrected = true;
                            }
                        }
                        diagnostics.push(diag);
                    }
                }
                return;
            }
            idx += 1;
        }

        let msg = if enforced_style == "always_true" {
            "Missing magic comment `# frozen_string_literal: true`."
        } else {
            "Missing frozen string literal comment."
        };
        let mut diag = self.diagnostic(source, 1, 0, msg.to_string());
        if let Some(ref mut corr) = corrections {
            // Insert after shebang/encoding lines
            let insert_offset = source
                .line_col_to_offset(insert_after_line + 1, 0)
                .unwrap_or(0);
            corr.push(crate::correction::Correction {
                start: insert_offset,
                end: insert_offset,
                replacement: "# frozen_string_literal: true\n".to_string(),
                cop_name: self.name(),
                cop_index: 0,
            });
            diag.corrected = true;
        }
        diagnostics.push(diag);
    }
}

/// Returns true when the file is bare `__END__` data: its first non-blank line is `__END__`
/// with no leading comments or shebang lines ahead of it.
fn starts_with_end_data_only(lines: &[&[u8]]) -> bool {
    for line in lines {
        match first_non_padding_byte(line) {
            None => continue,
            Some(b'#') => return false,
            Some(b'_') => {
                let trimmed = trimmed_ascii_bytes(line);
                return trimmed.starts_with(b"__END__");
            }
            Some(_) => return false,
        }
    }
    false
}

fn normalized_ascii_bytes(line: &[u8]) -> Vec<u8> {
    line.iter()
        .copied()
        .filter(|&b| b != 0x00 && b.is_ascii())
        .collect()
}

fn looks_like_utf16be_without_bom(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return false;
    }

    let sample: Vec<[u8; 2]> = bytes
        .chunks_exact(2)
        .take(6)
        .filter_map(|pair| <[u8; 2]>::try_from(pair).ok())
        .collect();

    sample.len() >= 3
        && sample
            .iter()
            .all(|pair| pair[0] == 0x00 && pair[1].is_ascii())
        && sample
            .iter()
            .any(|pair| matches!(pair[1], b'#' | b' ' | b'e' | b'c'))
}

fn trimmed_ascii_bytes(line: &[u8]) -> Vec<u8> {
    let ascii = normalized_ascii_bytes(line);
    let start = ascii
        .iter()
        .position(|&b| b != b' ' && b != b'\t' && b != b'\r')
        .unwrap_or(ascii.len());
    ascii[start..].to_vec()
}

fn normalized_ascii_string(line: &[u8]) -> String {
    String::from_utf8(normalized_ascii_bytes(line)).expect("ASCII normalization must stay ASCII")
}

fn first_non_padding_byte(line: &[u8]) -> Option<u8> {
    line.iter()
        .copied()
        .filter(|&b| b != 0x00)
        .find(|&b| b != b' ' && b != b'\t' && b != b'\r')
}

fn starts_with_shebang(line: &[u8]) -> bool {
    normalized_ascii_bytes(line).starts_with(b"#!")
}

fn is_comment_line(line: &[u8]) -> bool {
    first_non_padding_byte(line) == Some(b'#')
}

fn is_blank_line(line: &[u8]) -> bool {
    first_non_padding_byte(line).is_none()
}

fn is_comment_or_blank_line(line: &[u8]) -> bool {
    is_blank_line(line) || is_comment_line(line)
}

fn is_encoding_comment(line: &[u8]) -> bool {
    let s = normalized_ascii_string(line);
    let trimmed = s.trim_start_matches([' ', '\t']);
    let lower = trimmed.to_ascii_lowercase();
    // Explicit encoding/coding directive: `# encoding: utf-8` or `# coding: utf-8`
    if lower.starts_with("# encoding:") || lower.starts_with("# coding:") {
        return true;
    }
    // Emacs-style mode line: `# -*- encoding: utf-8 -*-` or `# -*- coding: utf-8 -*-`
    // The space before the colon is optional: `# -*- encoding : utf-8 -*-`
    if lower.starts_with("# -*-") {
        return lower.contains("encoding") || lower.contains("coding");
    }
    false
}

fn has_leading_encoding_comment(lines: &[&[u8]]) -> bool {
    let mut idx = 0;

    if idx < lines.len() && starts_with_shebang(lines[idx]) {
        idx += 1;
    }

    while idx < lines.len() && is_blank_line(lines[idx]) {
        idx += 1;
    }

    idx < lines.len() && is_encoding_comment(lines[idx])
}

/// Match `frozen_string_literal:` or `frozen-string-literal:` case-insensitively,
/// consistent with RuboCop's regex `frozen[_-]string[_-]literal` with `/i` flag.
///
/// For simple comments, RuboCop requires the key to be the ONLY content after `#`:
///   `\A\s*#\s*frozen[_-]string[_-]literal:\s*TOKEN\s*\z`
/// This means `# # frozen_string_literal: true` (double-hash) is NOT valid.
///
/// For Emacs-style comments (`# -*- ... -*-`), the key can appear anywhere
/// among semicolon-separated directives.
fn is_frozen_string_literal_comment(line: &[u8]) -> bool {
    frozen_string_literal_value(line)
        .as_deref()
        .is_some_and(is_frozen_string_literal_boolean)
}

/// Check if a string STARTS WITH `frozen_string_literal:` or `frozen-string-literal:`
/// (case-insensitive, allowing hyphens or underscores as separators).
/// Used for simple (non-Emacs) magic comments where the key must be at the start.
fn starts_with_frozen_string_literal_key(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    // "frozen" (6) + sep (1) + "string" (6) + sep (1) + "literal:" (8) = 22 chars
    bytes.starts_with(b"frozen")
        && bytes.len() >= 22
        && (bytes[6] == b'_' || bytes[6] == b'-')
        && bytes[7..].starts_with(b"string")
        && (bytes[13] == b'_' || bytes[13] == b'-')
        && bytes[14..].starts_with(b"literal:")
}

fn is_frozen_string_literal_true(line: &[u8]) -> bool {
    frozen_string_literal_value(line)
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn frozen_string_literal_value(line: &[u8]) -> Option<String> {
    let normalized = normalized_ascii_string(line);
    let trimmed = normalized.trim_start().strip_prefix('#')?.trim_start();
    if trimmed.starts_with("-*-") && trimmed.ends_with("-*-") {
        let after_key = strip_frozen_string_literal_key(trimmed)?;
        return Some(
            after_key
                .split([';', '-'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
        );
    }
    let after_key = strip_prefix_frozen_string_literal_key(trimmed)?;
    Some(after_key.trim().to_string())
}

fn is_frozen_string_literal_boolean(value: &str) -> bool {
    value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false")
}

/// If the string STARTS WITH `frozen[_-]string[_-]literal:` (case-insensitive),
/// return the portion after the colon. Used for simple (non-Emacs) comments.
fn strip_prefix_frozen_string_literal_key(s: &str) -> Option<&str> {
    if starts_with_frozen_string_literal_key(s) {
        Some(&s[22..])
    } else {
        None
    }
}

/// If the string contains `frozen[_-]string[_-]literal:` (case-insensitive),
/// return the portion after the colon.
fn strip_frozen_string_literal_key(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    // "frozen" (6) + sep (1) + "string" (6) + sep (1) + "literal:" (8) = 22 chars
    for i in 0..bytes.len() {
        if bytes[i..].starts_with(b"frozen")
            && i + 22 <= bytes.len()
            && (bytes[i + 6] == b'_' || bytes[i + 6] == b'-')
            && bytes[i + 7..].starts_with(b"string")
            && (bytes[i + 13] == b'_' || bytes[i + 13] == b'-')
            && bytes[i + 14..].starts_with(b"literal:")
        {
            return Some(&s[i + 22..]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn utf16be_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    crate::cop_scenario_fixture_tests!(
        FrozenStringLiteralComment,
        "cops/style/frozen_string_literal_comment",
        plain_missing = "plain_missing.rb",
        shebang_missing = "shebang_missing.rb",
        encoding_missing = "encoding_missing.rb",
        generated_gemspec_missing = "generated_gemspec_missing.rb",
        double_hash_frozen = "double_hash_frozen.rb",
        invalid_token = "invalid_token.rb",
        comment_before_end = "comment_before_end.rb",
        encoding_before_end = "encoding_before_end.rb",
    );

    #[test]
    fn missing_comment() {
        let source = SourceFile::from_bytes("test.rb", b"puts 'hello'\n".to_vec());
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 0);
        assert_eq!(diags[0].message, "Missing frozen string literal comment.");
    }

    #[test]
    fn with_frozen_true() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn with_frozen_false() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen_string_literal: false\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn with_shebang_and_frozen() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#!/usr/bin/env ruby\n# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn with_shebang_no_frozen() {
        let source =
            SourceFile::from_bytes("test.rb", b"#!/usr/bin/env ruby\nputs 'hello'\n".to_vec());
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn with_encoding_and_frozen() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# encoding: utf-8\n# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn with_shebang_encoding_and_frozen() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#!/usr/bin/env ruby\n# encoding: utf-8\n# frozen_string_literal: true\nputs 'hello'\n"
                .to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn empty_file() {
        // Empty files should not be flagged — Lint/EmptyFile handles them
        let source = SourceFile::from_bytes("test.rb", b"".to_vec());
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty(), "Empty files should not be flagged");
    }

    #[test]
    fn emacs_encoding_style() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# -*- coding: utf-8 -*-\n# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn emacs_encoding_with_spaces() {
        // Emacs mode line with spaces around colon: `# -*- encoding : utf-8 -*-`
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# -*- encoding : utf-8 -*-\n# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize encoding comment with spaces around colon"
        );
    }

    #[test]
    fn enforced_style_never_flags_presence() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("never".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Unnecessary"));
    }

    #[test]
    fn enforced_style_never_allows_missing() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("never".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("test.rb", b"puts 'hello'\n".to_vec());
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &config, &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should not flag missing comment with 'never' style"
        );
    }

    #[test]
    fn enforced_style_always_true_flags_false() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("always_true".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen_string_literal: false\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("must be set to `true`"));
    }

    #[test]
    fn enforced_style_always_true_allows_true() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("always_true".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &config, &mut diags, None);
        assert!(diags.is_empty(), "Should allow true with always_true style");
    }

    #[test]
    fn leading_whitespace_recognized() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"  # frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen_string_literal with leading whitespace"
        );
    }

    #[test]
    fn with_typed_comment_before_frozen() {
        // Sorbet typed: true comment before frozen_string_literal should be recognized
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# typed: true\n# frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should find frozen_string_literal after # typed: true"
        );
    }

    #[test]
    fn with_shebang_and_typed_and_frozen() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#!/usr/bin/env ruby\n# typed: strict\n# frozen_string_literal: true\nputs 'hello'\n"
                .to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should find frozen_string_literal after shebang + typed comment"
        );
    }

    #[test]
    fn emacs_combined_encoding_and_frozen() {
        // Emacs-style: # -*- encoding: utf-8; frozen_string_literal: true -*-
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# -*- encoding: utf-8; frozen_string_literal: true -*-\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen_string_literal in Emacs-style combined comment"
        );
    }

    #[test]
    fn emacs_frozen_only() {
        // Emacs-style with only frozen_string_literal
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# -*- frozen_string_literal: true -*-\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize Emacs-style frozen_string_literal-only comment"
        );
    }

    #[test]
    fn emacs_combined_frozen_false() {
        // Emacs-style with frozen_string_literal: false — should still count as present
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# -*- encoding: utf-8; frozen_string_literal: false -*-\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen_string_literal: false in Emacs-style comment"
        );
    }

    #[test]
    fn emacs_combined_with_shebang() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#!/usr/bin/env ruby\n# -*- encoding: utf-8; frozen_string_literal: true -*-\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize Emacs-style comment after shebang"
        );
    }

    #[test]
    fn blank_line_between_shebang_and_frozen() {
        // FP pattern: shebang, blank line, then frozen_string_literal
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#!/usr/bin/env ruby\n\n# frozen_string_literal: true\n\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen_string_literal after shebang + blank line"
        );
    }

    #[test]
    fn leading_blank_line_before_frozen() {
        // FP pattern: blank line at start, then frozen_string_literal
        let source = SourceFile::from_bytes(
            "test.rb",
            b"\n# frozen_string_literal: true\n\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen_string_literal after leading blank line"
        );
    }

    #[test]
    fn case_insensitive_frozen_string_literal() {
        // FP pattern: typo with different case like frozen_sTring_literal
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen_sTring_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen_string_literal case-insensitively"
        );
    }

    #[test]
    fn hyphen_separator_frozen_string_literal() {
        // FP pattern: hyphens instead of underscores (frozen-string-literal)
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# frozen-string-literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should recognize frozen-string-literal with hyphens"
        );
    }

    #[test]
    fn shebang_blank_line_encoding_frozen() {
        // shebang, blank line, encoding, frozen_string_literal
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#!/usr/bin/env ruby\n\n# encoding: ascii-8bit\n# frozen_string_literal: true\n\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should find frozen_string_literal after shebang + blank + encoding"
        );
    }

    #[test]
    fn skip_file_with_invalid_utf8() {
        // Files with invalid UTF-8 bytes should not be flagged — RuboCop's tokenizer
        // produces no tokens for these, so it returns early.
        let mut content = b"# @!method foo()\n# \treturn [String] ".to_vec();
        content.push(0xFF); // invalid UTF-8 byte
        content.push(b'\n');
        let source = SourceFile::from_bytes("test.rb", content);
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should not flag files with invalid UTF-8 bytes"
        );
    }

    #[test]
    fn flags_utf16le_file_without_bom() {
        let content = utf16le_bytes("# encoding: utf-16le\nputs 'hello'\n");
        let source = SourceFile::from_bytes("test.rb", content);
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "UTF-16LE files should still be flagged for a missing comment"
        );
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn skips_utf16be_file_without_bom() {
        let content = utf16be_bytes("# encoding: utf-16be\nputs 'hello'\n");
        let source = SourceFile::from_bytes("test.rb", content);
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "UTF-16BE files without a BOM should remain unflagged"
        );
    }

    #[test]
    fn flags_non_utf8_file_with_encoding_comment() {
        let mut content = b"# encoding: EUC-JP\nputs \"135".to_vec();
        content.extend_from_slice(&[0xA1, 0xA1]);
        content.extend_from_slice(b"C\"\n");
        let source = SourceFile::from_bytes("test.rb", content);
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "Non-UTF-8 files with a leading encoding comment should be flagged"
        );
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn flags_ascii_header_when_file_contains_later_null_bytes() {
        let mut content = b"# Generated by juwelier\n# DO NOT EDIT THIS FILE DIRECTLY\n# Instead, edit Juwelier::Tasks in Rakefile, and run 'rake gemspec'\n# -*- encoding: utf-8 -*-\n# stub: glimmer-dsl-swt 4.30.1.1 ruby lib".to_vec();
        content.push(0x00);
        content.extend_from_slice(
            b"bin\n\nGem::Specification.new do |s|\n  s.name = \"glimmer-dsl-swt\".freeze\nend\n",
        );
        let source = SourceFile::from_bytes("test.rb", content);
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "Later null bytes should not suppress the missing comment offense"
        );
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn skip_file_starting_with_end_only() {
        // Files that start with __END__ and have no code should not be flagged
        let source = SourceFile::from_bytes("test.rb", b"__END__\ndata only\n".to_vec());
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "Should not flag files starting with __END__"
        );
    }

    #[test]
    fn double_hash_not_valid_magic_comment() {
        // `# # frozen_string_literal: true` has a double hash prefix — not a valid magic comment.
        // RuboCop's SimpleComment regex requires `\A\s*#\s*frozen...` (one # only).
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# # frozen_string_literal: true\nputs 'hello'\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "Double-hash frozen_string_literal should NOT be recognized as valid"
        );
    }

    #[test]
    fn encoding_only_no_frozen() {
        // File with encoding magic comment but no frozen string literal comment should be flagged.
        let source = SourceFile::from_bytes(
            "test.rb",
            b"# -*- encoding: iso-8859-9 -*-\nclass Foo; end\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "File with encoding-only magic comment should be flagged"
        );
    }

    #[test]
    fn comment_only_file_not_flagged() {
        // A file with only comments and no real code tokens should not be flagged.
        // RuboCop returns early via `processed_source.tokens.empty?`.
        let source = SourceFile::from_bytes(
            "test.rb",
            b"#~# ORIGINAL retry\n\nretry\n\n#~# EXPECTED\nretry\n".to_vec(),
        );
        let mut diags = Vec::new();
        FrozenStringLiteralComment.check_lines(&source, &CopConfig::default(), &mut diags, None);
        // This file HAS code tokens (retry), so it should be flagged.
        // The FP in the corpus is likely a config issue (file extension .rb.spec), not cop logic.
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn autocorrect_insert_missing() {
        let input = b"puts 'hello'\n";
        let (diags, corrections) =
            crate::testutil::run_cop_autocorrect(&FrozenStringLiteralComment, input);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"# frozen_string_literal: true\nputs 'hello'\n");
    }

    #[test]
    fn autocorrect_insert_after_shebang() {
        let input = b"#!/usr/bin/env ruby\nputs 'hello'\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect(&FrozenStringLiteralComment, input);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(
            corrected,
            b"#!/usr/bin/env ruby\n# frozen_string_literal: true\nputs 'hello'\n"
        );
    }

    #[test]
    fn autocorrect_insert_after_encoding() {
        let input = b"# encoding: utf-8\nputs 'hello'\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect(&FrozenStringLiteralComment, input);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(
            corrected,
            b"# encoding: utf-8\n# frozen_string_literal: true\nputs 'hello'\n"
        );
    }

    #[test]
    fn autocorrect_remove_never_style() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("never".into()),
            )]),
            ..CopConfig::default()
        };
        let input = b"# frozen_string_literal: true\nputs 'hello'\n";
        let (diags, corrections) = crate::testutil::run_cop_autocorrect_with_config(
            &FrozenStringLiteralComment,
            input,
            config,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"puts 'hello'\n");
    }

    #[test]
    fn autocorrect_always_true_replaces_false() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("always_true".into()),
            )]),
            ..CopConfig::default()
        };
        let input = b"# frozen_string_literal: false\nputs 'hello'\n";
        let (diags, corrections) = crate::testutil::run_cop_autocorrect_with_config(
            &FrozenStringLiteralComment,
            input,
            config,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"# frozen_string_literal: true\nputs 'hello'\n");
    }
}
