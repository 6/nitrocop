use ruby_prism::Visit;

use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/StringLiteralsInInterpolation checks for string literals inside
/// `#{}` interpolation that use the wrong quote style.
///
/// ## Investigation findings (2026-03-15)
///
/// **FPs (65→0):** Original FPs came from string literals inside `%x()` or
/// backtick interpolation. RuboCop only checks `:dstr`, `:dsym`, `:regexp`
/// — NOT `:xstr`. Fixed by not setting `in_interpolation` for xstr embedded
/// statements, while still recursing to find nested regular interpolated strings.
///
/// **FNs (24→0):** Fixed `needs_double_quotes` to only allow `\\` and `\"` as
/// safe-to-convert escapes. All other `\X` sequences (including unrecognized
/// escapes like `\.`, `\/`, `\#`) produce different results in single vs
/// double quotes, so they require double quotes.
///
/// ## Investigation findings (2026-03-19)
///
/// **FPs (6):** Three root causes:
/// 1. Unrecognized escapes (`\.`, `\/`, `\#`) — in double quotes `"\."` → `.`,
///    but in single quotes `'\.'` → `\.` (two chars). Must require double quotes.
/// 2. `\'` escape hides a literal single quote — `needs_double_quotes` skipped
///    it because `'` appeared after `\`, never reaching the bare `'` check.
/// 3. Both fixed by simplifying: after `\`, only `\\` and `\"` are safe to
///    convert; everything else requires double quotes.
///
/// **FNs (1):** String `"id"` inside `#{item["id"]}` nested within a backtick
/// xstr. The previous fix skipped xstr nodes entirely, missing strings inside
/// regular interpolated strings nested within xstr. Fixed by tracking `in_xstr`
/// separately: xstr embedded statements don't set `in_interpolation`, but
/// entering a nested `InterpolatedStringNode` resets `in_xstr` so its
/// embedded statements correctly set `in_interpolation`.
///
/// ## Investigation findings (2026-04-09)
///
/// RuboCop never checks multiline quoted literals inside interpolation because
/// the `parser` gem emits them as nested `:dstr` nodes, while Prism keeps them
/// as `StringNode`s with literal newlines in their source. To match RuboCop,
/// this cop skips interpolated string nodes whose source spans a real newline.
/// That fixes the remaining default-style FPs and the `double_quotes` variant's
/// multiline FP cluster without suppressing ordinary single-line literals.
///
/// For `EnforcedStyle: double_quotes`, RuboCop also keeps single-quoted
/// literals when rewriting them would change semantics by activating
/// interpolation (`'#{'`, `'#@'`, `'#$'`), forcing extra escaping for literal
/// double quotes, or changing backslash escapes that only double quotes
/// interpret. This cop mirrors those exemptions from `wrong_quotes?`.
pub struct StringLiteralsInInterpolation;

fn single_quotes_allowed_under_double_quotes(source: &[u8]) -> bool {
    source.contains(&b'"')
        || source.windows(2).any(|w| {
            (w[0] == b'\\' && w[1] != b'\'' && w[1] != b'\\')
                || matches!(w, [b'#', b'@' | b'$' | b'{'])
        })
}

impl Cop for StringLiteralsInInterpolation {
    fn name(&self) -> &'static str {
        "Style/StringLiteralsInInterpolation"
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
        let enforced_style = config.get_str("EnforcedStyle", "single_quotes").to_string();

        let mut visitor = InterpStringVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            enforced_style,
            in_interpolation: false,
            in_xstr: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct InterpStringVisitor<'a> {
    cop: &'a StringLiteralsInInterpolation,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    enforced_style: String,
    in_interpolation: bool,
    in_xstr: bool,
}

impl<'pr> Visit<'pr> for InterpStringVisitor<'_> {
    fn visit_embedded_statements_node(&mut self, node: &ruby_prism::EmbeddedStatementsNode<'pr>) {
        let was = self.in_interpolation;
        // Only set in_interpolation for dstr/dsym/regexp — NOT xstr.
        if !self.in_xstr {
            self.in_interpolation = true;
        }
        ruby_prism::visit_embedded_statements_node(self, node);
        self.in_interpolation = was;
    }

    // RuboCop only checks strings inside dstr, dsym, and regexp — NOT xstr.
    // We still recurse into xstr to find nested regular interpolated strings,
    // but xstr's own embedded statements don't set in_interpolation.
    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        let was = self.in_xstr;
        self.in_xstr = true;
        ruby_prism::visit_interpolated_x_string_node(self, node);
        self.in_xstr = was;
    }

    // When entering a regular interpolated string (even nested inside xstr),
    // reset in_xstr so its embedded statements correctly set in_interpolation.
    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        let was_xstr = self.in_xstr;
        self.in_xstr = false;
        ruby_prism::visit_interpolated_string_node(self, node);
        self.in_xstr = was_xstr;
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if !self.in_interpolation {
            return;
        }

        let loc = node.location();

        // RuboCop never visits multiline quoted literals here because parser
        // represents them as nested dstr nodes instead of str nodes.
        if loc.as_slice().contains(&b'\n') || loc.as_slice().contains(&b'\r') {
            return;
        }

        let opening = match node.opening_loc() {
            Some(o) => o,
            None => return,
        };

        let open_bytes = opening.as_slice();

        // Check if the string uses the wrong quote style
        match self.enforced_style.as_str() {
            "single_quotes" => {
                if open_bytes == b"\"" {
                    // Check if it needs double quotes (has escape sequences)
                    let content = node.content_loc().as_slice();
                    if !util::double_quotes_required(content) {
                        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            "Prefer single-quoted strings inside interpolations.".to_string(),
                        ));
                    }
                }
            }
            "double_quotes" => {
                if open_bytes == b"'" && !single_quotes_allowed_under_double_quotes(loc.as_slice())
                {
                    let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                    self.diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line,
                        column,
                        "Prefer double-quoted strings inside interpolations.".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        StringLiteralsInInterpolation,
        "cops/style/string_literals_in_interpolation"
    );

    fn double_quotes_config() -> crate::cop::CopConfig {
        use std::collections::HashMap;
        crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn double_quotes_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &StringLiteralsInInterpolation,
            include_bytes!(
                "../../../tests/fixtures/cops/style/string_literals_in_interpolation/double_quotes_offense.rb"
            ),
            double_quotes_config(),
        );
    }

    #[test]
    fn double_quotes_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &StringLiteralsInInterpolation,
            include_bytes!(
                "../../../tests/fixtures/cops/style/string_literals_in_interpolation/double_quotes_no_offense.rb"
            ),
            double_quotes_config(),
        );
    }
}
