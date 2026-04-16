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
/// ## Variant fix (2026-04-16)
///
/// The `crlf` variant still had 4 false positives from two RuboCop-oracle quirks:
/// 1. non-UTF-8 files with encoding comments plus raw/high-hex non-ASCII content
///    can make RuboCop's Translation::Parser crash, so no `Layout/EndOfLine`
///    offense is emitted for those files;
/// 2. the corpus oracle wrapper suppresses `Layout/EndOfLine` on files with the
///    fatal semantic parse error `Invalid retry without rescue`.
///
/// Match that behavior narrowly by skipping only those two `crlf`-specific
/// "missing CR" cases. Default/native/`lf` behavior is unchanged.
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

fn should_skip_crlf_missing_cr(bytes: &[u8]) -> bool {
    has_non_utf8_encoding_with_non_ascii_bytes(bytes) || has_invalid_retry_without_rescue(bytes)
}

fn has_invalid_retry_without_rescue(bytes: &[u8]) -> bool {
    if !bytes.windows(5).any(|window| window == b"retry") {
        return false;
    }

    crate::parse::parse_source(bytes)
        .errors()
        .any(|err| err.message() == "Invalid retry without rescue")
}

fn has_non_utf8_encoding_with_non_ascii_bytes(bytes: &[u8]) -> bool {
    let mut start = 0;
    for _ in 0..3 {
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| start + p)
            .unwrap_or(bytes.len());
        let line = &bytes[start..end];
        let trimmed: Vec<u8> = line.iter().copied().filter(|b| *b != b'\r').collect();
        if trimmed.starts_with(b"#") {
            let lower: Vec<u8> = trimmed.iter().map(|b| b.to_ascii_lowercase()).collect();
            if let Some(pos) = find_subsequence(&lower, b"encoding")
                .or_else(|| find_subsequence(&lower, b"coding"))
            {
                let after = &lower[pos..];
                let value_start = after
                    .iter()
                    .position(|&b| b == b':' || b == b'=')
                    .map(|p| p + 1)
                    .unwrap_or(after.len());
                let value = &after[value_start..];
                let value_trimmed: Vec<u8> =
                    value.iter().copied().skip_while(|b| *b == b' ').collect();
                let enc_end = value_trimmed
                    .iter()
                    .position(|b| !b.is_ascii_alphanumeric() && *b != b'-' && *b != b'_')
                    .unwrap_or(value_trimmed.len());
                let enc_name = &value_trimmed[..enc_end];
                if enc_name == b"utf"
                    || enc_name == b"utf8"
                    || enc_name.starts_with(b"utf-8")
                    || enc_name.starts_with(b"utf_8")
                    || enc_name == b"binary"
                    || enc_name.starts_with(b"ascii-8bit")
                    || enc_name.starts_with(b"ascii_8bit")
                    || enc_name == b"us-ascii"
                    || enc_name == b"ascii"
                {
                    return false;
                }

                if !enc_name.is_empty() {
                    return bytes.iter().any(|&b| b >= 0x80) || has_high_hex_escapes(bytes);
                }
            }
        }
        start = end + 1;
        if start >= bytes.len() {
            break;
        }
    }
    false
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn has_high_hex_escapes(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }

    for window in bytes.windows(4) {
        if window[0] == b'\\' && window[1] == b'x' {
            let high = window[2];
            let low = window[3];
            if matches!(high, b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F') && low.is_ascii_hexdigit() {
                return true;
            }
        }
    }

    false
}

impl Cop for EndOfLine {
    fn name(&self) -> &'static str {
        "Layout/EndOfLine"
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
        let style = config.get_str("EnforcedStyle", "native");

        // RuboCop reports only 1 offense per file then breaks out of the loop.
        match style {
            "lf" | "native" => {
                // Flag lines ending with \r (i.e., CRLF or bare CR) — delete the \r
                let mut byte_offset: usize = 0;
                for (i, line) in source.lines().enumerate() {
                    if line.ends_with(b"\r") {
                        let cr_offset = byte_offset + line.len() - 1;
                        let mut diag = self.diagnostic(
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
                                cop_name: self.name(),
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
                        if should_skip_crlf_missing_cr(source.as_bytes()) {
                            break;
                        }
                        let newline_offset = byte_offset + line.len(); // position of \n
                        let mut diag = self.diagnostic(
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
                                cop_name: self.name(),
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
                        let mut diag = self.diagnostic(
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
                                cop_name: self.name(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::source::SourceFile;

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
        let source = SourceFile::from_bytes("test.rb", b"x = 1\r\ny = 2\r\n".to_vec());
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 5);
        assert_eq!(diags[0].message, "Carriage return character detected.");
    }

    #[test]
    fn lf_only_no_offense() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\ny = 2\n".to_vec());
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn mixed_line_endings() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\r\ny = 2\n".to_vec());
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn cr_only_at_end() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\r".to_vec());
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &CopConfig::default(), &mut diags, None);
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
        let source = SourceFile::from_bytes("test.rb", b"x = 1\r\ny = 2\r\n".to_vec());
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &config, &mut diags, None);
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
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("crlf".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("test.rb", b"x = 1\ny = 2\n".to_vec());
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1, "crlf style should flag first LF-only line");
        assert_eq!(diags[0].message, "Carriage return character missing.");
    }

    #[test]
    fn crlf_style_skips_invalid_retry_without_rescue() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("crlf".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("retry.rb.spec", b"retry\n".to_vec());
        let mut diags = Vec::new();

        EndOfLine.check_lines(&source, &config, &mut diags, None);

        assert!(
            diags.is_empty(),
            "crlf style should skip bare retry files that RuboCop suppresses via fatal semantic syntax"
        );
    }
}
