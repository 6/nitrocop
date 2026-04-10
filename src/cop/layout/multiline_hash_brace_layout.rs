use crate::cop::shared::node_type::{HASH_NODE, KEYWORD_HASH_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

use super::multiline_literal_brace_layout::{self, BracePositions, HASH_BRACE};

/// Layout/MultilineHashBraceLayout
///
/// ## Corpus investigation (2026-03-10)
///
/// Corpus oracle reported FP=0, FN=2.
///
/// FP=0: no corpus false positives are currently known.
///
/// FN=2:
/// - `elastic/elasticsearch-ruby`: the outer hash had a heredoc in an earlier
///   element, but the last element was a normal hash pair. RuboCop still checks
///   brace layout there; only a heredoc in the last element forces the closing
///   brace placement. Fixed by narrowing the heredoc skip to the last element.
/// - `peritor/webistrano`: the remaining FN is a commented-out snippet that has
///   not reproduced locally as a normal AST-based offense. Leave it for future
///   investigation if it persists after the next corpus oracle run.
///
/// ## Variant style divergence (`EnforcedStyle: same_line`, 2026-04-09)
///
/// FP clusters came from two narrow mismatches with RuboCop:
/// 1. The cop only used a shallow `contains_heredoc(last_elem)` check, so it
///    missed heredocs nested inside the last element's array/hash value and
///    flagged closing braces that RuboCop skips.
/// 2. RuboCop treats legacy `# rubocop:disable Layout:LineLength` comments as a
///    department-level `Layout` disable. nitrocop's shared directive parser
///    does not recognize that single-colon form yet, so this cop mirrors the
///    suppression locally to avoid `same_line`-only false positives.
pub struct MultilineHashBraceLayout;

impl Cop for MultilineHashBraceLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineHashBraceLayout"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[HASH_NODE, KEYWORD_HASH_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "symmetrical");

        // KeywordHashNode (keyword args `foo(a: 1)`) has no braces — skip
        if node.as_keyword_hash_node().is_some() {
            return;
        }

        let hash = match node.as_hash_node() {
            Some(h) => h,
            None => return,
        };

        let opening = hash.opening_loc();
        let closing = hash.closing_loc();

        // Only check brace hashes
        if opening.as_slice() != b"{" || closing.as_slice() != b"}" {
            return;
        }

        let elements = hash.elements();
        if elements.is_empty() {
            return;
        }

        let last_elem = elements.iter().last().unwrap();

        // Only the last element can force the closing brace to move because of
        // its heredoc terminator. Earlier heredocs do not exempt the hash.
        if multiline_literal_brace_layout::last_line_heredoc(source, &last_elem) {
            return;
        }

        let (open_line, _) = source.offset_to_line_col(opening.start_offset());
        let (close_line, close_col) = source.offset_to_line_col(closing.start_offset());

        if legacy_layout_department_disabled_at_line(source, parse_result, close_line) {
            return;
        }

        let first_elem = elements.iter().next().unwrap();
        let (first_elem_line, _) = source.offset_to_line_col(first_elem.location().start_offset());
        let (last_elem_line, _) =
            source.offset_to_line_col(last_elem.location().end_offset().saturating_sub(1));

        multiline_literal_brace_layout::check_brace_layout(
            self,
            source,
            enforced_style,
            &HASH_BRACE,
            &BracePositions {
                open_line,
                close_line,
                close_col,
                first_elem_line,
                last_elem_line,
            },
            diagnostics,
        );
    }
}

fn legacy_layout_department_disabled_at_line(
    source: &SourceFile,
    parse_result: &ruby_prism::ParseResult<'_>,
    line: usize,
) -> bool {
    let lines: Vec<&[u8]> = source.lines().collect();
    let mut layout_disabled = false;

    for comment in parse_result.comments() {
        let Some(action) = legacy_layout_directive_action(source, &lines, comment, line) else {
            continue;
        };

        match action {
            LegacyDirectiveAction::DisableInline => return true,
            LegacyDirectiveAction::DisableBlock => layout_disabled = true,
            LegacyDirectiveAction::EnableBlock => layout_disabled = false,
        }
    }

    layout_disabled
}

#[derive(Clone, Copy)]
enum LegacyDirectiveAction {
    DisableInline,
    DisableBlock,
    EnableBlock,
}

fn legacy_layout_directive_action(
    source: &SourceFile,
    lines: &[&[u8]],
    comment: ruby_prism::Comment<'_>,
    offense_line: usize,
) -> Option<LegacyDirectiveAction> {
    let loc = comment.location();
    let comment_bytes = &source.as_bytes()[loc.start_offset()..loc.end_offset()];
    let comment_str = std::str::from_utf8(comment_bytes).ok()?;
    let (comment_line, col) = source.offset_to_line_col(loc.start_offset());
    if comment_line > offense_line {
        return None;
    }

    let is_inline = if comment_line >= 1 && comment_line <= lines.len() {
        let line_bytes = lines[comment_line - 1];
        let before_comment = &line_bytes[..col.min(line_bytes.len())];
        before_comment.iter().any(|b| !b.is_ascii_whitespace())
    } else {
        false
    };

    let marker = find_legacy_directive_start(comment_str)?;
    if marker > 0 && !is_inline {
        let prefix = &comment_str[..marker];
        if prefix.bytes().all(|b| b == b'#' || b == b' ' || b == b'\t') {
            return None;
        }
    }

    let directive = comment_str[marker + 1..].trim_start();
    let (action, rest) = if let Some(rest) = directive.strip_prefix("rubocop:") {
        parse_legacy_layout_directive(rest)?
    } else if let Some(rest) = directive.strip_prefix("nitrocop:") {
        parse_legacy_layout_directive(rest)?
    } else {
        return None;
    };

    if !legacy_layout_token_present(rest) {
        return None;
    }

    match action {
        "disable" | "todo" => {
            if is_inline && comment_line == offense_line {
                Some(LegacyDirectiveAction::DisableInline)
            } else if !is_inline {
                Some(LegacyDirectiveAction::DisableBlock)
            } else {
                None
            }
        }
        "enable" if !is_inline => Some(LegacyDirectiveAction::EnableBlock),
        _ => None,
    }
}

fn parse_legacy_layout_directive(rest: &str) -> Option<(&str, &str)> {
    let trimmed = rest.trim_start();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let action = parts.next()?;
    let remainder = parts.next()?.trim_start();
    Some((action, remainder))
}

fn find_legacy_directive_start(comment: &str) -> Option<usize> {
    for (idx, _) in comment.match_indices('#') {
        let after_hash = comment[idx + 1..].trim_start();
        if after_hash.starts_with("rubocop:") || after_hash.starts_with("nitrocop:") {
            return Some(idx);
        }
    }
    None
}

fn legacy_layout_token_present(rest: &str) -> bool {
    let rest = match rest.find("--") {
        Some(idx) => &rest[..idx],
        None => rest,
    };

    rest.split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| match token.find(' ') {
            Some(idx) => &token[..idx],
            None => token,
        })
        .map(|token| match token.find('(') {
            Some(idx) => &token[..idx],
            None => token,
        })
        .map(|token| {
            token.trim_end_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '_' && c != '/' && c != ':'
            })
        })
        .any(|token| token.starts_with("Layout:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        MultilineHashBraceLayout,
        "cops/layout/multiline_hash_brace_layout"
    );

    fn style_config(style: &str) -> CopConfig {
        let mut opts = std::collections::HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        CopConfig {
            options: opts,
            ..CopConfig::default()
        }
    }

    #[test]
    fn same_line_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &MultilineHashBraceLayout,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/multiline_hash_brace_layout/same_line_offense.rb"
            ),
            style_config("same_line"),
        );
    }

    #[test]
    fn same_line_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &MultilineHashBraceLayout,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/multiline_hash_brace_layout/same_line_no_offense.rb"
            ),
            style_config("same_line"),
        );
    }

    #[test]
    fn earlier_heredoc_still_checks_closing_brace() {
        let source = br#"config = { subject: <<~BODY,
             body line
           BODY
           attachment: "report.yml"
}
"#;
        let diagnostics = run_cop_full(&MultilineHashBraceLayout, source);
        assert_eq!(
            diagnostics.len(),
            1,
            "Expected one offense: {diagnostics:?}"
        );
    }
}
