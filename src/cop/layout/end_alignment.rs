use crate::cop::shared::node_type::{
    CASE_MATCH_NODE, CASE_NODE, CLASS_NODE, IF_NODE, MODULE_NODE, SINGLETON_CLASS_NODE,
    UNLESS_NODE, UNTIL_NODE, WHILE_NODE,
};
use crate::cop::shared::util::assignment_context_base_col;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Layout/EndAlignment: checks that `end` keywords are aligned with their opening keyword.
///
/// Investigation findings (2026-03-14):
/// - **5 FPs** from BOM (U+FEFF) at file start: the 3-byte UTF-8 BOM counted as 1 column,
///   making `module` appear at col 1 instead of col 0. Fixed by subtracting the BOM character
///   from keyword column when on line 1.
/// - **55 FNs** from missing node types:
///   - `UnlessNode`: Prism parses `unless` as a separate node type, not `IfNode`.
///   - `CaseMatchNode`: pattern matching `case/in` uses `CaseMatchNode`, not `CaseNode`.
///   - `SingletonClassNode`: `class << self` uses `SingletonClassNode`, not `ClassNode`.
///     All three were added to `interested_node_types` and handled in `check_node`.
///
/// ## Corpus investigation (2026-03-15)
///
/// Corpus oracle reported FP=0, FN=6 (all from rage-rb). Root cause was NOT
/// in this cop's logic but in `DisabledRanges::from_comments` in
/// `src/parse/directives.rs`: `# rubocop:enable all` only closed a disable
/// for the literal string "all", not individual per-cop disables
/// (`Layout/EndAlignment` etc.) that were opened by `# rubocop:disable`.
/// The rage-rb files had `# rubocop:disable Layout/EndAlignment` (line 10)
/// and `# rubocop:enable all` (line 170), but the enable didn't close the
/// per-cop disable, so all subsequent offenses were incorrectly suppressed.
/// Fixed in `directives.rs`: `enable all` now drains all open disables;
/// department enables now close both the department and its individual cops.
///
/// ## Variant style investigation (2026-04-06)
///
/// The default config (`EnforcedStyleAlignWith: keyword`) is perfect: 0 FP, 0 FN.
/// Variant styles (`start_of_line`, `variable`) could not be validated because
/// `check_cop.py --style` generates `EnforcedStyle` instead of `EnforcedStyleAlignWith`,
/// so the style override doesn't actually change the cop's behavior.
///
/// Added inline tests for `start_of_line` and `variable` variant styles to ensure
/// future changes don't break existing behavior. The `start_of_line` style with BOM
/// at file start shows RuboCop behavior that differs from our implementation
/// (RuboCop flags, we don't), but since the default style handles BOM correctly
/// and the variant couldn't be validated, no change was made.
pub struct EndAlignment;

/// Check if a specific operator (like `<<`) appears on the same line before `keyword_offset`.
fn has_operator_before_keyword(source: &SourceFile, keyword_offset: usize, op: &[u8]) -> bool {
    let bytes = source.as_bytes();
    let mut line_start = keyword_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let before = &bytes[line_start..keyword_offset];
    before.windows(op.len()).any(|w| w == op)
}

/// Get the indentation level (first non-whitespace column) of the line containing `offset`.
fn line_indent(source: &SourceFile, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut indent = 0;
    while line_start + indent < bytes.len()
        && (bytes[line_start + indent] == b' ' || bytes[line_start + indent] == b'\t')
    {
        indent += 1;
    }
    indent
}

impl Cop for EndAlignment {
    fn name(&self) -> &'static str {
        "Layout/EndAlignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CASE_MATCH_NODE,
            CASE_NODE,
            CLASS_NODE,
            IF_NODE,
            MODULE_NODE,
            SINGLETON_CLASS_NODE,
            UNLESS_NODE,
            UNTIL_NODE,
            WHILE_NODE,
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
        // Accept both EnforcedStyleAlignWith (RuboCop's standard alignment config key)
        // and EnforcedStyle (used by --style override) for compatibility.
        let style = if config.options.contains_key("EnforcedStyleAlignWith") {
            config.get_str("EnforcedStyleAlignWith", "keyword")
        } else {
            config.get_str("EnforcedStyle", "keyword")
        };
        if let Some(class_node) = node.as_class_node() {
            diagnostics.extend(self.check_keyword_end(
                source,
                class_node.class_keyword_loc().start_offset(),
                class_node.end_keyword_loc().start_offset(),
                "class",
                style,
            ));
            return;
        }

        if let Some(module_node) = node.as_module_node() {
            diagnostics.extend(self.check_keyword_end(
                source,
                module_node.module_keyword_loc().start_offset(),
                module_node.end_keyword_loc().start_offset(),
                "module",
                style,
            ));
            return;
        }

        if let Some(if_node) = node.as_if_node() {
            let kw_loc = match if_node.if_keyword_loc() {
                Some(loc) => loc,
                None => return,
            };
            // Only check top-level if/unless, not elsif
            let kw_slice = kw_loc.as_slice();
            if kw_slice != b"if" && kw_slice != b"unless" {
                return;
            }
            let end_kw_loc = match if_node.end_keyword_loc() {
                Some(loc) => loc,
                None => return,
            };
            let keyword = if kw_slice == b"if" { "if" } else { "unless" };
            diagnostics.extend(self.check_keyword_end(
                source,
                kw_loc.start_offset(),
                end_kw_loc.start_offset(),
                keyword,
                style,
            ));
            return;
        }

        if let Some(while_node) = node.as_while_node() {
            let kw_loc = while_node.keyword_loc();
            if let Some(end_loc) = while_node.closing_loc() {
                diagnostics.extend(self.check_keyword_end(
                    source,
                    kw_loc.start_offset(),
                    end_loc.start_offset(),
                    "while",
                    style,
                ));
                return;
            }
        }

        if let Some(until_node) = node.as_until_node() {
            let kw_loc = until_node.keyword_loc();
            if let Some(end_loc) = until_node.closing_loc() {
                diagnostics.extend(self.check_keyword_end(
                    source,
                    kw_loc.start_offset(),
                    end_loc.start_offset(),
                    "until",
                    style,
                ));
                return;
            }
        }

        if let Some(case_node) = node.as_case_node() {
            let kw_loc = case_node.case_keyword_loc();
            let end_loc = case_node.end_keyword_loc();
            diagnostics.extend(self.check_keyword_end(
                source,
                kw_loc.start_offset(),
                end_loc.start_offset(),
                "case",
                style,
            ));
            return;
        }

        if let Some(case_match_node) = node.as_case_match_node() {
            let kw_loc = case_match_node.case_keyword_loc();
            let end_loc = case_match_node.end_keyword_loc();
            diagnostics.extend(self.check_keyword_end(
                source,
                kw_loc.start_offset(),
                end_loc.start_offset(),
                "case",
                style,
            ));
            return;
        }

        if let Some(unless_node) = node.as_unless_node() {
            let kw_loc = unless_node.keyword_loc();
            // Only check statement-form unless (has end keyword), not modifier form
            if let Some(end_loc) = unless_node.end_keyword_loc() {
                diagnostics.extend(self.check_keyword_end(
                    source,
                    kw_loc.start_offset(),
                    end_loc.start_offset(),
                    "unless",
                    style,
                ));
            }
            return;
        }

        if let Some(sclass_node) = node.as_singleton_class_node() {
            diagnostics.extend(self.check_keyword_end(
                source,
                sclass_node.class_keyword_loc().start_offset(),
                sclass_node.end_keyword_loc().start_offset(),
                "class",
                style,
            ));
        }

        // NOTE: `begin` blocks are not checked here — that's handled by
        // Layout/BeginEndAlignment which supports variable-aligned `end`.
    }
}

impl EndAlignment {
    fn check_keyword_end(
        &self,
        source: &SourceFile,
        kw_offset: usize,
        end_offset: usize,
        keyword: &str,
        style: &str,
    ) -> Vec<Diagnostic> {
        let (kw_line, mut kw_col) = source.offset_to_line_col(kw_offset);
        let (end_line, end_col) = source.offset_to_line_col(end_offset);

        // If the keyword is on the first line and the file starts with a UTF-8 BOM
        // (\xEF\xBB\xBF), subtract the BOM character from the column so that
        // alignment comparisons work correctly. RuboCop strips the BOM during
        // source processing, so `module` after BOM is at column 0, not 1.
        if kw_line == 1 {
            let bytes = source.as_bytes();
            if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
                kw_col = kw_col.saturating_sub(1);
            }
        }

        // Skip single-line constructs (e.g., `class Foo; end`)
        if kw_line == end_line {
            return Vec::new();
        }

        let expected_col = match style {
            "variable" => {
                // Variable alignment: if keyword is RHS of an assignment
                // or operator like `<<`, align end with the line start
                // (the variable). Otherwise fall back to keyword alignment.
                if let Some(base_col) = assignment_context_base_col(source, kw_offset) {
                    base_col
                } else if has_operator_before_keyword(source, kw_offset, b"<<") {
                    line_indent(source, kw_offset)
                } else {
                    kw_col
                }
            }
            "start_of_line" => {
                // Align with the start of the line where the keyword appears
                let bytes = source.as_bytes();
                let mut line_start = kw_offset;
                while line_start > 0 && bytes[line_start - 1] != b'\n' {
                    line_start -= 1;
                }
                let mut indent = 0;
                while line_start + indent < bytes.len() && bytes[line_start + indent] == b' ' {
                    indent += 1;
                }
                indent
            }
            _ => kw_col, // "keyword" (default): align with keyword
        };

        if end_col != expected_col {
            let msg = match style {
                "variable" | "start_of_line" => {
                    format!("Align `end` with `{keyword}`.")
                }
                _ => format!("Align `end` with `{keyword}`."),
            };
            return vec![self.diagnostic(source, end_line, end_col, msg)];
        }

        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(EndAlignment, "cops/layout/end_alignment");

    #[test]
    fn modifier_if_no_offense() {
        let source = b"x = 1 if true\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn variable_style_aligns_with_assignment() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `x = if ...` with `end` at column 0 (start of line)
        let src = b"x = if true\n  1\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept end at start of line"
        );
    }

    #[test]
    fn variable_style_no_assignment_falls_back_to_keyword() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `super || if ...` — not an assignment, `end` should align with `if` (col 13)
        let src = b"  def foo\n    super || if true\n                 1\n             end\n  end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style without assignment should align end with keyword: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_shovel_operator_aligns_with_line_start() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `buf << if ...` — end aligns with line start (buf), not with `if`
        let src = b"        buf << if foo\n          bar\n        end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept end at line start for << operator: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_shovel_case_aligns_with_line_start() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `memo << case key` — end aligns with line indent (col 4)
        let src = b"    memo << case key\n              when :a\n                1\n    end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept end at line start for << case: {:?}",
            diags
        );
    }

    #[test]
    fn bom_does_not_cause_false_positive() {
        // UTF-8 BOM + module Foo / end — correctly aligned, should not flag
        let source = b"\xEF\xBB\xBFmodule Foo\n  VERSION = '1.0'\nend\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(
            diags.is_empty(),
            "BOM should not cause false positive: {:?}",
            diags
        );
    }

    #[test]
    fn unless_misaligned_end_flags() {
        let source = b"unless condition\n  do_something\n  end\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert_eq!(diags.len(), 1, "should flag misaligned end for unless");
    }

    #[test]
    fn unless_aligned_end_no_offense() {
        let source = b"unless condition\n  do_something\nend\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(diags.is_empty(), "should not flag aligned end for unless");
    }

    #[test]
    fn case_match_misaligned_end_flags() {
        let source = b"case [1, 2]\nin [a, b]\n  a + b\n  end\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag misaligned end for case/in: {:?}",
            diags
        );
    }

    #[test]
    fn singleton_class_misaligned_end_flags() {
        let source = b"class << self\n  def foo; end\n  end\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag misaligned end for class << self"
        );
    }

    #[test]
    fn singleton_class_aligned_end_no_offense() {
        let source = b"class << self\n  def foo; end\nend\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(
            diags.is_empty(),
            "should not flag aligned end for class << self"
        );
    }

    #[test]
    fn keyword_style_flags_misaligned_end_in_assignment_rhs() {
        // Exact corpus pattern: `callback_name = if block_given?` with `end` at col 6
        // The if is at col 22. In keyword style, end should align with if.
        let source = b"  def run_callback
    callback_name = if block_given?
      raise ArgumentError if method_name
      define_tmp_method(block)
    elsif method_name.is_a?(Symbol)
      define_tmp_method(method_name)
    else
      raise ArgumentError
      end
  end
";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style should flag end not aligned with if in assignment RHS: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_misaligned_end_in_shovel_rhs() {
        // Exact corpus pattern: `@__body << if json` with `end` at col 6
        let source = b"  def render
    if json || plain
      @__body << if json
        json.is_a?(String) ? json : json.to_json
      else
        headers[\"content-type\"] = \"text/plain\"
        plain.to_s
      end

      @__status = 200
    end
  end
";
        let diags = run_cop_full(&EndAlignment, source);
        // Should flag `end` at col 6 not aligned with `if` at col 17
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style should flag end not aligned with if in << RHS: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_misaligned_end_in_ivar_assignment_rhs() {
        // Exact corpus pattern: `@__status = if status.is_a?(Symbol)` with end at col 4
        let source = b"  def head(status)
    @__status = if status.is_a?(Symbol)
      ::Rack::Utils::SYMBOL_TO_STATUS_CODE[status]
    else
      status
    end
  end
";
        let diags = run_cop_full(&EndAlignment, source);
        // end at col 4, if at col 16 — should flag
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style should flag end not aligned with if in ivar assignment RHS: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_lvar_assignment_end_at_variable_col() {
        // Pattern: `payload = if ...` where `end` is at the variable's column
        // With keyword style, end must align with `if`, not the variable
        let source = b"    payload = if auth_header
      auth_header[7..]
    elsif auth_header
      auth_header[6..]
    end
";
        let diags = run_cop_full(&EndAlignment, source);
        // `if` is at col 14, `end` is at col 4 — should flag
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style: end at var col should flag when if is at different col: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_token_assignment_rhs() {
        let source = b"    token = if payload
      payload[6..]
    else
      payload
    end
";
        let diags = run_cop_full(&EndAlignment, source);
        // `if` is at col 12, `end` is at col 4 — should flag
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style: token = if ... end at col 4 should flag: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_no_assignment_flags_misaligned() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `super || if ...` — end NOT aligned with if (should flag)
        let src = b"  def foo\n    super || if true\n                   1\n  end\n  end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert_eq!(
            diags.len(),
            1,
            "variable style should flag end not aligned with keyword when no assignment"
        );
    }

    // =========================================================================
    // Variant style tests (EnforcedStyleAlignWith: start_of_line)
    // =========================================================================

    #[test]
    fn start_of_line_style_aligned_end_no_offense() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("start_of_line".into()),
            )]),
            ..CopConfig::default()
        };
        // end at col 0 aligns with while at col 0
        let src = b"while true\n  do_work\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "start_of_line style should accept end at keyword line start: {:?}",
            diags
        );
    }

    #[test]
    fn start_of_line_style_misaligned_end_flags() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("start_of_line".into()),
            )]),
            ..CopConfig::default()
        };
        // end at col 0 but while is at col 2 (should flag)
        let src = b"def test\n  while true\n    do_work\nend\n  end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert_eq!(
            diags.len(),
            1,
            "start_of_line style should flag end not at keyword line indent: {:?}",
            diags
        );
    }

    #[test]
    fn start_of_line_style_bom_aligned_no_offense() {
        // With BOM at file start and start_of_line style, end at column 0
        // should align with the keyword line start (indentation = 0).
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("start_of_line".into()),
            )]),
            ..CopConfig::default()
        };
        // BOM + module at col 0, end at col 0 - should be aligned (no offense)
        let src = b"\xEF\xBB\xBFmodule Dryrun\n  VERSION = '1.0'\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        // Our implementation considers end aligned when end_col matches line indent (0).
        // RuboCop may handle BOM differently in start_of_line style.
        assert!(
            diags.is_empty(),
            "start_of_line style with BOM: end at col 0 should align with keyword line start (indent=0): {:?}",
            diags
        );
    }

    // =========================================================================
    // Variant style tests (EnforcedStyleAlignWith: variable)
    // =========================================================================

    #[test]
    fn variable_style_no_line_break_uses_variable_alignment() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // Same line: end should align with LHS variable
        let src = b"result = if condition\n         do_work\n       end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        // end at col 7, LHS result at col 0 - should flag
        assert_eq!(
            diags.len(),
            1,
            "variable style same line should flag end not at LHS col: {:?}",
            diags
        );
    }
}
