use std::path::Path;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// FP fix (2026-03): slashes inside `#{}` interpolation segments were wrongly
/// counted as inner slashes, causing false "Use %r" suggestions on regexps like
/// `/#{Regexp.quote("</")}/ `. RuboCop's `node_body` only examines `:str` children,
/// so interpolation content is excluded. Fixed by iterating over Prism's `parts()`
/// and only collecting `StringNode` content for the slash check.
///
/// FP fix (2026-04): RuboCop checks the parser-visible regexp `content` for the
/// leading space/`=` exemption, and that content skips `#{...}` interpolation
/// nodes. As a result, method arguments like
/// `assert_match %r(#{attribute}="#{value}")` and
/// `with(%r{#{Regexp.escape(duration.to_s)} seconds})` are accepted because
/// their first literal string chunk begins with `=` or a space. Standalone
/// interpolated `%r` literals must still be flagged, so the exemption remains
/// limited to direct call-like arguments while switching the prefix check to
/// the string-part content RuboCop uses.
///
/// FP fix (2026-04): `%r/#{column_name} .../ =~ value` from Rails was still
/// flagged even though RuboCop accepts it. The same parser-visible leading
/// space rule also applies when the regexp is the receiver of a call node
/// such as `=~`, not only when it appears inside an argument list. Fixed by
/// extending the existing call-context exemption to direct call parents while
/// still rejecting standalone `%r/#{foo} bar/`.
///
/// Mixed-style fix (2026-04): RuboCop's `mixed` style still requires `%r` for
/// single-line slash literals when the regexp body contains a disallowed `/`,
/// so `/\/$/` must be flagged even though it is not multiline. `%r` literals in
/// call contexts are also allowed when `Style/MethodCallWithArgsParentheses`
/// uses `omit_parentheses`; the cop now honors an injected
/// `MethodCallWithArgsParenthesesEnforcedStyle` override and falls back to the
/// file's discovered RuboCop config to mirror that cross-cop allowance.
pub struct RegexpLiteral;

impl Cop for RegexpLiteral {
    fn name(&self) -> &'static str {
        "Style/RegexpLiteral"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = RegexpLiteralVisitor {
            cop: self,
            source,
            config,
            diagnostics,
            ancestors: Vec::new(),
            method_call_with_args_parentheses_omit: None,
        };
        visitor.visit(&parse_result.node());
    }
}

struct RegexpLiteralVisitor<'a, 'pr> {
    cop: &'a RegexpLiteral,
    source: &'a SourceFile,
    config: &'a CopConfig,
    diagnostics: &'a mut Vec<Diagnostic>,
    ancestors: Vec<ruby_prism::Node<'pr>>,
    method_call_with_args_parentheses_omit: Option<bool>,
}

impl<'pr> RegexpLiteralVisitor<'_, 'pr> {
    fn visit_candidate(&mut self, node: &ruby_prism::Node<'pr>) {
        let enforced_style = self.config.get_str("EnforcedStyle", "slashes");
        let allow_inner_slashes = self.config.get_bool("AllowInnerSlashes", false);

        let (open_bytes, content_bytes, node_start, node_end): (Vec<u8>, Vec<u8>, usize, usize) =
            if let Some(re) = node.as_regular_expression_node() {
                let opening = re.opening_loc();
                let content = re.content_loc().as_slice();
                let loc = re.location();
                (
                    opening.as_slice().to_vec(),
                    content.to_vec(),
                    loc.start_offset(),
                    loc.end_offset(),
                )
            } else if let Some(re) = node.as_interpolated_regular_expression_node() {
                let opening = re.opening_loc();
                let loc = re.location();
                let mut content = Vec::new();

                // Only collect content from string literal parts, skipping interpolation.
                // RuboCop's `node_body` only examines `:str` children, so slashes
                // inside `#{}` interpolation are not counted as inner slashes.
                for part in re.parts().iter() {
                    if let Some(s) = part.as_string_node() {
                        content.extend_from_slice(s.location().as_slice());
                    }
                }

                (
                    opening.as_slice().to_vec(),
                    content,
                    loc.start_offset(),
                    loc.end_offset(),
                )
            } else {
                return;
            };

        let is_slash = open_bytes == b"/";
        let is_percent_r = open_bytes.starts_with(b"%r");
        let has_slash = content_bytes.contains(&b'/');
        let is_multiline = {
            let (start_line, _) = self.source.offset_to_line_col(node_start);
            let (end_line, _) = self.source.offset_to_line_col(node_end);
            end_line > start_line
        };

        // %r with content starting with space or = may be used to avoid syntax errors
        // when the regexp is a direct call argument:
        //   do_something %r{ regexp}  # valid
        //   do_something / regexp/    # syntax error
        let content_starts_with_space_or_eq =
            !content_bytes.is_empty() && (content_bytes[0] == b' ' || content_bytes[0] == b'=');
        let allowed_percent_r_call_context = self.allowed_percent_r_call_context(
            node_start,
            node_end,
            content_starts_with_space_or_eq,
        );

        match enforced_style {
            "slashes" => {
                if is_percent_r {
                    if has_slash && !allow_inner_slashes {
                        return;
                    }
                    if allowed_percent_r_call_context {
                        return;
                    }
                    self.add_offense(node_start, "Use `//` around regular expression.");
                }
            }
            "percent_r" => {
                if is_slash {
                    self.add_offense(node_start, "Use `%r` around regular expression.");
                }
            }
            "mixed" => {
                if is_slash {
                    if is_multiline || (has_slash && !allow_inner_slashes) {
                        self.add_offense(node_start, "Use `%r` around regular expression.");
                    }
                } else if is_percent_r {
                    // Multiline %r is always correct in mixed style
                    if is_multiline {
                        return;
                    }
                    if has_slash && !allow_inner_slashes {
                        return;
                    }
                    if allowed_percent_r_call_context {
                        return;
                    }
                    self.add_offense(node_start, "Use `//` around regular expression.");
                }
            }
            _ => {}
        }

        if enforced_style == "slashes" && is_slash && has_slash && !allow_inner_slashes {
            self.add_offense(node_start, "Use `%r` around regular expression.");
        }
    }

    fn allowed_percent_r_call_context(
        &mut self,
        node_start: usize,
        node_end: usize,
        content_starts_with_space_or_eq: bool,
    ) -> bool {
        let allow_omit_parentheses = self.method_call_with_args_parentheses_omit();

        let Some(parent) = self.ancestors.last() else {
            return false;
        };

        if let Some(call) = parent.as_call_node() {
            if call.receiver().is_some_and(|receiver| {
                let loc = receiver.location();
                loc.start_offset() == node_start && loc.end_offset() == node_end
            }) {
                return content_starts_with_space_or_eq || allow_omit_parentheses;
            }

            return call.arguments().is_some_and(|args| {
                args.arguments().iter().any(|arg| {
                    let loc = arg.location();
                    loc.start_offset() == node_start
                        && loc.end_offset() == node_end
                        && (content_starts_with_space_or_eq || allow_omit_parentheses)
                })
            });
        }

        if let Some(super_node) = parent.as_super_node() {
            if !content_starts_with_space_or_eq {
                return false;
            }
            return super_node.arguments().is_some_and(|args| {
                args.arguments().iter().any(|arg| {
                    let loc = arg.location();
                    loc.start_offset() == node_start && loc.end_offset() == node_end
                })
            });
        }

        if let Some(yield_node) = parent.as_yield_node() {
            if !content_starts_with_space_or_eq {
                return false;
            }
            return yield_node.arguments().is_some_and(|args| {
                args.arguments().iter().any(|arg| {
                    let loc = arg.location();
                    loc.start_offset() == node_start && loc.end_offset() == node_end
                })
            });
        }

        if parent.as_arguments_node().is_some() {
            return self
                .ancestors
                .get(self.ancestors.len().saturating_sub(2))
                .is_some_and(|grandparent| {
                    if grandparent.as_call_node().is_some() {
                        content_starts_with_space_or_eq || allow_omit_parentheses
                    } else {
                        content_starts_with_space_or_eq
                            && (grandparent.as_super_node().is_some()
                                || grandparent.as_yield_node().is_some())
                    }
                });
        }

        false
    }

    fn method_call_with_args_parentheses_omit(&mut self) -> bool {
        if let Some(omit) = self.method_call_with_args_parentheses_omit {
            return omit;
        }

        let omit = self
            .config
            .options
            .get("MethodCallWithArgsParenthesesEnforcedStyle")
            .and_then(|value| value.as_str())
            .map(|style| style == "omit_parentheses")
            .unwrap_or_else(|| self.discover_method_call_with_args_parentheses_omit());

        self.method_call_with_args_parentheses_omit = Some(omit);
        omit
    }

    fn discover_method_call_with_args_parentheses_omit(&self) -> bool {
        let path = Path::new(&self.source.path);
        if !path.exists() {
            return false;
        }

        crate::config::load_config(None, Some(path), None)
            .ok()
            .map(|resolved| {
                resolved
                    .cop_config_for_file("Style/MethodCallWithArgsParentheses", path)
                    .get_str("EnforcedStyle", "require_parentheses")
                    == "omit_parentheses"
            })
            .unwrap_or(false)
    }

    fn add_offense(&mut self, start_offset: usize, message: &str) {
        let (line, column) = self.source.offset_to_line_col(start_offset);
        self.diagnostics.push(
            self.cop
                .diagnostic(self.source, line, column, message.to_string()),
        );
    }
}

impl<'pr> Visit<'pr> for RegexpLiteralVisitor<'_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.visit_candidate(&node);
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.visit_candidate(&node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config, run_cop_full,
    };

    crate::cop_fixture_tests!(RegexpLiteral, "cops/style/regexp_literal");

    #[test]
    fn percent_r_with_effective_leading_space_is_allowed_as_match_receiver() {
        let source = b"if %r/#{foo} bar/ =~ baz\n  nil\nend\n";
        let diags = run_cop_full(&RegexpLiteral, source);
        assert!(diags.is_empty());
    }

    fn mixed_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("mixed".into()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn mixed_style_flags_single_line_slash_regex_with_inner_slash() {
        assert_cop_offenses_full_with_config(
            &RegexpLiteral,
            include_bytes!("../../../tests/fixtures/cops/style/regexp_literal/mixed_offense.rb"),
            mixed_config(),
        );
    }

    #[test]
    fn mixed_style_allows_percent_r_call_receiver_when_omit_parentheses_is_configured() {
        let mut config = mixed_config();
        config.options.insert(
            "MethodCallWithArgsParenthesesEnforcedStyle".to_string(),
            serde_yml::Value::String("omit_parentheses".into()),
        );

        assert_cop_no_offenses_full_with_config(
            &RegexpLiteral,
            include_bytes!("../../../tests/fixtures/cops/style/regexp_literal/mixed_no_offense.rb"),
            config,
        );
    }

    #[test]
    fn mixed_style_allows_multiline_percent_r() {
        let source = b"regex = %r{\n  foo\n  bar\n}x\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&RegexpLiteral, source, mixed_config());
        assert!(
            diags.is_empty(),
            "Multiline %r is always correct in mixed style"
        );
    }

    #[test]
    fn mixed_style_still_flags_percent_r_call_receiver_without_omit_parentheses() {
        let source = b"assert_equal(':', %r:\\::.source, bug5484)\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&RegexpLiteral, source, mixed_config());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Use `//` around regular expression.");
    }
}
