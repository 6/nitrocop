use crate::cop::shared::constant_predicates;
use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct Exit;

const EXIT_METHODS: &[&[u8]] = &[b"exit", b"exit!", b"abort"];
const EXPLICIT_RECEIVERS: &[&[u8]] = &[b"Kernel", b"Process"];

impl Cop for Exit {
    fn name(&self) -> &'static str {
        "Rails/Exit"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/lib/**/*.rake"]
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

        let name = call.name().as_slice();
        if !EXIT_METHODS.contains(&name) {
            return;
        }

        // Check argument count (must be 0 or 1)
        if let Some(args) = call.arguments() {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if arg_list.len() > 1 {
                return;
            }
        }

        // Check receiver: must be nil, Kernel, or Process
        if let Some(receiver) = call.receiver() {
            let is_allowed_receiver =
                if let Some(name) = constant_predicates::constant_short_name(&receiver) {
                    EXPLICIT_RECEIVERS.contains(&name)
                } else {
                    false
                };
            if !is_allowed_receiver {
                return;
            }
        }

        let name_str = std::str::from_utf8(name).unwrap_or("exit");

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Do not use `{name_str}` in Rails applications."),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(Exit, "cops/rails/exit");
}
