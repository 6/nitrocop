use crate::cop::shared::constant_predicates;
use crate::cop::shared::node_type::{CALL_NODE, MULTI_WRITE_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

const MSG: &str = "Use `Time.use_zone` with block instead of `Time.zone=`.";

/// Matches RuboCop's `Time.zone=` send check for both direct assignments and
/// Prism's parallel-assignment form, where `Time.zone` is a `CallTargetNode`
/// inside `MultiWriteNode` (for example `a, Time.zone = ...`).
pub struct TimeZoneAssignment;

fn time_receiver_is_simple_time(receiver: &ruby_prism::Node<'_>) -> bool {
    constant_predicates::is_simple_constant(receiver, b"Time")
}

fn time_zone_assignment(call: &ruby_prism::CallNode<'_>) -> bool {
    call.name().as_slice() == b"zone="
        && call
            .receiver()
            .is_some_and(|receiver| time_receiver_is_simple_time(&receiver))
}

fn time_zone_assignment_target(target: &ruby_prism::CallTargetNode<'_>) -> bool {
    target.name().as_slice() == b"zone=" && time_receiver_is_simple_time(&target.receiver())
}

fn push_offense(
    cop: &TimeZoneAssignment,
    source: &SourceFile,
    loc: ruby_prism::Location<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (line, column) = source.offset_to_line_col(loc.start_offset());
    diagnostics.push(cop.diagnostic(source, line, column, MSG.to_string()));
}

impl Cop for TimeZoneAssignment {
    fn name(&self) -> &'static str {
        "Rails/TimeZoneAssignment"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["spec/**/*.rb", "test/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, MULTI_WRITE_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        if let Some(call) = node.as_call_node() {
            if time_zone_assignment(&call) {
                push_offense(self, source, node.location(), diagnostics);
            }
            return;
        }

        let Some(multi_write) = node.as_multi_write_node() else {
            return;
        };

        for target in multi_write.lefts().iter() {
            if let Some(call_target) = target.as_call_target_node() {
                if time_zone_assignment_target(&call_target) {
                    push_offense(self, source, call_target.location(), diagnostics);
                }
            }
        }

        if let Some(rest_target) = multi_write
            .rest()
            .and_then(|target| target.as_call_target_node())
        {
            if time_zone_assignment_target(&rest_target) {
                push_offense(self, source, rest_target.location(), diagnostics);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(TimeZoneAssignment, "cops/rails/time_zone_assignment");
}
