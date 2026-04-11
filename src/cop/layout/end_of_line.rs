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
/// ## Non-UTF-8 files
///
/// Files with non-UTF-8 bytes (e.g., EUC-JP or Shift-JIS encoded content) cause
/// fatal `Lint/Syntax` errors in RuboCop ("Invalid byte sequence in utf-8."),
/// which prevents all other cops — including EndOfLine — from running. Prism
/// handles these files via encoding magic comments and parses them successfully,
/// so nitrocop explicitly skips the EndOfLine cop on non-UTF-8 files to match
/// RuboCop's behavior.
///
/// One remaining edge case: `test_windows_1252.rb` in the jruby corpus is pure
/// ASCII but declares `# encoding:windows-1252` and contains regex patterns
/// (`\xdf`) that cause fatal Parser gem errors but not Prism errors. This file
/// cannot be detected by a UTF-8 validity check alone.
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

    fn check_lines(
        &self,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Files with non-UTF-8 bytes cause fatal Lint/Syntax errors in RuboCop
        // ("Invalid byte sequence in utf-8."), which prevents all other cops
        // (including EndOfLine) from running. Prism handles these files via
        // encoding magic comments, so nitrocop must explicitly skip them to
        // avoid false positives.
        if std::str::from_utf8(source.as_bytes()).is_err() {
            return;
        }

        let style = config.get_str("EnforcedStyle", "native");
        let _bytes = source.as_bytes();

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
    fn crlf_style_skips_non_utf8_files() {
        // Files with non-UTF-8 bytes cause fatal Lint/Syntax errors in RuboCop,
        // preventing EndOfLine from running. Prism handles these files via
        // encoding magic comments, so nitrocop must explicitly skip them.
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("crlf".into()),
            )]),
            ..CopConfig::default()
        };
        // Simulate a file with EUC-JP encoding comment and non-UTF-8 bytes
        let mut content = b"# encoding: euc-jp\nx = 1\n".to_vec();
        content.extend_from_slice(&[0xA3, 0xE1]); // non-UTF-8 EUC-JP bytes
        content.push(b'\n');
        let source = SourceFile::from_bytes("test.rb", content);
        let mut diags = Vec::new();
        EndOfLine.check_lines(&source, &config, &mut diags, None);
        assert!(
            diags.is_empty(),
            "crlf style should skip files with non-UTF-8 bytes (RuboCop reports fatal Lint/Syntax instead)"
        );
    }
}
