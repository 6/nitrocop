use crate::cop::shared::util::is_dsl_call;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// ## Corpus investigation (2026-03-07)
///
/// FP=1, FN=0 per corpus oracle.
///
/// ## Corpus investigation (2026-03-10)
///
/// FP=1: `write_attribute("conditions", exp)` inside `def conditions=(exp)`.
/// The shadowing method check only handled symbol args (`:attr`), not string
/// args (`"attr"`). RuboCop's `within_shadowing_method?` calls
/// `first_arg.respond_to?(:value)` which works for both sym and str nodes.
/// Fixed by also extracting attribute names from `StringNode` in the
/// shadowing check.
pub struct ReadWriteAttribute;

impl Cop for ReadWriteAttribute {
    fn name(&self) -> &'static str {
        "Rails/ReadWriteAttribute"
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
        let mut visitor = RWVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            enclosing_method: None,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct RWVisitor<'a, 'src> {
    cop: &'a ReadWriteAttribute,
    source: &'src SourceFile,
    diagnostics: Vec<Diagnostic>,
    /// Name of the enclosing def method, if any.
    enclosing_method: Option<Vec<u8>>,
}

impl<'pr> RWVisitor<'_, '_> {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'pr>) {
        let is_read = is_dsl_call(call, b"read_attribute");
        let is_write = !is_read && is_dsl_call(call, b"write_attribute");
        if !is_read && !is_write {
            return;
        }

        // Check for shadowing method: read_attribute(:foo) inside def foo, or
        // write_attribute(:foo, val) inside def foo=
        if let Some(ref method_name) = self.enclosing_method {
            if let Some(args) = call.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                if !arg_list.is_empty() {
                    // Extract attribute name from symbol (:attr) or string ("attr")
                    let attr_name: Option<Vec<u8>> = arg_list[0]
                        .as_symbol_node()
                        .map(|sym| sym.unescaped().to_vec())
                        .or_else(|| arg_list[0].as_string_node().map(|s| s.unescaped().to_vec()));
                    if let Some(attr_name) = attr_name {
                        let mut expected_method = attr_name;
                        if is_write {
                            expected_method.push(b'=');
                        }
                        if method_name == &expected_method {
                            return;
                        }
                    }
                }
            }
        }

        let loc = call.message_loc().unwrap_or(call.location());
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        let msg = if is_read {
            "Use `self[:attr]` instead of `read_attribute`.".to_string()
        } else {
            "Use `self[:attr] = val` instead of `write_attribute`.".to_string()
        };
        self.diagnostics
            .push(self.cop.diagnostic(self.source, line, column, msg));
    }
}

impl<'pr> Visit<'pr> for RWVisitor<'_, '_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let old = self.enclosing_method.take();
        self.enclosing_method = Some(node.name().as_slice().to_vec());
        ruby_prism::visit_def_node(self, node);
        self.enclosing_method = old;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ReadWriteAttribute, "cops/rails/read_write_attribute");

    #[test]
    fn skips_shadowing_method() {
        use crate::testutil::run_cop_full;

        // read_attribute(:slug) inside def slug should not be flagged
        let source = b"class Foo < ApplicationRecord
  def slug
    read_attribute(:slug)
  end
end
";
        let diags = run_cop_full(&ReadWriteAttribute, source);
        assert!(
            diags.is_empty(),
            "should not flag read_attribute inside shadowing method: {:?}",
            diags
        );
    }

    #[test]
    fn skips_write_in_setter() {
        use crate::testutil::run_cop_full;

        // write_attribute(:title, t) inside def title= should not be flagged
        let source = b"class Foo < ApplicationRecord
  def title=(t)
    write_attribute(:title, t)
  end
end
";
        let diags = run_cop_full(&ReadWriteAttribute, source);
        assert!(
            diags.is_empty(),
            "should not flag write_attribute inside setter: {:?}",
            diags
        );
    }
}
