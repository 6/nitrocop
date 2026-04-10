use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::node_type::{
    ARRAY_NODE, ASSOC_NODE, BLOCK_NODE, BLOCK_PARAMETERS_NODE, CALL_NODE, CONSTANT_PATH_NODE,
    CONSTANT_READ_NODE, FALSE_NODE, FLOAT_NODE, HASH_NODE, IMAGINARY_NODE, INTEGER_NODE,
    INTERPOLATED_STRING_NODE, NIL_NODE, RANGE_NODE, RATIONAL_NODE, REGULAR_EXPRESSION_NODE,
    STATEMENTS_NODE, STRING_NODE, SYMBOL_NODE, TRUE_NODE, X_STRING_NODE,
};
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Default style is `and_return` — flags block-style stubs returning static values.
///
/// **Investigation (2026-03):** 46 FPs caused by flagging `receive_message_chain` blocks.
/// RuboCop only flags `receive` calls, not `receive_message_chain`. Fixed by checking the
/// root method name in `find_block_on_receive_chain` and skipping non-`receive` chains.
/// Detects: `allow(X).to receive(:y) { static_value }`
///
/// **Investigation (2026-03, FN=126):** `is_static_value` was missing several node types
/// that RuboCop's `recursive_literal_or_const?` considers static:
/// - Constants (`ConstantReadNode`, `ConstantPathNode` — e.g., `CONST`, `Foo::BAR`)
/// - Ranges (`RangeNode` — e.g., `1..10`)
/// - Regular expressions (`RegularExpressionNode` — e.g., `/pattern/`)
/// - Rational/imaginary literals (`RationalNode`, `ImaginaryNode` — e.g., `1r`, `1i`)
///
/// All added to match RuboCop's `recursive_literal_or_const?` behavior.
///
/// ## Corpus investigation (2026-03-14, round 1)
///
/// Corpus oracle reported FP=2, FN=86.
///
/// FP=2: Both in procore-oss/blueprinter `spec/units/blueprint_validator_spec.rb:26`
/// and `:35`. Source file not in local corpus. Cannot diagnose without concrete
/// reproduction. Possible cause: `allow(X).to receive(:y) { complex_value }` where
/// our `is_static_value` incorrectly returns true for a dynamic expression.
///
/// FN=86: Large FN count initially suspected to be missing chained pattern handling.
///
/// ## Corpus investigation (2026-03-15, round 2)
///
/// Investigated FN=86 root cause hypothesis: that chained calls like
/// `allow(Q).to receive(:meaning).with(:universe) { 42 }` were not detected.
///
/// **Finding:** The chained patterns ARE correctly detected. Prism parses
/// `receive(:meaning).with(:universe) { 42 }` as a CallNode (`.with`) with a
/// block, not a BlockNode argument to `.to`. So `find_block_on_receive_chain`
/// receives a CallNode, correctly walks the chain to find `receive` at the root,
/// and returns the block. Confirmed via unit tests and Prism AST inspection.
///
/// ## Corpus investigation (2026-03-15, round 3)
///
/// FN=112 root cause: the receiver name check only accepted `allow` and `expect`,
/// missing `allow_any_instance_of` and `expect_any_instance_of`. These are RSpec DSL
/// methods called without a receiver, used for any-instance stubbing. Corpus examples:
/// `allow_any_instance_of(Foo).to receive(:bar) { true }`. Fixed by adding both
/// names to the receiver check. Both are receiverless calls, so the existing
/// `recv_call.receiver().is_some()` guard is already correct for them.
///
/// ## Corpus investigation (2026-03-18, round 4)
///
/// FP=2: Both from procore-oss/blueprinter. Pattern:
/// `expect { ... }.to raise_error(Err) do 'msg' end`. The `block_on_to` fallback
/// was incorrectly used even when the argument to `.to` was NOT a `receive` chain
/// (it was `raise_error`). Fixed by gating `block_on_to` on `is_receive_chain()`.
///
/// FN=13 (3 root causes):
/// 1. `is_expected.to receive(:can?) { true }` (9 FN, cancancan): `is_expected` is
///    equivalent to `expect(subject)` and was not in the receiver name check. Fixed.
/// 2. Blocks with parameters but static body (4 FN: fastlane 2, Freika 1, opal 1):
///    e.g., `receive(:foo) do |arg| nil end`. RuboCop only checks body staticness,
///    not whether the block has parameters. Removed the parameter skip.
///
/// ## Corpus investigation (2026-03-29)
///
/// FN=12 concentrated in two detector gaps:
/// 1. Matcher-style receivers such as `wrapped.to receive(:foo) { 5 }` and helper
///    wrappers such as `allow_it.to receive(:results) { :all }` were missed because
///    we required `.to` to be called on specific receiver names (`allow`, `expect`,
///    etc.). RuboCop only requires a receiverless `receive(...)` matcher anywhere in
///    the `.to` argument chain, so the receiver-name gate was too narrow.
/// 2. Static pseudo-literals like `__FILE__` were not treated as static values.
///    RuboCop's `recursive_literal_or_const?` treats `__FILE__`, `__LINE__`, and
///    `__ENCODING__` as literal-like, so we now accept Prism's corresponding source
///    pseudo-literal nodes in `is_static_value`.
///
/// ## Corpus investigation (2026-03-30)
///
/// FP=1: `allow(...).to receive(...) { 'RefundPayment'.freeze }` is accepted by
/// RuboCop. `recursive_literal_or_const?` stays false for the `send(..., :freeze)`
/// body even when the receiver is a string literal, so the earlier `.freeze`
/// special-case was too broad and caused the corpus FP.
///
/// FN=1: `allow(driver).to receive(:\`) do |cmd|; \`false\`; "Error"; end`
/// should be flagged. Prism parses the backtick command as `XStringNode`, and
/// RuboCop treats `begin(xstr, str)` as static because both child nodes are
/// literal-like. Added `XStringNode` handling in `is_static_value`.
///
/// ## Corpus investigation (2026-04-08, `EnforcedStyle: block`)
///
/// Variant divergence was a mix of one FP/FN location split plus two real
/// detector gaps:
/// 1. Offenses were reported at the whole send location (`allow(...)`) instead
///    of the `and_return` selector. On multiline chains this produced paired
///    corpus mismatches like 24pullrequests `:140` (our FP) vs `:142`
///    (RuboCop's offense).
/// 2. Bare keyword-hash arguments (`and_return(foo: 42)`) were treated as
///    dynamic because Prism uses `KeywordHashNode`, while RuboCop still treats
///    them as static via `recursive_literal_or_const?`.
/// 3. The block-style branch was broader than RuboCop: RuboCop only flags
///    `and_return` calls that contain a receiverless `receive(...)` matcher,
///    and it ignores multi-argument `and_return(1, 2)` as dynamic.
///
/// ## Corpus investigation (2026-04-09, `EnforcedStyle: block`)
///
/// FN=63 reduced to Prism's `InterpolatedStringNode` semantics. RuboCop's
/// `recursive_literal_or_const?` treats composite strings recursively, so
/// these are static and should be flagged in block style:
/// - constant-only interpolation (`"#{AWS_S3_ENDPOINT}/#{S3_BUCKET}"`)
/// - adjacent string literals parsed as a single interpolated string
///
/// Dynamic interpolation like `"#{bar}/path"` must still be ignored, so the
/// fix recurses through embedded statements and only accepts literal/constant
/// content there.
///
/// ## Corpus investigation (2026-04-09, `EnforcedStyle: block`, round 2)
///
/// FN=4 came from spaced calls like `and_return ("partiallll")` and
/// `and_return (0)`. Prism wraps those arguments in `ParenthesesNode`, while
/// RuboCop's Parser AST sees a `:begin` node and still applies
/// `recursive_literal_or_const?` recursively. Match RuboCop by recursing
/// through parenthesized bodies instead of treating the wrapper as dynamic.
pub struct ReturnFromStub;
impl Cop for ReturnFromStub {
    fn name(&self) -> &'static str {
        "RSpec/ReturnFromStub"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ARRAY_NODE,
            ASSOC_NODE,
            BLOCK_NODE,
            BLOCK_PARAMETERS_NODE,
            CALL_NODE,
            CONSTANT_PATH_NODE,
            CONSTANT_READ_NODE,
            FALSE_NODE,
            FLOAT_NODE,
            HASH_NODE,
            IMAGINARY_NODE,
            INTEGER_NODE,
            INTERPOLATED_STRING_NODE,
            NIL_NODE,
            RANGE_NODE,
            RATIONAL_NODE,
            REGULAR_EXPRESSION_NODE,
            STATEMENTS_NODE,
            STRING_NODE,
            SYMBOL_NODE,
            TRUE_NODE,
            X_STRING_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Config: EnforcedStyle — "and_return" (default) or "block"
        let enforced_style = config.get_str("EnforcedStyle", "and_return");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name().as_slice();

        // "block" style: flag `.and_return(value)` — prefer block form
        if enforced_style == "block" {
            if method_name == b"and_return" && contains_receive_matcher(node) {
                if let Some(args) = call.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if arg_list.len() == 1 && is_static_value(&arg_list[0]) {
                        let loc = call.message_loc().unwrap_or(call.location());
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use block for static values.".to_string(),
                        ));
                    }
                }
            }
            return;
        }

        // Default "and_return" style: flag block-style stubs returning static values
        // We need `.to` or `.not_to`
        if method_name != b"to" && method_name != b"not_to" && method_name != b"to_not" {
            return;
        }

        // Get the argument chain (receive(:y) or receive(:y).with(...))
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        // Find the `receive` call in the argument chain and check for a block on it
        let block_on_receive = find_block_on_receive_chain(&arg_list[0]);
        // Also check for a block on `.to` itself, but only if the argument chain
        // contains a `receive` call. Without this check, blocks on `.to raise_error(...)`
        // or other non-stub matchers would be incorrectly flagged.
        let block_on_to = if block_on_receive.is_none() && is_receive_chain(&arg_list[0]) {
            call.block()
        } else {
            None
        };

        let block_node = if let Some(b) = block_on_receive {
            b
        } else if let Some(b) = block_on_to {
            match b.as_block_node() {
                Some(bn) => bn,
                None => return,
            }
        } else {
            return;
        };

        // RuboCop does not skip blocks with parameters — it only checks the body.
        // A block like `receive(:foo) { |arg| nil }` is still flagged if the body is static.
        let body = match block_node.body() {
            Some(b) => b,
            None => {
                let loc = block_node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `and_return` for static values.".to_string(),
                ));
                return;
            }
        };

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        let stmt_list: Vec<_> = stmts.body().iter().collect();
        if stmt_list.is_empty() {
            return;
        }

        let all_static = stmt_list.iter().all(|s| is_static_value(s));
        if !all_static {
            return;
        }

        let loc = block_node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Use `and_return` for static values.".to_string(),
        ));
    }
}

fn contains_receive_matcher(node: &ruby_prism::Node<'_>) -> bool {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return false,
    };

    if method_dispatch_predicates::is_command(&call, b"receive") {
        return true;
    }

    if let Some(recv) = call.receiver() {
        if contains_receive_matcher(&recv) {
            return true;
        }
    }

    if let Some(args) = call.arguments() {
        for arg in args.arguments().iter() {
            if contains_receive_matcher(&arg) {
                return true;
            }
        }
    }

    false
}

/// Check if a call chain has `receive` as the root method name (without looking for blocks).
fn is_receive_chain(node: &ruby_prism::Node<'_>) -> bool {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return false,
    };
    let mut current = call;
    while let Some(recv) = current.receiver() {
        match recv.as_call_node() {
            Some(c) => current = c,
            None => return false,
        }
    }
    current.name().as_slice() == b"receive"
}

fn find_block_on_receive_chain<'a>(
    node: &ruby_prism::Node<'a>,
) -> Option<ruby_prism::BlockNode<'a>> {
    let call = node.as_call_node()?;
    let mut current = call;
    let mut block_node = None;
    // Walk the chain to find: (1) any block, and (2) the root method name
    loop {
        if block_node.is_none() {
            if let Some(block) = current.block() {
                block_node = block.as_block_node();
            }
        }
        match current.receiver() {
            Some(recv) => match recv.as_call_node() {
                Some(c) => current = c,
                None => return None,
            },
            None => break,
        }
    }
    // Only flag `receive` calls, not `receive_message_chain`
    let root_name = current.name();
    if root_name.as_slice() != b"receive" {
        return None;
    }
    block_node
}

fn is_static_value(node: &ruby_prism::Node<'_>) -> bool {
    // Simple literals
    if node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_string_node().is_some()
        || node.as_symbol_node().is_some()
        || node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_nil_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
        || node.as_regular_expression_node().is_some()
        || node.as_x_string_node().is_some()
        || node.as_source_file_node().is_some()
        || node.as_source_line_node().is_some()
        || node.as_source_encoding_node().is_some()
    {
        return true;
    }

    // Constants: Foo, Foo::BAR (recursive_literal_or_const? in RuboCop)
    if node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some() {
        return true;
    }

    // Ranges: 1..10, 1...10 — static if both endpoints are static
    if let Some(range) = node.as_range_node() {
        let left_ok = match range.left() {
            Some(l) => is_static_value(&l),
            None => true,
        };
        let right_ok = match range.right() {
            Some(r) => is_static_value(&r),
            None => true,
        };
        return left_ok && right_ok;
    }

    if let Some(string) = node.as_interpolated_string_node() {
        return string
            .parts()
            .iter()
            .all(|part| is_static_interpolated_part(&part));
    }

    if let Some(command) = node.as_interpolated_x_string_node() {
        return command
            .parts()
            .iter()
            .all(|part| is_static_interpolated_part(&part));
    }

    if let Some(symbol) = node.as_interpolated_symbol_node() {
        return symbol
            .parts()
            .iter()
            .all(|part| is_static_interpolated_part(&part));
    }

    if let Some(regex) = node.as_interpolated_regular_expression_node() {
        return regex
            .parts()
            .iter()
            .all(|part| is_static_interpolated_part(&part));
    }

    if let Some(parentheses) = node.as_parentheses_node() {
        let Some(body) = parentheses.body() else {
            return false;
        };

        if let Some(stmts) = body.as_statements_node() {
            let stmt_list: Vec<_> = stmts.body().iter().collect();
            return !stmt_list.is_empty() && stmt_list.iter().all(|stmt| is_static_value(stmt));
        }

        return is_static_value(&body);
    }

    if node.as_call_node().is_some() {
        return false;
    }

    if let Some(arr) = node.as_array_node() {
        return arr.elements().iter().all(|e| is_static_value(&e));
    }

    if let Some(hash) = node.as_hash_node() {
        return hash.elements().iter().all(|e| {
            if let Some(assoc) = e.as_assoc_node() {
                is_static_value(&assoc.key()) && is_static_value(&assoc.value())
            } else {
                false
            }
        });
    }

    if let Some(keyword_hash) = node.as_keyword_hash_node() {
        return keyword_hash.elements().iter().all(|e| {
            if let Some(assoc) = e.as_assoc_node() {
                is_static_value(&assoc.key()) && is_static_value(&assoc.value())
            } else {
                false
            }
        });
    }

    false
}

fn is_static_interpolated_part(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(embedded) = node.as_embedded_statements_node() {
        return embedded
            .statements()
            .is_some_and(|stmts| stmts.body().iter().all(|stmt| is_static_value(&stmt)));
    }

    is_static_value(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ReturnFromStub, "cops/rspec/return_from_stub");

    fn block_style_config() -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("block".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn block_style_flags_and_return() {
        let config = block_style_config();
        let source = b"allow(foo).to receive(:bar).and_return(42)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Use block for static values.");
    }

    #[test]
    fn chained_with_detects_offense() {
        let source = b"allow(Question).to receive(:meaning).with(:universe) { 42 }\n";
        let diags = crate::testutil::run_cop_full(&ReturnFromStub, source);
        assert_eq!(diags.len(), 1, "should detect block on chained .with()");
    }

    #[test]
    fn chained_once_detects_offense() {
        let source = b"expect(Foo).to receive(:bar).once { 42 }\n";
        let diags = crate::testutil::run_cop_full(&ReturnFromStub, source);
        assert_eq!(diags.len(), 1, "should detect block on chained .once");
    }

    #[test]
    fn block_style_does_not_flag_block_form() {
        let config = block_style_config();
        let source = b"allow(foo).to receive(:bar) { 42 }\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn block_style_flags_keyword_hash_argument() {
        let config = block_style_config();
        let source = b"allow(i).to receive(:settings).and_return(multiple: true, select_values: [\"handhelds\", \"fridges\"])\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Use block for static values.");
    }

    #[test]
    fn block_style_flags_constant_only_interpolated_string() {
        let config = block_style_config();
        let source = b"allow(service).to receive(:url).and_return(\"#{AWS_S3_ENDPOINT}/#{S3_BUCKET}/test-url\")\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Use block for static values.");
    }

    #[test]
    fn block_style_ignores_dynamic_interpolated_string() {
        let config = block_style_config();
        let source = b"bar = 42\nallow(service).to receive(:url).and_return(\"#{bar}/test-url\")\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn block_style_reports_selector_location() {
        let config = block_style_config();
        let source = b"allow(controller).to receive(:repo_with_labels)\n  .with('24pullrequests/24pullrequests')\n  .and_return({ data: { repository: { html_url: \"/foo\" }, labels: ['foo', 'bar'] }, status: 200 })\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 3);
        assert_eq!(diags[0].location.column, 3);
    }

    #[test]
    fn block_style_flags_parenthesized_static_literal() {
        let config = block_style_config();
        let source = b"allow(Foo).to receive(:bar).and_return (42)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Use block for static values.");
    }

    #[test]
    fn block_style_ignores_parenthesized_dynamic_value() {
        let config = block_style_config();
        let source = b"allow(Foo).to receive(:bar).and_return (value)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn block_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ReturnFromStub,
            include_bytes!("../../../tests/fixtures/cops/rspec/return_from_stub/block_offense.rb"),
            block_style_config(),
        );
    }

    #[test]
    fn block_style_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ReturnFromStub,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/return_from_stub/block_no_offense.rb"
            ),
            block_style_config(),
        );
    }

    #[test]
    fn block_style_ignores_irrelevant_and_return() {
        let config = block_style_config();
        let source = b"library.visit.and_return(42)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn block_style_ignores_multiple_return_values() {
        let config = block_style_config();
        let source = b"allow(foo).to receive(:bar).and_return(42, 43, 44)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ReturnFromStub, source, config);
        assert!(diags.is_empty());
    }
}
