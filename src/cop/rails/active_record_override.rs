use crate::cop::shared::constant_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

pub struct ActiveRecordOverride;

const BAD_METHODS: &[&[u8]] = &[b"create", b"destroy", b"save", b"update"];

const ACTIVE_RECORD_CLASSES: &[&str] = &[
    "ApplicationRecord",
    "ActiveModel::Base",
    "ActiveRecord::Base",
];

/// Visitor to check if a node contains bare `super` (ForwardingSuperNode only)
struct HasSuperVisitor {
    found: bool,
}

impl<'pr> Visit<'pr> for HasSuperVisitor {
    fn visit_forwarding_super_node(&mut self, _node: &ruby_prism::ForwardingSuperNode<'pr>) {
        self.found = true;
    }
}

impl Cop for ActiveRecordOverride {
    fn name(&self) -> &'static str {
        "Rails/ActiveRecordOverride"
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
        let mut visitor = ActiveRecordOverrideVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            in_active_record_class: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct ActiveRecordOverrideVisitor<'a> {
    cop: &'a ActiveRecordOverride,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    in_active_record_class: bool,
}

impl<'pr> Visit<'pr> for ActiveRecordOverrideVisitor<'_> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let was_in_ar = self.in_active_record_class;
        if is_active_record_class(self.source, node) {
            self.in_active_record_class = true;
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.in_active_record_class = was_in_ar;
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if !self.in_active_record_class {
            return;
        }

        // Must not be a class method (def self.save)
        if node.receiver().is_some() {
            return;
        }

        let method_name = node.name().as_slice();
        if !BAD_METHODS.contains(&method_name) {
            return;
        }

        // Check for bare `super` call in body (ForwardingSuperNode only)
        let mut visitor = HasSuperVisitor { found: false };
        if let Some(body) = node.body() {
            visitor.visit(&body);
        }
        if !visitor.found {
            return;
        }

        let method_str = std::str::from_utf8(method_name).unwrap_or("?");
        let callbacks =
            format!("`before_{method_str}`, `around_{method_str}`, or `after_{method_str}`");

        let loc = node.name_loc();
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            format!("Use {callbacks} callbacks instead of overriding the Active Record method `{method_str}`."),
        ));
    }

    // Don't descend into modules — methods in modules are not AR overrides
    fn visit_module_node(&mut self, _node: &ruby_prism::ModuleNode<'pr>) {}
}

/// Check if a class inherits from ApplicationRecord, ActiveRecord::Base, or ActiveModel::Base
fn is_active_record_class(source: &SourceFile, class: &ruby_prism::ClassNode<'_>) -> bool {
    let superclass = match class.superclass() {
        Some(s) => s,
        None => return false,
    };

    let full_path = constant_predicates::full_constant_path(source, &superclass);
    let path_str = std::str::from_utf8(full_path).unwrap_or("");
    ACTIVE_RECORD_CLASSES.contains(&path_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ActiveRecordOverride, "cops/rails/active_record_override");
}
