use crate::cop::shared::node_type::{
    CONSTANT_OR_WRITE_NODE, CONSTANT_PATH_OR_WRITE_NODE, CONSTANT_PATH_WRITE_NODE,
    CONSTANT_WRITE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/MutableConstant: freeze mutable objects assigned to constants.
///
/// ## 2026-03-29 investigation
///
/// - FP: plain string constants were still flagged when the file used a long
///   leading comment block before `# frozen_string_literal: true`, or the
///   hyphenated `# frozen-string-literal: true` spelling that RuboCop accepts.
/// - FN: continued strings like `"foo #{bar}" \ "baz"` were treated as plain
///   strings under `frozen_string_literal: true` because Prism wraps them in an
///   outer `InterpolatedStringNode` whose nested parts must be inspected
///   recursively to find interpolation.
/// - Fix: scan the full leading comment block for both frozen-string-literal
///   spellings, and recurse through nested interpolated-string parts before
///   treating a continued string as already frozen.
///
/// ## 2026-03-31 investigation
///
/// - FP: `has_frozen_string_literal_true` did not handle `=begin`/`=end` block
///   comments in the leading section. Files with a `=begin` license block before
///   `# frozen_string_literal: true` (e.g. `grosser/maxitest`) caused the
///   scanner to hit `=begin` as a non-comment, non-blank line and `break`,
///   missing the magic comment entirely. Plain string constants were then
///   falsely flagged.
/// - Fix: skip `=begin`/`=end` block comments during the leading-section scan.
///
/// ## 2026-04-07 investigation (strict mode variant)
///
/// - FP: interpolated symbol constants (e.g. `:"#{name}.batched_queries"`) were
///   incorrectly flagged as mutable. Symbols are always immutable in Ruby, and
///   RuboCop's `immutable_literal?` includes `dsym` (interpolated symbol).
/// - Fix: added `as_interpolated_symbol_node()` to `is_immutable_literal()`.
///   Unlike plain `SymbolNode`, `InterpolatedSymbolNode` was missing from the check.
///
/// ## 2026-04-08 investigation (strict mode variant)
///
/// - FP: bare range constants (e.g. `200..299`, `MIN..MAX`) were incorrectly
///   flagged in strict mode. RuboCop treats bare regexp and range literals as
///   immutable when `TargetRubyVersion >= 3.0`.
/// - Root cause: a prior strict-mode change removed ranges from the immutable
///   path entirely, which matched neither the vendored RuboCop source nor the
///   corpus examples.
/// - Fix: thread `TargetRubyVersion` into strict-mode literal checks and treat
///   bare regexps/ranges as immutable only on Ruby 3.0+.
/// - RuboCop quirk preserved: parenthesized ranges like `(1..99)` stay
///   offenses in strict mode because RuboCop does not unwrap the parentheses
///   before calling `immutable_literal?`.
pub struct MutableConstant;

impl MutableConstant {
    /// Check if a node is a mutable literal (array, hash, string, xstring).
    /// In `literals` mode, only literal values are flagged.
    /// Matches RuboCop's MUTABLE_LITERALS = %i[str dstr xstr array hash regexp irange erange]
    /// (regexp/range are excluded via frozen_regexp_or_range_literals? for Ruby 3.0+).
    fn is_mutable_literal(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
        node.as_array_node().is_some()
            || node.as_hash_node().is_some()
            || node.as_keyword_hash_node().is_some()
            || node.as_string_node().is_some()
            || node.as_source_file_node().is_some()
            || Self::is_interpolated_string(source, node)
            // XStringNode = backtick literal (`command`), always mutable
            || node.as_x_string_node().is_some()
            // InterpolatedXStringNode = backtick with interpolation (`cmd #{x}`)
            || node.as_interpolated_x_string_node().is_some()
    }

    /// Check if node is a non-interpolated string literal (StringNode only, no heredocs
    /// with interpolation). `frozen_string_literal: true` only freezes these.
    fn is_plain_string(_source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
        if let Some(s) = node.as_string_node() {
            // Heredocs are mutable even with frozen_string_literal: true in Ruby 3.0+
            // ... actually no: plain (non-interpolated) heredocs ARE frozen with the magic comment.
            // Only interpolated heredocs are not frozen.
            // StringNode = non-interpolated, so always plain.
            let _ = s;
            // But we need to check: is it a heredoc? Plain heredocs are still frozen
            // with the magic comment. StringNode heredocs are non-interpolated, so they're fine.
            return true;
        }
        // InterpolatedStringNode that has NO actual interpolation parts:
        // In Ruby, `"hello"` can parse as InterpolatedStringNode in some contexts,
        // but for frozen_string_literal purposes, only non-interpolated strings are frozen.
        // Multiline string concatenation with `\` produces InterpolatedStringNode.
        // We need to check if it actually has interpolation.
        if let Some(isn) = node.as_interpolated_string_node() {
            if !Self::interpolated_string_has_interpolation(&isn) {
                // Also check: is it a heredoc?
                // Non-interpolated heredocs are frozen with the magic comment.
                // Non-interpolated multiline strings are frozen too.
                return true;
            }
        }
        false
    }

    fn interpolated_string_has_interpolation(
        node: &ruby_prism::InterpolatedStringNode<'_>,
    ) -> bool {
        node.parts().iter().any(|part| {
            part.as_embedded_statements_node().is_some()
                || part.as_embedded_variable_node().is_some()
                || part
                    .as_interpolated_string_node()
                    .is_some_and(|nested| Self::interpolated_string_has_interpolation(&nested))
        })
    }

    /// Check if node is an InterpolatedStringNode (which includes heredocs with interpolation).
    fn is_interpolated_string(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
        if let Some(isn) = node.as_interpolated_string_node() {
            // Check if it's a heredoc
            if let Some(opening) = isn.opening_loc() {
                let bytes = &source.as_bytes()[opening.start_offset()..opening.end_offset()];
                if bytes.starts_with(b"<<") {
                    return true;
                }
            }
            // Regular interpolated string
            return true;
        }
        false
    }

    /// Check if the value is a `.freeze` call (meaning the value is already frozen).
    fn is_frozen_value(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(call) = node.as_call_node() {
            if call.name().as_slice() == b"freeze" {
                return true;
            }
        }
        false
    }

    /// For `strict` mode: check if the value is an immutable literal.
    /// Bare regexp and range literals are frozen only on Ruby 3.0+.
    /// Parenthesized ranges stay offenses to match RuboCop's `begin`-node quirk.
    fn is_immutable_literal(node: &ruby_prism::Node<'_>, target_ruby_version: f64) -> bool {
        node.as_integer_node().is_some()
            || node.as_float_node().is_some()
            || node.as_symbol_node().is_some()
            || node.as_interpolated_symbol_node().is_some()
            || node.as_true_node().is_some()
            || node.as_false_node().is_some()
            || node.as_nil_node().is_some()
            || node.as_rational_node().is_some()
            || node.as_imaginary_node().is_some()
            || node.as_source_line_node().is_some()
            || node.as_source_encoding_node().is_some()
            || (target_ruby_version >= 3.0
                && (node.as_regular_expression_node().is_some()
                    || node.as_interpolated_regular_expression_node().is_some()
                    || node.as_range_node().is_some()))
    }

    /// For `strict` mode: check if operation produces an immutable object.
    /// Matches RuboCop's `operation_produces_immutable_object?` NodePattern.
    fn operation_produces_immutable_object(node: &ruby_prism::Node<'_>) -> bool {
        // Constants (OTHER_CONST, Namespace::CONST) are immutable references
        if node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some() {
            return true;
        }

        if let Some(call) = node.as_call_node() {
            let name = call.name();
            let name_bytes = name.as_slice();

            // .freeze calls
            if name_bytes == b"freeze" {
                return true;
            }

            // Struct.new / ::Struct.new
            if name_bytes == b"new" {
                if let Some(recv) = call.receiver() {
                    if Self::is_struct_constant(&recv) {
                        return true;
                    }
                }
            }

            // ENV['foo'] / ::ENV['foo']
            if name_bytes == b"[]" {
                if let Some(recv) = call.receiver() {
                    if Self::is_env_constant(&recv) {
                        return true;
                    }
                }
            }

            // Comparison operators: ==, ===, !=, <=, >=, <, >
            if matches!(
                name_bytes,
                b"==" | b"===" | b"!=" | b"<=" | b">=" | b"<" | b">"
            ) {
                return true;
            }

            // count/length/size methods
            if matches!(name_bytes, b"count" | b"length" | b"size") {
                return true;
            }

            // Arithmetic with int/float operands: int/float op anything, or anything op int/float
            if matches!(name_bytes, b"+" | b"-" | b"*" | b"**" | b"/" | b"%" | b"<<") {
                if let Some(recv) = call.receiver() {
                    if recv.as_integer_node().is_some() || recv.as_float_node().is_some() {
                        return true;
                    }
                }
                let args = call.arguments();
                if let Some(args) = args {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if let Some(arg) = arg_list.first() {
                        if arg.as_integer_node().is_some() || arg.as_float_node().is_some() {
                            return true;
                        }
                    }
                }
            }
        }

        // Block with Struct.new: `Struct.new(:a) do ... end`
        if let Some(block) = node.as_call_node() {
            // Already handled above via call_node
            let _ = block;
        }

        // ENV['foo'] || 'fallback'
        if let Some(or_node) = node.as_or_node() {
            let left = or_node.left();
            if let Some(call) = left.as_call_node() {
                if call.name().as_slice() == b"[]" {
                    if let Some(recv) = call.receiver() {
                        if Self::is_env_constant(&recv) {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    fn is_struct_constant(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(cr) = node.as_constant_read_node() {
            return cr.name().as_slice() == b"Struct";
        }
        if let Some(cp) = node.as_constant_path_node() {
            // ::Struct
            if cp.parent().is_none() {
                if let Some(name) = cp.name() {
                    return name.as_slice() == b"Struct";
                }
            }
        }
        false
    }

    fn is_env_constant(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(cr) = node.as_constant_read_node() {
            return cr.name().as_slice() == b"ENV";
        }
        if let Some(cp) = node.as_constant_path_node() {
            // ::ENV
            if cp.parent().is_none() {
                if let Some(name) = cp.name() {
                    return name.as_slice() == b"ENV";
                }
            }
        }
        false
    }

    fn is_blank_line(line: &[u8]) -> bool {
        line.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r')
    }

    fn is_comment_line(line: &[u8]) -> bool {
        let trimmed = line.iter().skip_while(|&&b| b == b' ' || b == b'\t');
        matches!(trimmed.clone().next(), Some(b'#'))
    }

    /// Check if a line starts a `=begin` block comment.
    fn is_begin_block_comment(line: &[u8]) -> bool {
        line.starts_with(b"=begin")
            && line
                .get(6)
                .is_none_or(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
    }

    /// Check if a line ends a `=begin` block comment with `=end`.
    fn is_end_block_comment(line: &[u8]) -> bool {
        line.starts_with(b"=end")
            && line
                .get(4)
                .is_none_or(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
    }

    fn starts_with_frozen_string_literal_key(s: &str) -> bool {
        let lower = s.to_ascii_lowercase();
        let bytes = lower.as_bytes();
        bytes.starts_with(b"frozen")
            && bytes.len() >= 22
            && (bytes[6] == b'_' || bytes[6] == b'-')
            && bytes[7..].starts_with(b"string")
            && (bytes[13] == b'_' || bytes[13] == b'-')
            && bytes[14..].starts_with(b"literal:")
    }

    fn strip_prefix_frozen_string_literal_key(s: &str) -> Option<&str> {
        if Self::starts_with_frozen_string_literal_key(s) {
            Some(&s[22..])
        } else {
            None
        }
    }

    fn strip_frozen_string_literal_key(s: &str) -> Option<&str> {
        let lower = s.to_ascii_lowercase();
        let bytes = lower.as_bytes();

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

    fn is_frozen_string_literal_true_comment(line: &[u8]) -> bool {
        let s = match std::str::from_utf8(line) {
            Ok(s) => s.trim_start(),
            Err(_) => return false,
        };
        let trimmed = s.strip_prefix('#').unwrap_or("").trim_start();

        if trimmed.starts_with("-*-") && trimmed.ends_with("-*-") {
            if let Some(after_key) = Self::strip_frozen_string_literal_key(trimmed) {
                let value = after_key.split([';', '-']).next().unwrap_or("");
                return value.trim() == "true";
            }
            return false;
        }

        Self::strip_prefix_frozen_string_literal_key(trimmed)
            .is_some_and(|value| value.trim() == "true")
    }

    /// Check if the source file has a leading frozen string literal magic comment.
    /// This makes plain string literals frozen, but not interpolated strings.
    fn has_frozen_string_literal_true(source: &SourceFile) -> bool {
        let mut iter = source.lines();
        while let Some(line) = iter.next() {
            if Self::is_blank_line(line) {
                continue;
            }
            // Skip =begin ... =end block comments in the leading section
            if Self::is_begin_block_comment(line) {
                for inner in iter.by_ref() {
                    if Self::is_end_block_comment(inner) {
                        break;
                    }
                }
                continue;
            }
            if Self::is_comment_line(line) {
                if Self::is_frozen_string_literal_true_comment(line) {
                    return true;
                }
                continue;
            }
            break;
        }

        false
    }

    /// Find the most recent `shareable_constant_value` magic comment that applies
    /// to the given byte offset. Returns true if it enables sharing (literal,
    /// experimental_everything, experimental_copy), false otherwise.
    fn has_shareable_constant_value(source: &SourceFile, node_offset: usize) -> bool {
        let (node_line, _) = source.offset_to_line_col(node_offset);
        let mut result = false;

        let lines = source.lines();
        for (i, line) in lines.enumerate() {
            let line_num = i + 1;
            if line_num > node_line {
                break;
            }
            let s = match std::str::from_utf8(line) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let s = s.trim();
            if let Some(rest) = s.strip_prefix('#') {
                let rest = rest.trim_start();
                // Simple format: # shareable_constant_value: literal
                if let Some(value) = rest.strip_prefix("shareable_constant_value:") {
                    let value = value.trim();
                    result = matches!(
                        value,
                        "literal" | "experimental_everything" | "experimental_copy"
                    );
                }
                // Emacs format: # -*- shareable_constant_value: literal -*-
                if rest.starts_with("-*-") && rest.ends_with("-*-") {
                    let inner = &rest[3..rest.len() - 3].trim();
                    for directive in inner.split(';') {
                        let directive = directive.trim();
                        if let Some(value) = directive.strip_prefix("shareable_constant_value:") {
                            let value = value.trim();
                            result = matches!(
                                value,
                                "literal" | "experimental_everything" | "experimental_copy"
                            );
                        }
                    }
                }
            }
        }
        result
    }

    /// Check if a `CallNode` wraps a Struct.new with a block (strict mode immutable).
    fn is_struct_new_block(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(call) = node.as_call_node() {
            if call.name().as_slice() == b"new" && call.block().is_some() {
                if let Some(recv) = call.receiver() {
                    return Self::is_struct_constant(&recv);
                }
            }
        }
        false
    }

    fn check_value(
        &self,
        source: &SourceFile,
        value: &ruby_prism::Node<'_>,
        frozen_strings: bool,
        enforced_style: &str,
        target_ruby_version: f64,
    ) -> Vec<Diagnostic> {
        // Already frozen via .freeze call
        if Self::is_frozen_value(value) {
            return Vec::new();
        }

        // Check shareable_constant_value magic comment
        if Self::has_shareable_constant_value(source, value.location().start_offset()) {
            return Vec::new();
        }

        if enforced_style == "strict" {
            // Strict mode: flag everything that isn't immutable
            if Self::is_immutable_literal(value, target_ruby_version) {
                return Vec::new();
            }
            if Self::operation_produces_immutable_object(value) {
                return Vec::new();
            }
            if Self::is_struct_new_block(value) {
                return Vec::new();
            }
            // In strict mode, frozen_string_literal: true makes plain strings immutable
            if frozen_strings && Self::is_plain_string(source, value) {
                return Vec::new();
            }
        } else {
            // Literals mode: only flag mutable literals
            if !Self::is_mutable_literal(source, value) {
                return Vec::new();
            }
            // When frozen_string_literal: true is set, plain (non-interpolated) string
            // constants are already frozen — don't flag them.
            // But interpolated strings are NOT frozen in Ruby 3.0+.
            if frozen_strings && Self::is_plain_string(source, value) {
                return Vec::new();
            }
        }

        // Point at the mutable value (RHS), matching RuboCop behavior
        let (line, column) = source.offset_to_line_col(value.location().start_offset());
        vec![self.diagnostic(
            source,
            line,
            column,
            "Freeze mutable objects assigned to constants.".to_string(),
        )]
    }
}

impl Cop for MutableConstant {
    fn name(&self) -> &'static str {
        "Style/MutableConstant"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CONSTANT_OR_WRITE_NODE,
            CONSTANT_PATH_OR_WRITE_NODE,
            CONSTANT_PATH_WRITE_NODE,
            CONSTANT_WRITE_NODE,
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
        let enforced_style = config.get_str("EnforcedStyle", "literals");
        let frozen_strings = Self::has_frozen_string_literal_true(source);
        let target_ruby_version = target_ruby_version(config);

        // Check ConstantWriteNode (CONST = value)
        if let Some(cw) = node.as_constant_write_node() {
            let value = cw.value();
            diagnostics.extend(self.check_value(
                source,
                &value,
                frozen_strings,
                enforced_style,
                target_ruby_version,
            ));
            return;
        }

        // Check ConstantPathWriteNode (Module::CONST = value)
        if let Some(cpw) = node.as_constant_path_write_node() {
            let value = cpw.value();
            diagnostics.extend(self.check_value(
                source,
                &value,
                frozen_strings,
                enforced_style,
                target_ruby_version,
            ));
            return;
        }

        // Check ConstantOrWriteNode (CONST ||= value)
        if let Some(cow) = node.as_constant_or_write_node() {
            let value = cow.value();
            diagnostics.extend(self.check_value(
                source,
                &value,
                frozen_strings,
                enforced_style,
                target_ruby_version,
            ));
            return;
        }

        // Check ConstantPathOrWriteNode (Module::CONST ||= value)
        if let Some(cpow) = node.as_constant_path_or_write_node() {
            let value = cpow.value();
            diagnostics.extend(self.check_value(
                source,
                &value,
                frozen_strings,
                enforced_style,
                target_ruby_version,
            ));
        }
    }
}

fn target_ruby_version(config: &CopConfig) -> f64 {
    config
        .options
        .get("TargetRubyVersion")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_u64().map(|value| value as f64))
        })
        .unwrap_or(2.7)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
    };
    crate::cop_fixture_tests!(MutableConstant, "cops/style/mutable_constant");

    fn strict_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".into(),
            serde_yml::Value::String("strict".into()),
        );
        options.insert(
            "TargetRubyVersion".into(),
            serde_yml::Value::Number(4.into()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn strict_no_offense_fixture() {
        assert_cop_no_offenses_full_with_config(
            &MutableConstant,
            include_bytes!(
                "../../../tests/fixtures/cops/style/mutable_constant/strict_no_offense.rb"
            ),
            strict_config(),
        );
    }

    #[test]
    fn strict_parenthesized_range_is_an_offense() {
        assert_cop_offenses_full_with_config(
            &MutableConstant,
            b"PARENTHESIZED_RANGE = (1..99)\n# nitrocop-expect: 1:22 Style/MutableConstant: Freeze mutable objects assigned to constants.\n",
            strict_config(),
        );
    }

    /// XStringNode (backtick) should be flagged even with frozen_string_literal: true.
    /// The magic comment only freezes str/dstr, not xstr.
    #[test]
    fn xstring_flagged_with_frozen_string_literal() {
        let cop = MutableConstant;
        let source = b"# frozen_string_literal: true\n\nCONST = `uname`\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            1,
            "xstring should be flagged even with frozen_string_literal: true, got {:?}",
            diags
        );
    }

    /// Plain strings should NOT be flagged with frozen_string_literal: true.
    #[test]
    fn plain_string_not_flagged_with_frozen_string_literal() {
        let cop = MutableConstant;
        let source = b"# frozen_string_literal: true\n\nCONST = \"hello\"\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            0,
            "plain string should not be flagged with frozen_string_literal: true, got {:?}",
            diags
        );
    }

    /// Emacs-style frozen_string_literal comment should also suppress plain strings.
    #[test]
    fn emacs_style_frozen_string_literal() {
        let cop = MutableConstant;
        let source = b"# -*- frozen_string_literal: true -*-\n\nCONST = \"hello\"\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            0,
            "Emacs-style frozen_string_literal should suppress plain string offense, got {:?}",
            diags
        );
    }

    /// Emacs-style frozen_string_literal combined with encoding.
    #[test]
    fn emacs_style_combined_magic_comment() {
        let cop = MutableConstant;
        let source = b"# -*- coding: utf-8; frozen_string_literal: true -*-\n\nCONST = \"hello\"\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            0,
            "Emacs-style combined magic comment should suppress plain string offense, got {:?}",
            diags
        );
    }

    /// Long header comments and hyphenated magic comments still freeze plain strings.
    #[test]
    fn hyphenated_frozen_string_literal_after_header() {
        let cop = MutableConstant;
        let source =
            b"# Copyright 2026 Nitrocop\n#\n# frozen-string-literal: true\n\nCONST = \"/\"\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            0,
            "header + frozen-string-literal should suppress plain string offense, got {:?}",
            diags
        );
    }

    /// Continued strings with nested interpolation remain mutable with frozen string literals.
    #[test]
    fn continued_interpolated_string_flagged_with_frozen_string_literal() {
        let cop = MutableConstant;
        let source = b"# frozen_string_literal: true\n\nETCD_URL = \"https://github.com/coreos/etcd/releases/download/\" \\\n           \"#{ETCD_VERSION}/etcd-#{ETCD_VERSION}-linux-amd64.tar.gz\"\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            1,
            "continued interpolated strings should still be flagged with frozen_string_literal, got {:?}",
            diags
        );
    }

    /// `__FILE__` behaves like a mutable string and should be flagged.
    #[test]
    fn source_file_node_flagged() {
        let cop = MutableConstant;
        let source = b"FILE_PATH = __FILE__\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            1,
            "__FILE__ should be flagged like a mutable string, got {:?}",
            diags
        );
    }

    /// `__LINE__` remains an immutable numeric literal.
    #[test]
    fn source_line_node_not_flagged() {
        let cop = MutableConstant;
        let source = b"LINE_NO = __LINE__\n";
        let diags =
            crate::testutil::run_cop_full_internal(&cop, source, CopConfig::default(), "test.rb");
        assert_eq!(
            diags.len(),
            0,
            "__LINE__ should remain immutable, got {:?}",
            diags
        );
    }
}
