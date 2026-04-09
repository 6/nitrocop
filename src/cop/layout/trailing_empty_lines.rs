use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-10)
///
/// CI baseline reported FP=47, FN=22.
///
/// Fixed sampled FN: whitespace-only lines at EOF were previously excluded
/// from the trailing-blank-line scan, so the default `final_newline` style
/// could miss `Trailing blank line detected.` and sometimes fall through to
/// `Final newline missing.` instead. The accepted fix treats space/tab/CR-only
/// lines as blank and autocorrect removes the full blank tail.
///
/// Acceptance gate after this patch (`scripts/check-cop.py --verbose --rerun`):
/// expected=9,691, actual=9,587, CI baseline=9,716, excess=0, missing=104,
/// file-drop noise=308.
///
/// Remaining gap: 104 potential FN remain. This batch only addressed the
/// whitespace-only trailing-line case; no broader config-resolution issue was
/// involved.
///
/// ## Corpus investigation (2026-03-14)
///
/// FP=62, FN=6. Two missing exemptions from RuboCop:
/// 1. Files containing `__END__` anywhere are skipped by RuboCop (regex
///    `/\s*__END__/`). Nitrocop was flagging trailing blank lines after
///    `__END__`, causing false positives.
/// 2. Files ending with `"%\n\n"` (percent blank string edge case) are
///    skipped by RuboCop. Nitrocop was flagging these as trailing blanks.
///
/// Both exemptions now implemented.
///
/// ## Corpus investigation (2026-03-14, second pass)
///
/// FP=46, FN=6. Rewrote detection to match RuboCop's exact algorithm:
/// RuboCop captures trailing whitespace with `/\s*\Z/`, counts newlines
/// in that match, and computes `blank_lines = newline_count - 1`. The old
/// nitrocop approach used line-by-line `is_blank_line()` which incorrectly
/// treated whitespace-only content after the final newline (e.g., `"code\n  "`)
/// as a trailing blank line. RuboCop does NOT flag such files because the
/// trailing whitespace contains only 1 newline, yielding blank_lines=0.
///
/// Also fixed message format: RuboCop reports "N trailing blank lines
/// detected." (with count) when N > 1, not the generic singular message.
///
/// ## Corpus investigation (2026-03-15)
///
/// FP=2, FN=1. No example locations available from corpus oracle.
/// Compared nitrocop vs vendor RuboCop source. Found one concrete
/// difference: `trailing_whitespace_info` only matched `[ \t\r\n]` as
/// whitespace, but RuboCop's `/\s*\Z/` regex also matches `\x0b`
/// (vertical tab) and `\x0c` (form feed). Files with these rare chars
/// in trailing whitespace would compute different `newline_count` values,
/// causing FP (nitrocop stops scanning at the char, sees fewer trailing
/// newlines than RuboCop, may report "Final newline missing" when RuboCop
/// reports "Trailing blank line") or FN (nitrocop undercounts blank lines).
/// Fixed by adding `\x0b` and `\x0c` to the whitespace match set.
///
/// Remaining known location gap: CRLF files with trailing blank lines were
/// reported on the code line's `\r` byte rather than the first trailing blank
/// line. RuboCop reports the first blank line. Fixed by advancing past the
/// terminating line ending of the last code line before converting the report
/// offset to line/column.
///
/// Remaining CI FP on `simplecov` was another already-known encoding case:
/// non-UTF-8 source without a magic `coding:`/`encoding:` comment. RuboCop only
/// reports the encoding syntax error there, so line-based cops should skip the
/// file entirely. Fixed by reusing the same guard pattern already applied to
/// `Layout/LeadingEmptyLines` and `Naming/FileName`.
///
/// ## Corpus investigation (2026-04-09)
///
/// Variant gate for `EnforcedStyle: final_blank_line` reported FP=31, FN=30
/// with paired locations on adjacent lines (for example, RuboCop at `:9`
/// vs nitrocop at `:10`). The offense itself was correct; the divergence was
/// only the reported range when a file ended with exactly one newline and the
/// last code line also had trailing whitespace, e.g. `"...end \n"` or
/// `"...end \r\n"`.
///
/// RuboCop reports `Trailing blank line missing.` on the last real line when
/// the trailing whitespace starts with spaces/tabs before the final line ending,
/// but still reports on the phantom next line for a pure final `\n` or `\r\n`.
/// Nitrocop was sending every `blank_lines == 0` case to the phantom line.
pub struct TrailingEmptyLines;

/// Check if the source contains `__END__` (with optional leading whitespace).
/// Matches RuboCop's `/\s*__END__/` regex which matches anywhere in the file.
fn contains_end_marker(bytes: &[u8]) -> bool {
    // Scan for __END__ preceded only by whitespace on each line
    // The RuboCop regex /\s*__END__/ matches __END__ anywhere in the source
    // with optional leading whitespace characters
    bytes.windows(7).any(|w| w == b"__END__")
}

/// Count trailing whitespace length and newlines, matching RuboCop's
/// `buffer.source[/\s*\Z/]` approach. Returns (trailing_ws_len, newline_count).
fn trailing_whitespace_info(bytes: &[u8]) -> (usize, usize) {
    let mut ws_len = 0;
    let mut newline_count = 0;
    for &b in bytes.iter().rev() {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c') {
            ws_len += 1;
            if b == b'\n' {
                newline_count += 1;
            }
        } else {
            break;
        }
    }
    (ws_len, newline_count)
}

fn first_trailing_blank_line_offset(bytes: &[u8], trailing_start: usize) -> usize {
    let mut offset = trailing_start;
    if bytes.get(offset) == Some(&b'\r') {
        offset += 1;
    }
    if bytes.get(offset) == Some(&b'\n') {
        offset += 1;
    }
    offset.min(bytes.len().saturating_sub(1))
}

fn missing_blank_line_report_location(
    source: &SourceFile,
    bytes: &[u8],
    trailing_start: usize,
) -> (usize, usize) {
    if matches!(bytes.get(trailing_start), Some(b'\n' | b'\r')) {
        let (last_line, _) = source.offset_to_line_col(trailing_start);
        (last_line + 1, 0)
    } else {
        source.offset_to_line_col(trailing_start + 1)
    }
}

fn skip_for_invalid_utf8_without_magic_encoding(source: &SourceFile) -> bool {
    if std::str::from_utf8(source.as_bytes()).is_ok() {
        return false;
    }
    !has_magic_encoding_comment(source.as_bytes())
}

fn has_magic_encoding_comment(source: &[u8]) -> bool {
    for line in source.split(|&b| b == b'\n').take(2) {
        let lower = String::from_utf8_lossy(line).to_ascii_lowercase();
        if lower.contains("coding:") || lower.contains("coding=") {
            return true;
        }
        if lower.contains("encoding:") || lower.contains("encoding=") {
            return true;
        }
    }
    false
}

impl Cop for TrailingEmptyLines {
    fn name(&self) -> &'static str {
        "Layout/TrailingEmptyLines"
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
        if skip_for_invalid_utf8_without_magic_encoding(source) {
            return;
        }

        let style = config.get_str("EnforcedStyle", "final_newline");
        let bytes = source.as_bytes();
        if bytes.is_empty() {
            return;
        }

        // RuboCop skips files containing __END__ (with optional leading whitespace)
        if contains_end_marker(bytes) {
            return;
        }

        // RuboCop skips files ending with "%\n\n" (percent blank string edge case)
        if bytes.ends_with(b"%\n\n") {
            return;
        }
        // Match RuboCop's approach: capture trailing whitespace, count newlines
        // blank_lines = newline_count - 1
        // wanted_blank_lines = 0 for final_newline, 1 for final_blank_line
        let (ws_len, newline_count) = trailing_whitespace_info(bytes);
        let blank_lines = newline_count as isize - 1;
        let wanted_blank_lines: isize = if style == "final_blank_line" { 1 } else { 0 };

        if blank_lines == wanted_blank_lines {
            return;
        }

        // Determine offense location and message
        let message = match blank_lines {
            -1 => "Final newline missing.".to_string(),
            0 => "Trailing blank line missing.".to_string(),
            1 => "Trailing blank line detected.".to_string(),
            n => {
                if wanted_blank_lines == 0 {
                    format!("{n} trailing blank lines detected.")
                } else {
                    format!("{n} trailing blank lines instead of {wanted_blank_lines} detected.")
                }
            }
        };

        // Calculate report position: RuboCop reports at begin_pos+1 (unless
        // trailing whitespace is empty), which is the first byte after the
        // last non-whitespace content.
        let begin_pos = bytes.len() - ws_len;
        let (report_line, report_col) = if blank_lines == 0 {
            // RuboCop reports on the phantom line for a pure trailing line
            // ending, but on the last real line if spaces/tabs precede that
            // line ending (for example, `end \n`).
            missing_blank_line_report_location(source, bytes, begin_pos)
        } else if blank_lines > 0 {
            source.offset_to_line_col(first_trailing_blank_line_offset(bytes, begin_pos))
        } else if ws_len > 0 {
            source.offset_to_line_col(begin_pos + 1)
        } else {
            source.offset_to_line_col(begin_pos)
        };

        let mut diag = self.diagnostic(source, report_line, report_col, message);

        if let Some(ref mut corr) = corrections {
            let replacement = if style == "final_blank_line" {
                "\n\n".to_string()
            } else {
                "\n".to_string()
            };
            corr.push(crate::correction::Correction {
                start: begin_pos,
                end: bytes.len(),
                replacement,
                cop_name: self.name(),
                cop_index: 0,
            });
            diag.corrected = true;
        }
        diagnostics.push(diag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::source::SourceFile;

    crate::cop_scenario_fixture_tests!(
        TrailingEmptyLines,
        "cops/layout/trailing_empty_lines",
        missing_newline = "missing_newline.rb",
        trailing_blank = "trailing_blank.rb",
        multiple_trailing = "multiple_trailing.rb",
        whitespace_trailing = "whitespace_trailing.rb",
    );

    #[test]
    fn missing_final_newline() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].message, "Final newline missing.");
    }

    #[test]
    fn missing_final_newline_multiline() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\ny = 2".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(diags[0].message, "Final newline missing.");
    }

    #[test]
    fn trailing_blank_line() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(diags[0].message, "Trailing blank line detected.");
    }

    #[test]
    fn trailing_blank_lines_with_crlf_report_first_blank_line() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\r\n\r\n\r\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(diags[0].location.column, 0);
        assert_eq!(diags[0].message, "2 trailing blank lines detected.");
    }

    #[test]
    fn ignores_non_utf8_source_without_magic_encoding() {
        let source =
            SourceFile::from_bytes("test.rb", b"\n# localized to Espa\xf1ol thus:\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "non-UTF8 source without magic encoding should be ignored"
        );
    }

    #[test]
    fn non_utf8_source_with_magic_encoding_still_registers_offense() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"\n# encoding: iso-8859-1\n# localized to Espa\xf1ol thus:\n\n".to_vec(),
        );
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 4);
        assert_eq!(diags[0].message, "Trailing blank line detected.");
    }

    #[test]
    fn multiple_trailing_blank_lines() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\n\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(diags[0].message, "3 trailing blank lines detected.");
    }

    #[test]
    fn proper_final_newline() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn multiline_proper() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\ny = 2\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn empty_file() {
        let source = SourceFile::from_bytes("test.rb", b"".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn final_blank_line_style_accepts_trailing_blank() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &config, &mut diags, None);
        assert!(
            diags.is_empty(),
            "final_blank_line style should accept trailing blank line"
        );
    }

    #[test]
    fn final_blank_line_style_flags_missing_blank() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Trailing blank line missing.");
        // Pure trailing "\n" still reports on the phantom line.
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(diags[0].location.column, 0);
    }

    #[test]
    fn final_blank_line_style_flags_missing_blank_with_trailing_space() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("test.rb", b"module A\nend \n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Trailing blank line missing.");
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(diags[0].location.column, 4);
    }

    #[test]
    fn final_blank_line_style_flags_missing_blank_multiline() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        // 5-line file ending with \n — should report at line 6
        let source = SourceFile::from_bytes(
            "test.rb",
            b"source 'http://rubygems.org'\nruby \">= 2.7\"\ngem 'sinatra'\ngem 'githubchart'\ngem 'webrick'\n".to_vec(),
        );
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Trailing blank line missing.");
        assert_eq!(diags[0].location.line, 6);
        assert_eq!(diags[0].location.column, 0);
    }

    #[test]
    fn final_blank_line_style_no_offense_with_blank() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        // File ends with blank line — no offense
        let source = SourceFile::from_bytes("test.rb", b"x = 1\ny = 2\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &config, &mut diags, None);
        assert!(
            diags.is_empty(),
            "final_blank_line style should accept trailing blank line"
        );
    }

    #[test]
    fn final_blank_line_style_too_many_blanks() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &config, &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].message,
            "2 trailing blank lines instead of 1 detected."
        );
    }

    #[test]
    fn skip_file_with_end_marker() {
        // RuboCop skips files containing __END__ anywhere
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n__END__\nsome data\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(diags.is_empty(), "should skip files containing __END__");
    }

    #[test]
    fn skip_file_with_end_marker_leading_whitespace() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n  __END__\nsome data\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "should skip files with __END__ preceded by whitespace"
        );
    }

    #[test]
    fn skip_file_ending_with_percent_blank_string() {
        // RuboCop skips files ending with "%\n\n"
        let source = SourceFile::from_bytes("test.rb", b"x = \"%\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "should skip files ending with percent blank string"
        );
    }

    #[test]
    fn whitespace_after_final_newline_no_offense() {
        // File ends with "x = 1\n  " (trailing spaces after newline, no second newline)
        // RuboCop does NOT flag this: trailing whitespace regex counts 1 newline,
        // blank_lines = 0 = wanted, so no offense.
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n  ".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "should not flag trailing whitespace after final newline: got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn whitespace_after_final_newline_tabs_no_offense() {
        // Same as above but with tabs
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\t\t".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert!(
            diags.is_empty(),
            "should not flag trailing tabs after final newline: got {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn whitespace_newline_whitespace_flags_offense() {
        // File "x = 1\n  \n  " - has a blank line (with whitespace) followed by
        // more whitespace. RuboCop counts 2 newlines, blank_lines = 1, flags it.
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n  \n  ".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Trailing blank line detected.");
    }

    #[test]
    fn multiple_trailing_message_format() {
        // RuboCop reports "N trailing blank lines detected." for N > 1
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\n\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "3 trailing blank lines detected.");
    }

    #[test]
    fn two_trailing_blank_lines_message() {
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\n\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "2 trailing blank lines detected.");
    }

    #[test]
    fn form_feed_between_newlines_counts_as_trailing_blank() {
        // File "x = 1\n\x0c\n" — RuboCop's \s*\Z counts \x0c as whitespace,
        // so trailing ws = "\n\x0c\n", newline_count = 2, blank_lines = 1.
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\x0c\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "form feed between newlines should count as trailing blank"
        );
        assert!(
            diags[0].message.contains("railing blank line"),
            "got: {}",
            diags[0].message
        );
    }

    #[test]
    fn vertical_tab_between_newlines_counts_as_trailing_blank() {
        // File "x = 1\n\x0b\n" — RuboCop's \s counts \x0b as whitespace
        let source = SourceFile::from_bytes("test.rb", b"x = 1\n\x0b\n".to_vec());
        let mut diags = Vec::new();
        TrailingEmptyLines.check_lines(&source, &CopConfig::default(), &mut diags, None);
        assert_eq!(
            diags.len(),
            1,
            "vertical tab between newlines should count as trailing blank"
        );
        assert!(
            diags[0].message.contains("railing blank line"),
            "got: {}",
            diags[0].message
        );
    }

    #[test]
    fn autocorrect_missing_newline() {
        let input = b"x = 1";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect(&TrailingEmptyLines, input);
        assert!(!corrections.is_empty());
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = 1\n");
    }

    #[test]
    fn autocorrect_trailing_blank() {
        let input = b"x = 1\n\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect(&TrailingEmptyLines, input);
        assert!(!corrections.is_empty());
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = 1\n");
    }

    #[test]
    fn autocorrect_multiple_trailing() {
        let input = b"x = 1\n\n\n\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect(&TrailingEmptyLines, input);
        assert!(!corrections.is_empty());
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = 1\n");
    }

    #[test]
    fn autocorrect_final_blank_line_style_missing() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("final_blank_line".into()),
            )]),
            ..CopConfig::default()
        };
        let input = b"x = 1\n";
        let (_diags, corrections) =
            crate::testutil::run_cop_autocorrect_with_config(&TrailingEmptyLines, input, config);
        assert!(!corrections.is_empty());
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"x = 1\n\n");
    }
}
