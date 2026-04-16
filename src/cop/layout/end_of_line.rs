use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for Windows-style line endings in the source code.
///
/// ## Variant behavior
///
/// For `EnforcedStyle: crlf`, RuboCop's `unimportant_missing_cr?` skips reporting
/// when the last line has no LF at all (i.e., a file with no trailing newline).
/// This is because the "missing CR" on the last line is considered unimportant
/// when there is no line break to attach it to. nitrocop previously flagged such
/// files as "Carriage return character missing." on every line, causing 108 false
/// positives in the corpus.
///
/// RuboCop also skips `Layout/EndOfLine` entirely when `processed_source.valid_syntax?`
/// is false. The `crlf` variant exposed four false positives from files that never
/// reach this cop in RuboCop: malformed encoding-comment headers that emit only
/// `Lint/Syntax`, and Ruby 4.0 `retry` files where `valid_syntax?` is false.
/// nitrocop previously ran this cop from `check_lines()` before parse context
/// existed, so it still reported missing carriage returns. The fix moves the
/// implementation to `check_source()` and mirrors RuboCop's fatal-syntax gate.
///
/// @example EnforcedStyle: native (default)
///   # The `native` style means that CR+LF (Carriage Return + Line Feed) is
///   # enforced on Windows, and LF is enforced on other platforms.
///   # bad
///   puts 'Hello' # Return character is LF on Windows.
///   puts 'Hello' # Return character is CR+LF on other than Windows.
///   # good
///   puts 'Hello' # Return character is CR+LF on Windows.
///   puts 'Hello' # Return character is LF on other than Windows.
///
/// @example EnforcedStyle: lf
///   # The `lf` style means that LF (Line Feed) is enforced on all platforms.
///   # bad
///   puts 'Hello' # Return character is CR+LF on all platforms.
///   # good
///   puts 'Hello' # Return character is LF on all platforms.
///
/// @example EnforcedStyle: crlf
///   # The `crlf` style means that CR+LF (Carriage Return + Line Feed) is
///   # enforced on all platforms.
///   # bad
///   puts 'Hello' # Return character is LF on all platforms.
///   # good
///   puts 'Hello' # Return character is CR+LF on all platforms.
pub struct EndOfLine;

impl Cop for EndOfLine {
    fn name(&self) -> &'static str {
        "Layout/EndOfLine"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        if rubocop_skips_end_of_line(source, parse_result, config) {
            return;
        }

        check_end_of_line(self, source, config, diagnostics, corrections);
    }
}

fn check_end_of_line(
    cop: &EndOfLine,
    source: &SourceFile,
    config: &CopConfig,
    diagnostics: &mut Vec<Diagnostic>,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
) {
    let style = config.get_str("EnforcedStyle", "native");

    // RuboCop reports only 1 offense per file then breaks out of the loop.
    match style {
        "lf" | "native" => {
            // Flag lines ending with \r (i.e., CRLF or bare CR) — delete the \r
            let mut byte_offset: usize = 0;
            for (i, line) in source.lines().enumerate() {
                if line.ends_with(b"\r") {
                    let cr_offset = byte_offset + line.len() - 1;
                    let mut diag = cop.diagnostic(
                        source,
                        i + 1,
                        line.len() - 1,
                        "Carriage return character detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: cr_offset,
                            end: cr_offset + 1,
                            replacement: String::new(),
                            cop_name: cop.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                    break;
                }
                byte_offset += line.len() + 1; // +1 for \n
            }
        }
        "crlf" => {
            // Flag lines that do NOT end with \r (i.e., bare LF) — insert \r before \n
            let lines: Vec<&[u8]> = source.lines().collect();
            // If the last line has no \n at all, RuboCop considers CRLF requirement
            // satisfied (unimportant_missing_cr? returns true). We replicate this.
            let last_line_has_newline = if let Some(last) = lines.last() {
                last.ends_with(b"\n")
            } else {
                true // empty file, no offense
            };
            let mut byte_offset: usize = 0;
            for (i, line) in lines.iter().enumerate() {
                if i == lines.len() - 1 && (line.is_empty() || !last_line_has_newline) {
                    break;
                }
                if !line.ends_with(b"\r") {
                    let newline_offset = byte_offset + line.len(); // position of \n
                    let mut diag = cop.diagnostic(
                        source,
                        i + 1,
                        line.len(),
                        "Carriage return character missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: newline_offset,
                            end: newline_offset,
                            replacement: "\r".to_string(),
                            cop_name: cop.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                    break;
                }
                byte_offset += line.len() + 1;
            }
        }
        _ => {
            // Unknown style, fall back to native (LF) behavior
            let mut byte_offset: usize = 0;
            for (i, line) in source.lines().enumerate() {
                if line.ends_with(b"\r") {
                    let cr_offset = byte_offset + line.len() - 1;
                    let mut diag = cop.diagnostic(
                        source,
                        i + 1,
                        line.len() - 1,
                        "Carriage return character detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: cr_offset,
                            end: cr_offset + 1,
                            replacement: String::new(),
                            cop_name: cop.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                    break;
                }
                byte_offset += line.len() + 1;
            }
        }
    }
}

fn rubocop_skips_end_of_line(
    source: &SourceFile,
    parse_result: &ruby_prism::ParseResult<'_>,
    config: &CopConfig,
) -> bool {
    let has_structural_errors = parse_result
        .errors()
        .any(|err| !is_semantic_parse_error(err.message()));
    if has_structural_errors {
        return true;
    }

    if rubocop_skips_non_utf8_regex_escape_file(source) {
        return true;
    }

    target_ruby_version(config) >= 3.4
        && parse_result.errors().any(|err| {
            let message = err.message();
            message.starts_with("Invalid retry")
                || message.starts_with("Invalid return in class/module body")
        })
}

fn target_ruby_version(config: &CopConfig) -> f64 {
    config
        .options
        .get("TargetRubyVersion")
        .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
        .unwrap_or(2.7)
}

fn is_semantic_parse_error(message: &str) -> bool {
    message.starts_with("Invalid break")
        || message.starts_with("Invalid next")
        || message.starts_with("Invalid redo")
        || message.starts_with("Invalid retry")
        || message == "Invalid yield"
        || message.starts_with("Invalid return in class/module body")
}

fn rubocop_skips_non_utf8_regex_escape_file(source: &SourceFile) -> bool {
    let lines: Vec<&[u8]> = source.lines().collect();
    has_non_utf8_encoding_comment(&lines)
        && lines
            .iter()
            .copied()
            .any(line_contains_high_hex_escape_in_regex_literal)
}

fn has_non_utf8_encoding_comment(lines: &[&[u8]]) -> bool {
    let mut idx = 0;

    if idx < lines.len() && starts_with_shebang(lines[idx]) {
        idx += 1;
    }

    while idx < lines.len() && is_blank_line(lines[idx]) {
        idx += 1;
    }

    let Some(line) = lines.get(idx) else {
        return false;
    };
    if !is_encoding_comment(line) {
        return false;
    }

    let lower = normalized_ascii_string(line).to_ascii_lowercase();
    let Some(keyword_idx) = lower.find("encoding").or_else(|| lower.find("coding")) else {
        return false;
    };
    let after_keyword = &lower[keyword_idx..];
    let value = after_keyword
        .split_once([':', '='])
        .map(|(_, rhs)| rhs.trim_start())
        .unwrap_or("");
    let enc_name: String = value
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();

    if enc_name.is_empty() {
        return false;
    }

    !(enc_name == "utf"
        || enc_name == "utf8"
        || enc_name.starts_with("utf-8")
        || enc_name.starts_with("utf_8")
        || enc_name == "binary"
        || enc_name.starts_with("ascii-8bit")
        || enc_name.starts_with("ascii_8bit")
        || enc_name == "us-ascii"
        || enc_name == "ascii")
}

fn starts_with_shebang(line: &[u8]) -> bool {
    normalized_ascii_bytes(line).starts_with(b"#!")
}

fn is_blank_line(line: &[u8]) -> bool {
    first_non_padding_byte(line).is_none()
}

fn first_non_padding_byte(line: &[u8]) -> Option<u8> {
    line.iter()
        .copied()
        .filter(|&b| b != 0x00)
        .find(|&b| b != b' ' && b != b'\t' && b != b'\r')
}

fn is_encoding_comment(line: &[u8]) -> bool {
    let s = normalized_ascii_string(line);
    let trimmed = s.trim_start_matches([' ', '\t']);
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("# encoding:") || lower.starts_with("# coding:") {
        return true;
    }
    if lower.starts_with("# -*-") {
        return lower.contains("encoding") || lower.contains("coding");
    }
    false
}

fn normalized_ascii_string(line: &[u8]) -> String {
    String::from_utf8(normalized_ascii_bytes(line)).expect("ASCII normalization must stay ASCII")
}

fn normalized_ascii_bytes(line: &[u8]) -> Vec<u8> {
    line.iter()
        .copied()
        .filter(|&b| b != 0x00 && b.is_ascii())
        .collect()
}

fn line_contains_high_hex_escape_in_regex_literal(line: &[u8]) -> bool {
    let mut start = 0;

    while start < line.len() {
        let Some(open_idx) = line[start..].iter().position(|&byte| byte == b'/') else {
            return false;
        };
        let slash_idx = start + open_idx;
        if !looks_like_regex_open(line, slash_idx) {
            start = slash_idx + 1;
            continue;
        }

        let body_start = slash_idx + 1;
        let mut idx = body_start;
        let mut escaped = false;

        while idx < line.len() {
            let byte = line[idx];

            if escaped {
                escaped = false;
                idx += 1;
                continue;
            }

            if byte == b'\\' {
                escaped = true;
                idx += 1;
                continue;
            }

            if byte == b'/' {
                if contains_non_utf8_hex_escape(&line[body_start..idx]) {
                    return true;
                }
                start = idx + 1;
                break;
            }

            idx += 1;
        }

        if idx >= line.len() {
            return false;
        }
    }

    false
}

fn looks_like_regex_open(line: &[u8], slash_idx: usize) -> bool {
    let prev = line[..slash_idx]
        .iter()
        .rfind(|&&byte| !byte.is_ascii_whitespace())
        .copied();

    !matches!(
        prev,
        Some(
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'_'
            | b')'
            | b']'
            | b'}'
            | b'"'
            | b'\''
            | b'/',
        )
    )
}

fn contains_non_utf8_hex_escape(body: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < body.len() {
        if body[i] == b'\\' && body[i + 1] == b'x' {
            let (d1, d2) = (body[i + 2], body[i + 3]);
            if d1.is_ascii_hexdigit() && d2.is_ascii_hexdigit() {
                let byte = hex_pair_to_byte(d1, d2);
                if byte >= 0x80 {
                    let mut bytes = vec![byte];
                    let mut j = i + 4;
                    while j + 3 < body.len()
                        && body[j] == b'\\'
                        && body[j + 1] == b'x'
                        && body[j + 2].is_ascii_hexdigit()
                        && body[j + 3].is_ascii_hexdigit()
                    {
                        let next = hex_pair_to_byte(body[j + 2], body[j + 3]);
                        if next >= 0x80 {
                            bytes.push(next);
                            j += 4;
                        } else {
                            break;
                        }
                    }
                    if std::str::from_utf8(&bytes).is_err() {
                        return true;
                    }
                    i = j;
                    continue;
                }
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    false
}

fn hex_pair_to_byte(h1: u8, h2: u8) -> u8 {
    hex_digit_val(h1) * 16 + hex_digit_val(h2)
}

fn hex_digit_val(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn run_with_config(source: &[u8], config: CopConfig) -> Vec<Diagnostic> {
        crate::testutil::run_cop_full_with_config(&EndOfLine, source, config)
    }

    fn run(source: &[u8]) -> Vec<Diagnostic> {
        run_with_config(source, CopConfig::default())
    }

    crate::cop_scenario_fixture_tests!(
        EndOfLine,
        "cops/layout/end_of_line",
        single_crlf = "single_crlf.rb",
        assignment_crlf = "assignment_crlf.rb",
        method_call_crlf = "method_call_crlf.rb",
    );
    crate::cop_variant_fixture_tests!(EndOfLine, "cops/layout/end_of_line", crlf);
    crate::cop_autocorrect_fixture_tests!(EndOfLine, "cops/layout/end_of_line");

    #[test]
    fn crlf_detected() {
        // RuboCop reports only 1 offense per file, then breaks
        let diags = run(b"x = 1\r\ny = 2\r\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 5);
        assert_eq!(diags[0].message, "Carriage return character detected.");
    }

    #[test]
    fn lf_only_no_offense() {
        let diags = run(b"x = 1\ny = 2\n");
        assert!(diags.is_empty());
    }

    #[test]
    fn mixed_line_endings() {
        let diags = run(b"x = 1\r\ny = 2\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn cr_only_at_end() {
        let diags = run(b"x = 1\r");
        // No \n split, so entire content is one line ending with \r
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 5);
    }

    #[test]
    fn crlf_style_accepts_crlf() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("crlf".into()),
            )]),
            ..CopConfig::default()
        };
        let diags = run_with_config(b"x = 1\r\ny = 2\r\n", config);
        assert!(
            diags.is_empty(),
            "crlf style should accept CRLF line endings"
        );
    }

    #[test]
    fn autocorrect_remove_cr() {
        // Only 1 correction (first CRLF line) since cop breaks after first offense
        let input = b"x = 1\r\ny = 2\r\n";
        let (_diags, corrections) = crate::testutil::run_cop_autocorrect(&EndOfLine, input);
        assert_eq!(corrections.len(), 1);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = 1\ny = 2\r\n");
    }

    #[test]
    fn autocorrect_insert_cr_crlf_style() {
        // Only 1 correction (first LF-only line) since cop breaks after first offense
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("crlf".into()),
            )]),
            ..CopConfig::default()
        };
        let input = b"x = 1\ny = 2\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect_with_config(&EndOfLine, input, config);
        assert_eq!(corrections.len(), 1);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = 1\r\ny = 2\n");
    }

    #[test]
    fn crlf_style_flags_lf() {
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("crlf".into()),
            )]),
            ..CopConfig::default()
        };
        let diags = run_with_config(b"x = 1\ny = 2\n", config);
        assert_eq!(diags.len(), 1, "crlf style should flag first LF-only line");
        assert_eq!(diags[0].message, "Carriage return character missing.");
    }

    #[test]
    fn crlf_style_skips_invalid_retry_with_target_ruby_4() {
        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("crlf".into()),
                ),
                (
                    "TargetRubyVersion".into(),
                    serde_yml::Value::Number(serde_yml::Number::from(4)),
                ),
            ]),
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_internal(
            &EndOfLine,
            b"#~# ORIGINAL retry\n\nretry\n\n#~# EXPECTED\nretry\n",
            config,
            "spec/lib/rufo/formatter_source_specs/retry.rb.spec",
        );
        assert!(
            diags.is_empty(),
            "TargetRubyVersion 4.0 should suppress Layout/EndOfLine on invalid retry files: {:?}",
            diags
        );
    }
}
