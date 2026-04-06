use crate::cop::shared::constant_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

pub struct TableNameAssignment;

impl Cop for TableNameAssignment {
    fn name(&self) -> &'static str {
        "Rails/TableNameAssignment"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = TableNameAssignmentVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            in_class: false,
            in_base_class: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct TableNameAssignmentVisitor<'a> {
    cop: &'a TableNameAssignment,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    in_class: bool,
    in_base_class: bool,
}

impl<'pr> Visit<'pr> for TableNameAssignmentVisitor<'_> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let was_in_class = self.in_class;
        let was_in_base = self.in_base_class;
        self.in_class = true;
        // Check if the class name ends with `Base` (e.g., `Base`, `Admin::Base`)
        let class_name_node = node.constant_path();
        if constant_predicates::constant_short_name(&class_name_node) == Some(b"Base") {
            self.in_base_class = true;
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.in_class = was_in_class;
        self.in_base_class = was_in_base;
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if node.name().as_slice() == b"table_name=" {
            if let Some(receiver) = node.receiver() {
                if receiver.as_self_node().is_some() && self.in_class && !self.in_base_class {
                    // Only flag if the argument is a literal string or symbol
                    // (not an interpolated string). RuboCop's `find_set_table_name`
                    // uses `{str sym}` which excludes `dstr` (interpolated strings).
                    let is_literal_arg = if let Some(args) = node.arguments() {
                        let arg_list: Vec<_> = args.arguments().iter().collect();
                        if arg_list.len() == 1 {
                            arg_list[0].as_string_node().is_some()
                                || arg_list[0].as_symbol_node().is_some()
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if is_literal_arg {
                        let loc = node.location();
                        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            "Do not set `self.table_name`. Use conventions or rename the table."
                                .to_string(),
                        ));
                    }
                }
            }
        }

        // Continue visiting child nodes for nested structures
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }
        if let Some(args) = node.arguments() {
            self.visit_arguments_node(&args);
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(TableNameAssignment, "cops/rails/table_name_assignment");
}
