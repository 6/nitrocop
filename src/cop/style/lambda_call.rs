use crate::cop::shared::node_type::{CALL_AND_WRITE_NODE, CALL_NODE, CALL_OR_WRITE_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for use of the lambda.(args) syntax.
///
/// The cop handles two styles:
/// - `call` (default): prefers `lambda.call(...)` over `lambda.()`
/// - `braces`: prefers `lambda.()` over `lambda.call()`
///
/// **Variant FN fix:** The `braces` style had 2 false negatives where compound
/// assignment forms like `call_availability.call ||= build(...)` were not detected.
/// These are parsed as `CallOrWriteNode` / `CallAndWriteNode` in Prism, not `CallNode`.
/// The cop now also registers interest in these node types to catch explicit `.call`
/// calls within compound assignments (e.g., `x.call ||=`, `x.call &&=`).
pub struct LambdaCall;

impl Cop for LambdaCall {
    fn name(&self) -> &'static str {
        "Style/LambdaCall"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, CALL_OR_WRITE_NODE, CALL_AND_WRITE_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "call");

        // Handle CallOrWriteNode and CallAndWriteNode (e.g., x.call ||= ...)
        // These represent explicit .call method calls with compound assignment.
        if enforced_style == "braces" {
            if let Some(cow) = node.as_call_or_write_node() {
                // For braces style: explicit .call with ||= should be flagged
                // e.g., call_availability.call ||= build(...)
                if cow.read_name().as_slice() == b"call" {
                    let loc = node.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Prefer the use of `lambda.(...)` over `lambda.call(...)`.".to_string(),
                    ));
                }
                return;
            }
            if let Some(caw) = node.as_call_and_write_node() {
                if caw.read_name().as_slice() == b"call" {
                    let loc = node.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Prefer the use of `lambda.(...)` over `lambda.call(...)`.".to_string(),
                    ));
                }
                return;
            }
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must have a receiver
        if call.receiver().is_none() {
            return;
        }

        match enforced_style {
            "call" => {
                // Detect lambda.() (implicit call — method name is "call" but no message_loc or
                // message_loc source is empty). In Prism, lambda.() is represented as CallNode
                // with name "call" but the method_name position is at the dot.
                let name = call.name();
                if name.as_slice() != b"call" {
                    return;
                }

                // Check if this is an implicit call (lambda.() syntax)
                // In implicit call, there's no explicit "call" selector
                let msg_loc = match call.message_loc() {
                    Some(loc) => loc,
                    None => {
                        // No message_loc means implicit call like foo.()
                        let loc = node.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Prefer the use of `lambda.call(...)` over `lambda.(...)`.".to_string(),
                        ));
                        return;
                    }
                };

                // If the message_loc source IS "call", this is already explicit style
                if msg_loc.as_slice() == b"call" {
                    return;
                }

                // Otherwise it's implicit
                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer the use of `lambda.call(...)` over `lambda.(...)`.".to_string(),
                ));
            }
            "braces" => {
                // Detect lambda.call() (explicit call)
                let name = call.name();
                if name.as_slice() != b"call" {
                    return;
                }

                // Check if this is an explicit call
                let msg_loc = match call.message_loc() {
                    Some(loc) => loc,
                    None => return, // Already implicit
                };

                if msg_loc.as_slice() != b"call" {
                    return;
                }

                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer the use of `lambda.(...)` over `lambda.call(...)`.".to_string(),
                ));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(LambdaCall, "cops/style/lambda_call");
}
