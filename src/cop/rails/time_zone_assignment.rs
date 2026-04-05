use crate::cop::shared::constant_predicates;
use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct TimeZoneAssignment;

impl Cop for TimeZoneAssignment {
    fn name(&self) -> &'static str {
        "Rails/TimeZoneAssignment"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.name().as_slice() != b"zone=" {
            return;
        }

        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };
        // Handle both ConstantReadNode (Time) and ConstantPathNode (::Time)
        if constant_predicates::constant_short_name(&recv) != Some(b"Time") {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Do not set `Time.zone` directly. Use `Time.use_zone` instead.".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(TimeZoneAssignment, "cops/rails/time_zone_assignment");
}
