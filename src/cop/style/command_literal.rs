use crate::cop::shared::node_type::{INTERPOLATED_X_STRING_NODE, X_STRING_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Corpus investigations found two RuboCop-matching edge cases for command
/// literals:
/// - inner backtick bytes anywhere in the literal body force `%x` unless
///   `AllowInnerBackticks` is enabled, including nested command literals inside
///   interpolation;
/// - under `EnforcedStyle: mixed`, any command literal whose source contains a
///   newline byte is multiline, even when it spans exactly two lines like
///   `%x(ls\n)` or ``ls\n``.
///
/// The previous implementation already missed the inner-backtick case, and it
/// also required more than one newline byte before treating a node as
/// multiline. Match RuboCop by scanning the full literal body for backticks and
/// by treating any newline in the node source as multiline.
pub struct CommandLiteral;

impl Cop for CommandLiteral {
    fn name(&self) -> &'static str {
        "Style/CommandLiteral"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[INTERPOLATED_X_STRING_NODE, X_STRING_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "backticks");
        let allow_inner_backticks = config.get_bool("AllowInnerBackticks", false);

        // Check both XStringNode and InterpolatedXStringNode
        let (opening, closing, node_loc, node_source) = if let Some(x) = node.as_x_string_node() {
            (
                x.opening_loc(),
                x.closing_loc(),
                x.location(),
                x.location().as_slice().to_vec(),
            )
        } else if let Some(x) = node.as_interpolated_x_string_node() {
            (
                x.opening_loc(),
                x.closing_loc(),
                x.location(),
                x.location().as_slice().to_vec(),
            )
        } else {
            return;
        };

        let opening_bytes = opening.as_slice();
        let body = source
            .as_bytes()
            .get(opening.end_offset()..closing.start_offset())
            .unwrap_or(&[]);
        let is_backtick = opening_bytes == b"`";
        let is_multiline = node_source.contains(&b'\n');
        let content_has_backticks = body.contains(&b'`');

        let disallowed_backtick = !allow_inner_backticks && content_has_backticks;

        match enforced_style {
            "backticks" => {
                if is_backtick && disallowed_backtick {
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `%x` around command string.".to_string(),
                    ));
                } else if !is_backtick && !disallowed_backtick {
                    // Flag %x usage unless it contains backticks (and AllowInnerBackticks is false)
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use backticks around command string.".to_string(),
                    ));
                }
            }
            "percent_x" => {
                // Flag backtick usage
                if is_backtick {
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `%x` around command string.".to_string(),
                    ));
                }
            }
            "mixed" => {
                if is_backtick && (is_multiline || disallowed_backtick) {
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `%x` around command string.".to_string(),
                    ));
                } else if !is_backtick && !is_multiline && !disallowed_backtick {
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use backticks around command string.".to_string(),
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
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
    };
    use std::collections::HashMap;

    crate::cop_fixture_tests!(CommandLiteral, "cops/style/command_literal");

    fn mixed_config() -> CopConfig {
        let mut options = HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("mixed".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn mixed_style_two_line_backtick_literal_is_an_offense() {
        let fixture =
            b"foo = `ls\n      ^ Style/CommandLiteral: Use `%x` around command string.\n`\n";
        assert_cop_offenses_full_with_config(&CommandLiteral, fixture, mixed_config());
    }

    #[test]
    fn mixed_style_two_line_percent_x_literal_is_allowed() {
        let fixture = b"foo = %x(ls\n)\n";
        assert_cop_no_offenses_full_with_config(&CommandLiteral, fixture, mixed_config());
    }

    #[test]
    fn mixed_style_backslash_continued_backtick_literal_is_an_offense() {
        let fixture = b"foo = `grep -sqxF #{needle} \"#{path}\" \\\n      ^ Style/CommandLiteral: Use `%x` around command string.\n  || echo #{needle} >> \"#{path}\"`\n";
        assert_cop_offenses_full_with_config(&CommandLiteral, fixture, mixed_config());
    }

    #[test]
    fn mixed_style_backslash_continued_percent_x_literal_is_allowed() {
        let fixture = b"foo = %x(ruby #{script} \\\n  | #{command} > out.svg)\n";
        assert_cop_no_offenses_full_with_config(&CommandLiteral, fixture, mixed_config());
    }
}
