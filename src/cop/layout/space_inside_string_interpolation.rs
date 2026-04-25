use crate::cop::shared::node_type::EMBEDDED_STATEMENTS_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Matches RuboCop's continued-string quirk for `"... \\\n#{expr}"`.
///
/// When an interpolation starts immediately after a backslash-newline string
/// continuation, RuboCop skips `Layout/SpaceInsideStringInterpolation` in both
/// styles. That only applies to an odd-length run of backslashes before the
/// newline. An even run, such as a `%(` body line ending in `\\` immediately
/// before `#{task_completions}`, is a literal trailing backslash, and RuboCop
/// still enforces `EnforcedStyle: space` there. Count the backslash run so the
/// continuation guard does not hide that variant offense.
pub struct SpaceInsideStringInterpolation;

impl Cop for SpaceInsideStringInterpolation {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideStringInterpolation"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[EMBEDDED_STATEMENTS_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "no_space");
        let require_space = style == "space";

        // EmbeddedStatementsNode represents `#{ ... }` inside strings
        let embedded = match node.as_embedded_statements_node() {
            Some(e) => e,
            None => return,
        };

        let open_loc = embedded.opening_loc();
        let close_loc = embedded.closing_loc();

        let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
        let (close_line, _) = source.offset_to_line_col(close_loc.start_offset());

        // Skip multiline interpolations
        if open_line != close_line {
            return;
        }

        let bytes = source.as_bytes();
        let open_end = open_loc.end_offset(); // position after `#{`
        let close_start = close_loc.start_offset(); // position of `}`

        // Skip empty interpolation
        if close_start <= open_end {
            return;
        }

        if starts_after_string_line_continuation(bytes, open_loc.start_offset()) {
            return;
        }

        let space_after_open = bytes.get(open_end) == Some(&b' ');
        let space_before_close = close_start > 0 && bytes.get(close_start - 1) == Some(&b' ');

        if require_space {
            // Require spaces
            if !space_after_open {
                let (line, col) = source.offset_to_line_col(open_end);
                let mut diag = self.diagnostic(
                    source,
                    line,
                    col,
                    "Missing space inside string interpolation.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: open_end,
                        end: open_end,
                        replacement: " ".to_string(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
                diagnostics.push(diag);
            }
            if !space_before_close {
                let (line, col) = source.offset_to_line_col(close_start);
                let mut diag = self.diagnostic(
                    source,
                    line,
                    col,
                    "Missing space inside string interpolation.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: close_start,
                        end: close_start,
                        replacement: " ".to_string(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
                diagnostics.push(diag);
            }
        } else {
            // "no_space" (default) — flag spaces
            if space_after_open {
                let (line, col) = source.offset_to_line_col(open_end);
                let mut diag = self.diagnostic(
                    source,
                    line,
                    col,
                    "Space inside string interpolation detected.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: open_end,
                        end: open_end + 1,
                        replacement: String::new(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
                diagnostics.push(diag);
            }
            if space_before_close {
                let (line, col) = source.offset_to_line_col(close_start - 1);
                let mut diag = self.diagnostic(
                    source,
                    line,
                    col,
                    "Space inside string interpolation detected.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: close_start - 1,
                        end: close_start,
                        replacement: String::new(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
                diagnostics.push(diag);
            }
        }
    }
}

fn starts_after_string_line_continuation(bytes: &[u8], open_start: usize) -> bool {
    let Some(run_end) = backslash_run_end_before_newline(bytes, open_start) else {
        return false;
    };

    let mut count = 0;
    let mut pos = run_end + 1;
    while pos > 0 && bytes[pos - 1] == b'\\' {
        count += 1;
        pos -= 1;
    }

    count % 2 == 1
}

fn backslash_run_end_before_newline(bytes: &[u8], open_start: usize) -> Option<usize> {
    if open_start < 2 || bytes.get(open_start - 1) != Some(&b'\n') {
        return None;
    }

    if open_start >= 3 && bytes.get(open_start - 2) == Some(&b'\r') {
        Some(open_start - 3)
    } else {
        Some(open_start - 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceInsideStringInterpolation,
        "cops/layout/space_inside_string_interpolation"
    );
    crate::cop_variant_fixture_tests!(
        SpaceInsideStringInterpolation,
        "cops/layout/space_inside_string_interpolation",
        space
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideStringInterpolation,
        "cops/layout/space_inside_string_interpolation"
    );
}
