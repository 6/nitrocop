use crate::cop::shared::node_type::{CALL_NODE, CONSTANT_PATH_NODE, CONSTANT_READ_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Style/EmptyLiteral: prefers literal `[]`, `{}`, `''` over `Array.new`, `Hash.new`, `String.new`.
///
/// ## Corpus investigation (2026-03-11)
///
/// Corpus oracle reported FP=1, FN=0.
///
/// FP=1: `Hash.new { Hash.new }` was incorrectly flagging the inner `Hash.new`.
/// RuboCop skips a nested constructor when its parent block belongs to `Hash.new`
/// or `Array.new`, because the parser parentage is the surrounding block rather than
/// the constructor call itself.
///
/// ## Corpus investigation (2026-03-27)
///
/// Corpus oracle reported FP=0, FN=30. The remaining misses came from the nested
/// constructor exemption being too broad: it skipped any inner `Hash.new`/`Array.new`
/// anywhere inside an outer constructor block body.
///
/// RuboCop is narrower. It only skips the inner constructor when that constructor is
/// the direct body expression of an outer zero-argument constructor block:
/// `Hash.new { Hash.new }` and `Array.new { Array.new }` are accepted, but
/// `Hash.new { |h, k| h[k] = Hash.new }` and `Array.new(n) { Array.new }` are
/// offenses because the inner constructor is nested under another expression or
/// the outer constructor has arguments. Fixed by checking only the direct parent
/// chain (`CallNode -> StatementsNode? -> BlockNode -> outer constructor CallNode`)
/// instead of any location-contained ancestor block.
///
/// **String.new special case:** RuboCop only flags `String.new` when `frozen_string_literal: false`
/// is explicitly set. When the comment is absent or set to `true`, `String.new` is needed to
/// create a mutable empty string, so it is not flagged. Prior to this fix, we incorrectly
/// flagged `String.new` when the comment was absent (121 FPs in corpus).
pub struct EmptyLiteral;

/// Check if the source file has `# frozen_string_literal: false` in the first few lines.
/// Returns true only when explicitly set to `false`.
fn has_frozen_string_literal_false(source: &SourceFile) -> bool {
    for line in source.lines().take(3) {
        let lower: Vec<u8> = line.to_ascii_lowercase();
        if let Some(pos) = lower
            .windows(22)
            .position(|w| w == b"frozen_string_literal:")
        {
            let after = &lower[pos + 22..];
            let trimmed: Vec<u8> = after.iter().copied().skip_while(|&b| b == b' ').collect();
            return trimmed.starts_with(b"false");
        }
    }
    false
}

fn is_matching_constructor_call(call: &ruby_prism::CallNode<'_>, const_name: &[u8]) -> bool {
    if call.name().as_slice() != b"new" {
        return false;
    }

    let Some(receiver) = call.receiver() else {
        return false;
    };

    if let Some(cr) = receiver.as_constant_read_node() {
        return cr.name().as_slice() == const_name;
    }

    if let Some(cp) = receiver.as_constant_path_node() {
        return cp.parent().is_none()
            && cp.name().is_some_and(|name| name.as_slice() == const_name);
    }

    false
}

fn call_has_no_arguments(call_node: &ruby_prism::CallNode<'_>) -> bool {
    call_node
        .arguments()
        .is_none_or(|args| args.arguments().is_empty())
}

fn is_target_direct_block_body(
    body: Option<ruby_prism::Node<'_>>,
    target_start: usize,
    target_end: usize,
) -> bool {
    let Some(body) = body else {
        return false;
    };

    if let Some(statements_node) = body.as_statements_node() {
        if statements_node.body().len() != 1 {
            return false;
        }

        let Some(statement) = statements_node.body().iter().next() else {
            return false;
        };
        let loc = statement.location();
        return loc.start_offset() == target_start && loc.end_offset() == target_end;
    }

    let loc = body.location();
    loc.start_offset() == target_start && loc.end_offset() == target_end
}

struct DirectConstructorBodyFinder<'a> {
    const_name: &'a [u8],
    target_start: usize,
    target_end: usize,
    found: bool,
}

impl<'a, 'pr> Visit<'pr> for DirectConstructorBodyFinder<'a> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.found {
            return;
        }

        if is_matching_constructor_call(node, self.const_name)
            && call_has_no_arguments(node)
            && node
                .block()
                .and_then(|block| block.as_block_node())
                .is_some_and(|block_node| {
                    is_target_direct_block_body(
                        block_node.body(),
                        self.target_start,
                        self.target_end,
                    )
                })
        {
            self.found = true;
            return;
        }

        ruby_prism::visit_call_node(self, node);
    }
}

fn is_direct_body_of_constructor_block(
    parse_result: &ruby_prism::ParseResult<'_>,
    call_node: &ruby_prism::CallNode<'_>,
    const_name: &[u8],
) -> bool {
    let loc = call_node.location();
    let mut finder = DirectConstructorBodyFinder {
        const_name,
        target_start: loc.start_offset(),
        target_end: loc.end_offset(),
        found: false,
    };
    finder.visit(&parse_result.node());
    finder.found
}

impl Cop for EmptyLiteral {
    fn name(&self) -> &'static str {
        "Style/EmptyLiteral"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, CONSTANT_PATH_NODE, CONSTANT_READ_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let call_node = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call_node.name();
        let method_bytes = method_name.as_slice();

        // Must be `new` or `[]`
        if method_bytes != b"new" && method_bytes != b"[]" {
            return;
        }

        // Must have a constant receiver: Array, Hash, or String
        let receiver = match call_node.receiver() {
            Some(r) => r,
            None => return,
        };

        let const_name: Vec<u8> = if let Some(cr) = receiver.as_constant_read_node() {
            cr.name().as_slice().to_vec()
        } else if let Some(cp) = receiver.as_constant_path_node() {
            // Handle ::Array, ::Hash, ::String
            let child_name = match cp.name() {
                Some(n) => n.as_slice().to_vec(),
                None => return,
            };
            // Only allow if the parent is nil/cbase (top-level)
            if cp.parent().is_some() {
                return;
            }
            child_name
        } else {
            return;
        };

        // Must have no arguments (empty constructor)
        if !call_has_no_arguments(&call_node) {
            // Exception: Array.new with empty array arg or Array[] with empty
            return;
        }

        // Must not have a block (Hash.new { |h, k| h[k] = [] })
        if call_node.block().is_some() {
            return;
        }

        // RuboCop only skips a nested constructor when it is the direct body
        // expression of an outer zero-argument constructor block.
        if method_bytes == b"new"
            && const_name.as_slice() == b"Array"
            && is_direct_body_of_constructor_block(parse_result, &call_node, b"Array")
        {
            return;
        }
        if method_bytes == b"new"
            && const_name.as_slice() == b"Hash"
            && is_direct_body_of_constructor_block(parse_result, &call_node, b"Hash")
        {
            return;
        }

        // String.new is only flagged when frozen_string_literal: false is explicitly set.
        // When the comment is absent or set to true, String.new may be needed for
        // a mutable empty string, so we don't flag it.
        if const_name.as_slice() == b"String"
            && method_bytes == b"new"
            && !has_frozen_string_literal_false(source)
        {
            return;
        }

        let (msg, replacement) = match const_name.as_slice() {
            b"Array" if method_bytes == b"new" || method_bytes == b"[]" => {
                let src = String::from_utf8_lossy(call_node.location().as_slice());
                (
                    format!("Use array literal `[]` instead of `{}`.", src),
                    "[]",
                )
            }
            b"Hash" if method_bytes == b"new" || method_bytes == b"[]" => {
                let src = String::from_utf8_lossy(call_node.location().as_slice());
                (
                    format!("Use hash literal `{{}}` instead of `{}`.", src),
                    "{}",
                )
            }
            b"String" if method_bytes == b"new" => (
                "Use string literal `''` instead of `String.new`.".to_string(),
                "''",
            ),
            _ => return,
        };

        let loc = call_node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        let mut diag = self.diagnostic(source, line, column, msg);
        if let Some(corr) = corrections {
            corr.push(crate::correction::Correction {
                start: loc.start_offset(),
                end: loc.end_offset(),
                replacement: replacement.to_string(),
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

    crate::cop_fixture_tests!(EmptyLiteral, "cops/style/empty_literal");
    crate::cop_autocorrect_fixture_tests!(EmptyLiteral, "cops/style/empty_literal");

    #[test]
    fn no_offense_string_new_with_frozen_string_literal() {
        let diags = crate::testutil::run_cop_full(
            &EmptyLiteral,
            b"# frozen_string_literal: true\n\ns = String.new\n",
        );
        assert!(
            diags.is_empty(),
            "String.new should not be flagged when frozen_string_literal is true"
        );
    }

    #[test]
    fn no_offense_string_new_without_frozen_string_literal() {
        let diags = crate::testutil::run_cop_full(&EmptyLiteral, b"s = String.new\n");
        assert!(
            diags.is_empty(),
            "String.new should not be flagged when frozen_string_literal comment is absent"
        );
    }

    #[test]
    fn offense_string_new_with_frozen_string_literal_false() {
        let diags = crate::testutil::run_cop_full(
            &EmptyLiteral,
            b"# frozen_string_literal: false\n\ns = String.new\n",
        );
        assert!(
            !diags.is_empty(),
            "String.new should be flagged when frozen_string_literal is false"
        );
    }
}
