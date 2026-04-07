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
///
/// Variant investigation (2026-04-07):
/// - FN: with_fixed_indentation style missed cases where ALL parameters are on
///   the same continuation line. The `skip(1)` logic combined with line-skip
///   caused ALL parameters to be skipped (since they're on the same line as
///   the first param, and skip(1) skips the first). For with_fixed_indentation,
///   the first parameter on a continuation line must also be at base_col
///   (def_indent + 2), not just subsequent params on new lines. Fixed by
///   checking the first parameter separately before the loop when using
///   with_fixed_indentation and it's on a different line from the def keyword.
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
        let _indent_width = config.get_usize("IndentationWidth", 2);

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

        let (def_line, _) = {
            let def_keyword_loc = def_node.def_keyword_loc();
            source.offset_to_line_col(def_keyword_loc.start_offset())
        };

        let base_col = match style {
            "with_fixed_indentation" => {
                let def_line_bytes = util::line_at(source, def_line).unwrap_or(b"");
                util::indentation_of(def_line_bytes) + 2
            }
            _ => first_col, // with_first_parameter
        };

        // For with_fixed_indentation, also check the first parameter if it's on
        // a different line from the def keyword. The first param on a
        // continuation line should also be at base_col.
        if style == "with_fixed_indentation" && first_line != def_line {
            if first_col != base_col {
                let msg = "Use one level of indentation for parameters following the first line of a multi-line method definition.";
                diagnostics.push(self.diagnostic(source, first_line, first_col, msg.to_string()));
            }
        }

        // Only check the FIRST parameter on each new line. Multiple parameters
        // on the same continuation line should not be checked individually.
        // Also skip parameters that don't begin their line (e.g., after a
        // closing paren of a multi-line default value: `), adapter: ...`).
        let mut last_checked_line = first_line;
        for &offset in param_offsets.iter().skip(1) {
            let (param_line, param_col) = source.offset_to_line_col(offset);
            if param_line == last_checked_line {
                continue; // Same line as a previously checked param, skip
            }
            last_checked_line = param_line;
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
    use serde_yml::Value;

    crate::cop_fixture_tests!(ParameterAlignment, "cops/layout/parameter_alignment");

    fn with_fixed_indentation_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "EnforcedStyle".to_string(),
            Value::String("with_fixed_indentation".to_string()),
        );
        config
    }

    #[test]
    fn with_fixed_indentation_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ParameterAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/parameter_alignment/with_fixed_indentation_offense.rb"
            ),
            with_fixed_indentation_config(),
        );
    }
}
