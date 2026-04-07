use crate::cop::shared::node_type::{BLOCK_NODE, CALL_NODE, FOR_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/For cop that flags `for` loops vs `.each` blocks based on `EnforcedStyle`.
///
/// ## Variant divergence fix (2026-04-07)
///
/// The cop previously only handled `EnforcedStyle: each` (the default). When
/// `EnforcedStyle: for` was configured, the cop returned early and never detected
/// any offenses, causing 65,207 false negatives against RuboCop's `for` style.
///
/// **Root cause:** The cop only visited `FOR_NODE` and returned early when the
/// style was not `each`. It never visited `CALL_NODE` to detect `.each` blocks.
///
/// **Fix applied:** Added `CALL_NODE` to `interested_node_types`. When style is
/// `for`, the cop now visits `CallNode` and checks if:
///   1. The method being called is `:each`
///   2. The call has a receiver (`.each` not `each`)
///   3. The call has no arguments
///   4. The associated block is multiline (single-line blocks are always allowed)
///
/// This matches RuboCop's `suspect_enumerable?` check in `on_block`.
///
/// Verified against RuboCop: both tools now flag the same offenses in test repos
/// (e.g., 8 offenses each in 8bitpal__hackful, 5 offenses each in Bodacious__blogit).
pub struct ForCop;

impl Cop for ForCop {
    fn name(&self) -> &'static str {
        "Style/For"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, FOR_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "each");

        if enforced_style == "each" {
            // Flag for loops when EnforcedStyle is each
            let for_node = match node.as_for_node() {
                Some(n) => n,
                None => return,
            };
            let loc = for_node.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `each` over `for`.".to_string(),
            ));
        } else if enforced_style == "for" {
            // Flag multiline .each blocks when EnforcedStyle is for
            let call_node = match node.as_call_node() {
                Some(n) => n,
                None => return,
            };

            // Check if method is :each
            if call_node.name().as_slice() != b"each" {
                return;
            }

            // Check if has receiver (without receiver, it's just 'each' not '.each')
            if call_node.receiver().is_none() {
                return;
            }

            // Check if no arguments (arguments would mean something like .each(&block))
            if call_node.arguments().is_some() {
                return;
            }

            // Check if has a block
            let block_node = match call_node.block() {
                Some(b) => b.as_block_node(),
                None => return,
            };
            let block_node = match block_node {
                Some(b) => b,
                None => return,
            };

            // Check if block is multiline
            let opening_loc = block_node.opening_loc();
            let closing_loc = block_node.closing_loc();
            let (opening_line, _) = source.offset_to_line_col(opening_loc.start_offset());
            let (closing_line, _) = source.offset_to_line_col(closing_loc.start_offset());

            if opening_line == closing_line {
                // Single-line block is always allowed
                return;
            }

            let loc = call_node.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `for` over `each`.".to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn for_style_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("for".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    crate::cop_fixture_tests!(ForCop, "cops/style/for_cop");

    #[test]
    fn for_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ForCop,
            include_bytes!("../../../tests/fixtures/cops/style/for_cop/for_style_offense.rb"),
            for_style_config(),
        );
    }

    #[test]
    fn for_style_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ForCop,
            include_bytes!("../../../tests/fixtures/cops/style/for_cop/for_style_no_offense.rb"),
            for_style_config(),
        );
    }
}
