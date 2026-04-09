use crate::cop::shared::node_type::DEF_NODE;
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Corpus investigation (2026-03-14):
/// - 1 FP: parameters following a multi-line default value (e.g., `), adapter: ...`)
///   were flagged even though they don't begin their line. Fixed by adding
///   `begins_its_line` check, matching RuboCop's Alignment mixin behavior.
/// - 14 FNs: block parameters (`&blk`, `&block`) were not included in alignment
///   checking because Prism stores them separately via `params.block()` rather
///   than in the regular parameter lists. Fixed by collecting all parameter
///   offsets including block params. Also added `keyword_rest` params.
/// - `with_fixed_indentation` FN: the loop always skipped the first parameter,
///   but RuboCop checks every parameter in this style because the expected
///   column is derived from the `def` line, not from the first parameter.
///   That hid offenses when the first parameter started its own continuation
///   line (for example `def method(\n      a,\n      b)`). Fixed by only
///   skipping the first parameter in `with_first_parameter` mode.
pub struct ParameterAlignment;

impl Cop for ParameterAlignment {
    fn name(&self) -> &'static str {
        "Layout/ParameterAlignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE]
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
        let style = config.get_str("EnforcedStyle", "with_first_parameter");
        let indent_width = config.get_usize("IndentationWidth", 2);

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        let params = match def_node.parameters() {
            Some(p) => p,
            None => return,
        };

        // Collect start offsets for all parameter types, including block params.
        // We use offsets directly because BlockParameterNode is a different type
        // than Node and can't be stored in the same Vec.
        let mut param_offsets: Vec<usize> = Vec::new();
        for p in params.requireds().iter() {
            param_offsets.push(p.location().start_offset());
        }
        for p in params.optionals().iter() {
            param_offsets.push(p.location().start_offset());
        }
        if let Some(rest) = params.rest() {
            param_offsets.push(rest.location().start_offset());
        }
        for kw in params.keywords().iter() {
            param_offsets.push(kw.location().start_offset());
        }
        if let Some(kw_rest) = params.keyword_rest() {
            param_offsets.push(kw_rest.location().start_offset());
        }
        if let Some(block) = params.block() {
            param_offsets.push(block.location().start_offset());
        }

        if param_offsets.len() < 2 {
            return;
        }

        let (first_line, first_col) = source.offset_to_line_col(param_offsets[0]);

        let base_col = match style {
            "with_fixed_indentation" => {
                let def_keyword_loc = def_node.def_keyword_loc();
                let (def_line, _) = source.offset_to_line_col(def_keyword_loc.start_offset());
                let def_line_bytes = util::line_at(source, def_line).unwrap_or(b"");
                util::indentation_of(def_line_bytes) + indent_width
            }
            _ => first_col, // with_first_parameter
        };

        // Only check the FIRST parameter on each new line. Multiple parameters
        // on the same continuation line should not be checked individually.
        // Also skip parameters that don't begin their line (e.g., after a
        // closing paren of a multi-line default value: `), adapter: ...`).
        let skip_first_parameter = style != "with_fixed_indentation";
        let mut last_checked_line = if skip_first_parameter {
            Some(first_line)
        } else {
            None
        };

        for &offset in param_offsets.iter().skip(usize::from(skip_first_parameter)) {
            let (param_line, param_col) = source.offset_to_line_col(offset);
            if last_checked_line == Some(param_line) {
                continue; // Same line as a previously checked param, skip
            }
            last_checked_line = Some(param_line);
            if !util::begins_its_line(source, offset) {
                continue; // Parameter doesn't begin its line (e.g., after `)` of a default value)
            }
            if param_col != base_col {
                let msg = if style == "with_fixed_indentation" {
                    "Use one level of indentation for parameters following the first line of a multi-line method definition."
                } else {
                    "Align the parameters of a method definition if they span more than one line."
                };
                diagnostics.push(self.diagnostic(source, param_line, param_col, msg.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(ParameterAlignment, "cops/layout/parameter_alignment");

    #[test]
    fn with_fixed_indentation_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ParameterAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/parameter_alignment/with_fixed_indentation_offense.rb"
            ),
            CopConfig {
                options: std::collections::HashMap::from([(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("with_fixed_indentation".into()),
                )]),
                ..CopConfig::default()
            },
        );
    }

    #[test]
    fn with_fixed_indentation_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ParameterAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/parameter_alignment/with_fixed_indentation_no_offense.rb"
            ),
            CopConfig {
                options: std::collections::HashMap::from([(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("with_fixed_indentation".into()),
                )]),
                ..CopConfig::default()
            },
        );
    }
}
